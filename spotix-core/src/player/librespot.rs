use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use librespot_core::{
    Session, SpotifyUri, authentication::Credentials as LibrespotCredentials,
    cache::Cache as LibrespotCache, config::SessionConfig,
};
use librespot_playback::{
    audio_backend,
    config::VolumeCtrl,
    config::{Bitrate, PlayerConfig},
    mixer::{Mixer, MixerConfig, softmixer::SoftMixer},
    player::{Player as LibrespotPlayer, PlayerEvent as LibrespotEvent},
};
use tokio::runtime::Runtime;

use crate::{
    connection::Credentials,
    error::Error,
    item_id::{ItemId, ItemIdType},
};

use super::{
    PlaybackConfig, PlayerEvent,
    file::{AudioFormat, MediaPath},
    item::PlaybackItem,
};
use crate::audio::normalize::NormalizationLevel;
use crate::item_id::FileId;
use crossbeam_channel::Sender;

pub struct LibrespotBackend {
    player: Arc<LibrespotPlayer>,
    mixer: Arc<dyn Mixer>,
    /// When true, the event forwarder will suppress the next `Stopped` event.
    /// Used during track transitions to prevent the UI from treating an
    /// internal stop-before-load as a real playback stop.
    transitioning: Arc<AtomicBool>,
    _runtime: Runtime,
    _event_thread: thread::JoinHandle<()>,
}

impl LibrespotBackend {
    pub fn new(
        config: &PlaybackConfig,
        creds: &Credentials,
        sender: Sender<PlayerEvent>,
        cache_dir: Option<PathBuf>,
        audio_cache_limit: Option<u64>,
    ) -> Result<Self, Error> {
        let runtime = Runtime::new().map_err(map_error)?;

        let cache = cache_dir
            .as_ref()
            .and_then(|dir| {
                let audio_dir = Some(dir.join("librespot-audio"));
                LibrespotCache::new(
                    None::<PathBuf>,
                    None::<PathBuf>,
                    audio_dir,
                    audio_cache_limit,
                )
                .map_err(|err| {
                    log::warn!("librespot: failed to create audio cache: {err}");
                    err
                })
                .ok()
            })
            .inspect(|_| {
                log::info!("librespot: audio cache enabled");
            });

        let session = {
            let _guard = runtime.enter();
            Session::new(SessionConfig::default(), cache)
        };

        let libre_creds = to_librespot_credentials(creds);
        runtime
            .block_on(session.connect(libre_creds, true))
            .map_err(map_error)?;

        let mixer = Arc::new(
            SoftMixer::open(MixerConfig {
                volume_ctrl: VolumeCtrl::Log(VolumeCtrl::DEFAULT_DB_RANGE),
                ..MixerConfig::default()
            })
            .map_err(map_error)?,
        );

        let volume_getter = mixer.get_soft_volume();
        let sink = audio_backend::find(None).ok_or_else(|| {
            Error::AudioOutputError(Box::new(io::Error::other(
                "librespot audio backend not available",
            )))
        })?;
        let player_config = build_player_config(config);
        let player = LibrespotPlayer::new(player_config, session, volume_getter, move || {
            sink(None, librespot_playback::config::AudioFormat::default())
        });

        let transitioning = Arc::new(AtomicBool::new(false));
        let event_thread =
            spawn_event_forwarder(Arc::clone(&player), sender, Arc::clone(&transitioning));

        Ok(Self {
            player,
            mixer,
            transitioning,
            _runtime: runtime,
            _event_thread: event_thread,
        })
    }

    pub fn load(&self, item: PlaybackItem, start_playing: bool, position: Duration) {
        if let Some(uri) = item_id_to_uri(item.item_id) {
            self.player.load(
                uri,
                start_playing,
                position.as_millis().min(u32::MAX as u128) as u32,
            );
        } else {
            log::warn!("librespot: unsupported item id {:?}", item.item_id);
        }
    }

    /// Stop the current track as part of a transition to a new track.
    /// The resulting `Stopped` event from librespot will be suppressed
    /// so the UI/core don't treat it as a real playback stop.
    pub fn stop_for_transition(&self) {
        self.transitioning.store(true, Ordering::SeqCst);
        self.player.stop();
    }

    pub fn preload(&self, item: PlaybackItem) {
        if let Some(uri) = item_id_to_uri(item.item_id) {
            self.player.preload(uri);
        }
    }

    pub fn play(&self) {
        self.player.play();
    }

    pub fn pause(&self) {
        self.player.pause();
    }

    pub fn stop(&self) {
        self.player.stop();
    }

    pub fn seek(&self, position: Duration) {
        self.player
            .seek(position.as_millis().min(u32::MAX as u128) as u32);
    }

    pub fn set_volume(&self, volume: f64) {
        let volume = (volume.clamp(0.0, 1.0) * f64::from(u16::MAX)).round() as u16;
        self.mixer.set_volume(volume);
        self.player.emit_volume_changed_event(volume);
    }
}

fn build_player_config(config: &PlaybackConfig) -> PlayerConfig {
    let bitrate = match config.bitrate {
        96 => Bitrate::Bitrate96,
        160 => Bitrate::Bitrate160,
        _ => Bitrate::Bitrate320,
    };
    PlayerConfig {
        bitrate,
        normalisation: config.normalization_enabled,
        normalisation_pregain_db: f64::from(config.pregain),
        position_update_interval: Some(Duration::from_millis(500)),
        ..PlayerConfig::default()
    }
}

fn spawn_event_forwarder(
    player: Arc<LibrespotPlayer>,
    sender: Sender<PlayerEvent>,
    transitioning: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let runtime = Runtime::new().expect("librespot event runtime");
        runtime.block_on(async move {
            let mut current_item: Option<ItemId> = None;
            let mut events = player.get_player_event_channel();
            while let Some(event) = events.recv().await {
                match event {
                    LibrespotEvent::Loading { track_id, .. } => {
                        // A new track is loading -- clear any lingering transition flag.
                        // This acts as a safety net: even if Stopped was never emitted,
                        // the transition is over once the next track starts loading.
                        transitioning.store(false, Ordering::SeqCst);
                        if let Some(item_id) = spotify_uri_to_item_id(&track_id) {
                            let item = PlaybackItem {
                                item_id,
                                norm_level: NormalizationLevel::None,
                            };
                            let _ = sender.send(PlayerEvent::Loading { item });
                        }
                    }
                    LibrespotEvent::Playing {
                        track_id,
                        position_ms,
                        ..
                    } => {
                        if let Some(item_id) = spotify_uri_to_item_id(&track_id) {
                            let path = make_media_path(item_id);
                            let position = Duration::from_millis(position_ms as u64);
                            if current_item == Some(item_id) {
                                let _ = sender.send(PlayerEvent::Resuming { path, position });
                            } else {
                                current_item = Some(item_id);
                                let _ = sender.send(PlayerEvent::Playing { path, position });
                            }
                        }
                    }
                    LibrespotEvent::Paused {
                        track_id,
                        position_ms,
                        ..
                    } => {
                        if let Some(item_id) = spotify_uri_to_item_id(&track_id) {
                            let path = make_media_path(item_id);
                            let position = Duration::from_millis(position_ms as u64);
                            let _ = sender.send(PlayerEvent::Pausing { path, position });
                        }
                    }
                    LibrespotEvent::PositionChanged {
                        track_id,
                        position_ms,
                        ..
                    } => {
                        if let Some(item_id) = spotify_uri_to_item_id(&track_id) {
                            let path = make_media_path(item_id);
                            let position = Duration::from_millis(position_ms as u64);
                            let _ = sender.send(PlayerEvent::Position { path, position });
                        }
                    }
                    LibrespotEvent::EndOfTrack { .. } => {
                        let _ = sender.send(PlayerEvent::EndOfTrack);
                    }
                    LibrespotEvent::Stopped { .. } => {
                        // If this stop was caused by a track transition (stop_for_transition),
                        // suppress the event so the UI/core don't clear the queue.
                        if transitioning.swap(false, Ordering::SeqCst) {
                            log::debug!("librespot: suppressed transition Stopped event");
                        } else {
                            current_item = None;
                            let _ = sender.send(PlayerEvent::Stopped);
                        }
                    }
                    LibrespotEvent::Unavailable { .. } => {
                        current_item = None;
                        let _ = sender.send(PlayerEvent::Stopped);
                    }
                    _ => {}
                }
            }
        });
    })
}

fn make_media_path(item_id: ItemId) -> MediaPath {
    MediaPath {
        item_id,
        file_id: FileId::default(),
        file_format: AudioFormat::OggVorbis,
        duration: Duration::ZERO,
    }
}

fn item_id_to_uri(item_id: ItemId) -> Option<SpotifyUri> {
    let uri = match item_id.id_type {
        ItemIdType::Track => format!("spotify:track:{}", item_id.to_base62()),
        ItemIdType::Podcast => format!("spotify:episode:{}", item_id.to_base62()),
        _ => return None,
    };
    SpotifyUri::from_uri(&uri).ok()
}

fn spotify_uri_to_item_id(uri: &SpotifyUri) -> Option<ItemId> {
    let uri = uri.to_uri().ok()?;
    ItemId::from_uri(&uri)
}

fn to_librespot_credentials(creds: &Credentials) -> LibrespotCredentials {
    LibrespotCredentials {
        username: creds.username.clone(),
        auth_type: creds.auth_type,
        auth_data: creds.auth_data.clone(),
    }
}

fn map_error(err: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::InvalidStateError(Box::new(err))
}
