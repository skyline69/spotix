use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::Sender;
use druid::{
    Code, ExtEventSink, InternalLifeCycle, KbKey, Target, WindowHandle,
    im::Vector,
    widget::{Controller, prelude::*},
};
use rustfm_scrobble::Scrobbler;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};
use spotix_core::{
    audio::{normalize::NormalizationLevel, output::DefaultAudioOutput},
    cache::Cache,
    cdn::Cdn,
    lastfm::LastFmClient,
    player::{PlaybackConfig, Player, PlayerCommand, PlayerEvent, item::PlaybackItem},
    session::SessionService,
};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    cmd,
    cmd::RestoreSnapshot,
    data::Nav,
    data::{
        AppState, Config, NowPlaying, Playable, Playback, PlaybackOrigin, PlaybackState,
        QueueBehavior, QueueEntry,
    },
    ui::lyrics,
    webapi::WebApi,
};
use serde_json;

pub struct PlaybackController {
    sender: Option<Sender<PlayerEvent>>,
    thread: Option<JoinHandle<()>>,
    output: Option<DefaultAudioOutput>,
    media_controls: Option<MediaControls>,
    has_scrobbled: bool,
    scrobbler: Option<Scrobbler>,
    startup: bool,
    pending_restore: Option<PendingRestore>,
    snapshot_path: Option<PathBuf>,
}

struct PendingRestore {
    progress: Duration,
    is_playing: bool,
}

static SNAPSHOT_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
fn init_scrobbler_instance(data: &AppState) -> Option<Scrobbler> {
    if data.config.lastfm_enable {
        if let (Some(api_key), Some(api_secret), Some(session_key)) = (
            data.config.lastfm_api_key.as_deref(),
            data.config.lastfm_api_secret.as_deref(),
            data.config.lastfm_session_key.as_deref(),
        ) {
            match LastFmClient::create_scrobbler(Some(api_key), Some(api_secret), Some(session_key))
            {
                Ok(scr) => {
                    log::info!("Last.fm Scrobbler instance created/updated.");
                    return Some(scr);
                }
                Err(e) => {
                    log::warn!("Failed to create/update Last.fm Scrobbler instance: {e}");
                }
            }
        } else {
            log::info!("Last.fm credentials incomplete or removed, clearing Scrobbler instance.");
        }
    } else {
        log::info!("Last.fm scrobbling is disabled, clearing Scrobbler instance.");
    }
    None
}

impl PlaybackController {
    pub fn new() -> Self {
        Self {
            sender: None,
            thread: None,
            output: None,
            media_controls: None,
            has_scrobbled: false,
            scrobbler: None,
            startup: true,
            pending_restore: None,
            snapshot_path: Config::last_playback_path(),
        }
    }

    fn open_audio_output_and_start_threads(
        &mut self,
        session: SessionService,
        config: PlaybackConfig,
        event_sink: ExtEventSink,
        widget_id: WidgetId,
        #[allow(unused_variables)] window: &WindowHandle,
    ) {
        let output = DefaultAudioOutput::open().unwrap();
        let cache_dir = Config::cache_dir().unwrap();
        let proxy_url = Config::proxy();
        let player = Player::new(
            session.clone(),
            Cdn::new(session, proxy_url.as_deref()).unwrap(),
            Cache::new(cache_dir).unwrap(),
            config,
            &output,
        );

        self.media_controls = Self::create_media_controls(player.sender(), window)
            .map_err(|err| log::error!("failed to connect to media control interface: {err:?}"))
            .ok();

        self.sender = Some(player.sender());
        self.thread = Some(thread::spawn(move || {
            Self::service_events(player, event_sink, widget_id);
        }));
        self.output.replace(output);
    }

    fn service_events(mut player: Player, event_sink: ExtEventSink, widget_id: WidgetId) {
        for event in player.receiver() {
            // Forward events that affect the UI state to the UI thread.
            match &event {
                PlayerEvent::Loading { item } => {
                    event_sink
                        .submit_command(cmd::PLAYBACK_LOADING, item.item_id, widget_id)
                        .unwrap();
                }
                PlayerEvent::Playing { path, position } => {
                    let progress = position.to_owned();
                    event_sink
                        .submit_command(cmd::PLAYBACK_PLAYING, (path.item_id, progress), widget_id)
                        .unwrap();
                }
                PlayerEvent::Pausing { .. } => {
                    event_sink
                        .submit_command(cmd::PLAYBACK_PAUSING, (), widget_id)
                        .unwrap();
                }
                PlayerEvent::Resuming { .. } => {
                    event_sink
                        .submit_command(cmd::PLAYBACK_RESUMING, (), widget_id)
                        .unwrap();
                }
                PlayerEvent::Position { position, .. } => {
                    let progress = position.to_owned();
                    event_sink
                        .submit_command(cmd::PLAYBACK_PROGRESS, progress, widget_id)
                        .unwrap();
                }
                PlayerEvent::Blocked { .. } => {
                    event_sink
                        .submit_command(cmd::PLAYBACK_BLOCKED, (), widget_id)
                        .unwrap();
                }
                PlayerEvent::Stopped => {
                    event_sink
                        .submit_command(cmd::PLAYBACK_STOPPED, (), widget_id)
                        .unwrap();
                }
                _ => {}
            }

            // Let the player react to its internal events.
            player.handle(event);
        }
    }

    fn create_media_controls(
        sender: Sender<PlayerEvent>,
        #[allow(unused_variables)] window: &WindowHandle,
    ) -> Result<MediaControls, souvlaki::Error> {
        let hwnd = {
            #[cfg(target_os = "windows")]
            {
                use druid_shell::raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
                let handle = match window.raw_window_handle() {
                    RawWindowHandle::Win32(h) => h,
                    _ => unreachable!(),
                };
                Some(handle.hwnd)
            }
            #[cfg(not(target_os = "windows"))]
            None
        };

        let mut media_controls = MediaControls::new(PlatformConfig {
            dbus_name: format!("com.skyline69.spotix.{}", random_lowercase_string(8)).as_str(),
            display_name: "Spotix",
            hwnd,
        })?;

        media_controls.attach(move |event| {
            Self::handle_media_control_event(event, &sender);
        })?;

        Ok(media_controls)
    }

    fn handle_media_control_event(event: MediaControlEvent, sender: &Sender<PlayerEvent>) {
        let cmd = match event {
            MediaControlEvent::Play => PlayerEvent::Command(PlayerCommand::Resume),
            MediaControlEvent::Pause => PlayerEvent::Command(PlayerCommand::Pause),
            MediaControlEvent::Toggle => PlayerEvent::Command(PlayerCommand::PauseOrResume),
            MediaControlEvent::Next => PlayerEvent::Command(PlayerCommand::Next),
            MediaControlEvent::Previous => PlayerEvent::Command(PlayerCommand::Previous),
            MediaControlEvent::SetPosition(MediaPosition(duration)) => {
                PlayerEvent::Command(PlayerCommand::Seek { position: duration })
            }
            _ => {
                return;
            }
        };
        sender.send(cmd).unwrap();
    }

    fn update_media_control_playback(&mut self, playback: &Playback) {
        if let Some(media_controls) = self.media_controls.as_mut() {
            let progress = playback
                .now_playing
                .as_ref()
                .map(|now_playing| MediaPosition(now_playing.progress));
            media_controls
                .set_playback(match playback.state {
                    PlaybackState::Loading | PlaybackState::Stopped => MediaPlayback::Stopped,
                    PlaybackState::Playing => MediaPlayback::Playing { progress },
                    PlaybackState::Paused => MediaPlayback::Paused { progress },
                })
                .unwrap_or_default();
        }
    }

    fn update_media_control_metadata(&mut self, playback: &Playback) {
        if let Some(media_controls) = self.media_controls.as_mut() {
            let title = playback.now_playing.as_ref().map(|p| p.item.name().clone());
            let album = playback
                .now_playing
                .as_ref()
                .and_then(|p| p.item.track())
                .map(|t| t.album_name());
            let artist = playback
                .now_playing
                .as_ref()
                .and_then(|p| p.item.track())
                .map(|t| t.artist_name());
            let duration = playback.now_playing.as_ref().map(|p| p.item.duration());
            let cover_url = playback
                .now_playing
                .as_ref()
                .and_then(|p| p.cover_image_url(512.0, 512.0));
            media_controls
                .set_metadata(MediaMetadata {
                    title: title.as_deref(),
                    album: album.as_deref(),
                    artist: artist.as_deref(),
                    duration,
                    cover_url,
                })
                .unwrap();
        }
    }

    fn send(&mut self, event: PlayerEvent) {
        if let Some(s) = &self.sender {
            s.send(event)
                .map_err(|e| log::error!("error sending message: {e:?}"))
                .ok();
        }
    }

    fn report_now_playing(&mut self, playback: &Playback) {
        if let Some(now_playing) = playback.now_playing.as_ref()
            && let Playable::Track(track) = &now_playing.item
        {
            if let Some(scrobbler) = &self.scrobbler {
                let artist = track.artist_name();
                let title = track.name.clone();
                let album = track.album.clone();

                if let Err(e) = LastFmClient::now_playing_song(
                    scrobbler,
                    artist.as_ref(),
                    title.as_ref(),
                    album.as_ref().map(|a| a.name.as_ref()),
                ) {
                    log::warn!("failed to report 'Now Playing' to Last.fm: {e}");
                } else {
                    log::info!("reported 'Now Playing' to Last.fm: {artist} - {title}");
                }
            } else {
                log::debug!("Last.fm not configured, skipping now_playing report.");
            }
        }
    }

    fn report_scrobble(&mut self, playback: &Playback) {
        if let Some(now_playing) = playback.now_playing.as_ref()
            && let Playable::Track(track) = &now_playing.item
            && now_playing.progress >= track.duration / 2
            && !self.has_scrobbled
        {
            if let Some(scrobbler) = &self.scrobbler {
                let artist = track.artist_name();
                let title = track.name.clone();
                let album = track.album.clone();

                if let Err(e) = LastFmClient::scrobble_song(
                    scrobbler,
                    artist.as_ref(),
                    title.as_ref(),
                    album.as_ref().map(|a| a.name.as_ref()),
                ) {
                    log::warn!("failed to scrobble track to Last.fm: {e}");
                } else {
                    log::info!("scrobbled track to Last.fm: {artist} - {title}");
                    self.has_scrobbled = true;
                }
            } else {
                log::debug!("Last.fm not configured, skipping scrobble.");
            }
        }
    }

    fn play(&mut self, items: &Vector<QueueEntry>, position: usize) {
        let playback_items = items.iter().map(|queued| PlaybackItem {
            item_id: queued.item.id(),
            norm_level: match queued.origin {
                PlaybackOrigin::Album(_) => NormalizationLevel::Album,
                _ => NormalizationLevel::Track,
            },
        });
        let playback_items_vec: Vec<PlaybackItem> = playback_items.collect();

        // Make sure position is within bounds
        let position = if position >= playback_items_vec.len() {
            0
        } else {
            position
        };

        self.send(PlayerEvent::Command(PlayerCommand::LoadQueue {
            items: playback_items_vec,
            position,
        }));
    }

    fn pause(&mut self) {
        self.send(PlayerEvent::Command(PlayerCommand::Pause));
    }

    fn resume(&mut self) {
        self.send(PlayerEvent::Command(PlayerCommand::Resume));
    }

    fn pause_or_resume(&mut self) {
        self.send(PlayerEvent::Command(PlayerCommand::PauseOrResume));
    }

    fn previous(&mut self) {
        self.send(PlayerEvent::Command(PlayerCommand::Previous));
    }

    fn next(&mut self) {
        self.send(PlayerEvent::Command(PlayerCommand::Next));
    }

    fn stop(&mut self) {
        self.send(PlayerEvent::Command(PlayerCommand::Stop));
    }

    fn seek(&mut self, position: Duration) {
        self.send(PlayerEvent::Command(PlayerCommand::Seek { position }));
    }

    fn seek_relative(&mut self, data: &AppState, forward: bool) {
        if let Some(now_playing) = &data.playback.now_playing {
            let seek_duration = Duration::from_secs(data.config.seek_duration as u64);

            // Calculate new position, ensuring it does not exceed duration for forward seeks.
            let seek_position = if forward {
                now_playing.progress + seek_duration
            } else {
                now_playing.progress.saturating_sub(seek_duration)
            }
            .min(now_playing.item.duration());

            self.seek(seek_position);
        }
    }

    fn set_volume(&mut self, volume: f64) {
        self.send(PlayerEvent::Command(PlayerCommand::SetVolume { volume }));
    }

    fn add_to_queue(&mut self, item: &PlaybackItem) {
        self.send(PlayerEvent::Command(PlayerCommand::AddToQueue {
            item: *item,
        }));
    }

    fn set_queue_behavior(&mut self, behavior: QueueBehavior) {
        self.send(PlayerEvent::Command(PlayerCommand::SetQueueBehavior {
            behavior: match behavior {
                QueueBehavior::Sequential => spotix_core::player::queue::QueueBehavior::Sequential,
                QueueBehavior::Random => spotix_core::player::queue::QueueBehavior::Random,
                QueueBehavior::LoopTrack => spotix_core::player::queue::QueueBehavior::LoopTrack,
                QueueBehavior::LoopAll => spotix_core::player::queue::QueueBehavior::LoopAll,
            },
        }));
    }

    fn update_lyrics(&mut self, ctx: &mut EventCtx, data: &AppState, now_playing: &NowPlaying) {
        if matches!(data.nav, Nav::Lyrics) {
            ctx.submit_command(lyrics::SHOW_LYRICS.with(now_playing.clone()));
        }
    }

    fn load_snapshot(&mut self, sink: ExtEventSink, widget_id: WidgetId) {
        let Some(path) = self.snapshot_path.clone() else {
            return;
        };

        thread::spawn(move || {
            fn parse_snapshot(contents: &str, path: &PathBuf) -> Option<RestoreSnapshot> {
                if let Ok(s) = serde_json::from_str::<RestoreSnapshot>(contents) {
                    return Some(s);
                }
                let mut value: serde_json::Value = serde_json::from_str(contents).ok()?;
                if let Some(track) = value
                    .get_mut("track")
                    .and_then(|t| t.as_object_mut())
                    .cloned()
                {
                    let mut track = track;
                    if let Some(dur) = track.get("duration_ms") {
                        if let (Some(secs), Some(nanos)) = (
                            dur.get("secs").and_then(|v| v.as_u64()),
                            dur.get("nanos").and_then(|v| v.as_u64()),
                        ) {
                            let millis = secs.saturating_mul(1000) + nanos / 1_000_000;
                            track
                                .insert("duration_ms".to_string(), serde_json::Value::from(millis));
                            value["track"] = serde_json::Value::Object(track);
                            if let Ok(snap) = serde_json::from_value::<RestoreSnapshot>(value) {
                                log::info!("parsed legacy playback snapshot from {:?}", path);
                                return Some(snap);
                            }
                        }
                    }
                }
                None
            }

            let snapshot_opt = match fs::read_to_string(&path) {
                Ok(contents) => match parse_snapshot(&contents, &path) {
                    Some(s) => {
                        log::info!("loaded playback snapshot from {:?}", path);
                        Some(s)
                    }
                    None => {
                        log::warn!(
                            "invalid playback snapshot {:?}: ({} bytes)",
                            path,
                            contents.len()
                        );
                        None
                    }
                },
                Err(err) => {
                    log::debug!("no playback snapshot {:?}: {err}", path);
                    None
                }
            };

            if let Some(snapshot) = snapshot_opt.clone() {
                if let Err(err) = sink.submit_command(
                    cmd::RESTORE_SNAPSHOT_READY,
                    snapshot,
                    Target::Widget(widget_id),
                ) {
                    log::error!("failed to dispatch snapshot restore: {err}");
                }
            }
            if snapshot_opt.is_none() {
                let _ = fs::remove_file(&path);
            }
        });
    }

    fn save_snapshot(&self, now_playing: &NowPlaying, state: PlaybackState) {
        let Some(path) = self.snapshot_path.clone() else {
            return;
        };

        let (id, is_episode, track_snapshot) = match &now_playing.item {
            Playable::Track(track) => {
                if track.is_local {
                    return;
                }
                let album = track
                    .album
                    .as_ref()
                    .map(|a| cmd::SnapshotAlbum {
                        id: a.id.to_string(),
                        name: a.name.clone(),
                        images: a.images.iter().cloned().collect(),
                    })
                    .unwrap_or(cmd::SnapshotAlbum {
                        id: String::new(),
                        name: Arc::from(""),
                        images: Vec::new(),
                    });
                let artists = track
                    .artists
                    .iter()
                    .map(|a| cmd::SnapshotArtist {
                        id: a.id.to_string(),
                        name: a.name.clone(),
                    })
                    .collect();
                let snap = cmd::SnapshotTrack {
                    id: track.id.0.to_base62(),
                    name: track.name.clone(),
                    album,
                    artists,
                    duration_ms: track.duration.as_millis() as u64,
                    explicit: track.explicit,
                    is_local: track.is_local,
                };
                (snap.id.clone(), false, Some(snap))
            }
            Playable::Episode(episode) => (episode.id.0.to_base62(), true, None),
        };

        let snapshot = RestoreSnapshot {
            id,
            is_episode,
            origin: now_playing.origin.clone(),
            progress_ms: now_playing.progress.as_millis().min(u64::MAX as u128) as u64,
            is_playing: matches!(state, PlaybackState::Playing),
            track: track_snapshot,
        };

        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _guard = SNAPSHOT_WRITE_LOCK.lock().ok();
        let tmp = path.with_extension("tmp");
        match fs::File::create(&tmp) {
            Ok(file) => {
                let mut writer = std::io::BufWriter::new(file);
                if let Err(err) = serde_json::to_writer(&mut writer, &snapshot) {
                    log::warn!("failed to serialize snapshot to {:?}: {err}", tmp);
                    let _ = fs::remove_file(&tmp);
                    return;
                }
                if let Err(err) = writer.flush() {
                    log::warn!("failed to flush snapshot {:?}: {err}", tmp);
                    let _ = fs::remove_file(&tmp);
                    return;
                }
                match fs::rename(&tmp, &path) {
                    Ok(_) => log::debug!("saved playback snapshot to {:?}", path),
                    Err(err) => log::warn!("failed to store snapshot {:?}: {err}", path),
                }
            }
            Err(err) => log::warn!("failed to create snapshot temp {:?}: {err}", tmp),
        }
    }
}

impl<W> Controller<AppState, W> for PlaybackController
where
    W: Widget<AppState>,
{
    fn event(
        &mut self,
        child: &mut W,
        ctx: &mut EventCtx,
        event: &Event,
        data: &mut AppState,
        env: &Env,
    ) {
        match event {
            Event::Command(cmd) if cmd.is(cmd::SET_FOCUS) => {
                ctx.request_focus();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAYBACK_LOADING) => {
                let item = cmd.get_unchecked(cmd::PLAYBACK_LOADING);

                if let Some(queued) = data.queued_entry(*item) {
                    data.loading_playback(queued.item, queued.origin);
                    self.update_media_control_playback(&data.playback);
                    self.update_media_control_metadata(&data.playback);
                } else {
                    log::warn!("loaded item not found in playback queue");
                }
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAYBACK_PLAYING) => {
                let (item, progress) = cmd.get_unchecked(cmd::PLAYBACK_PLAYING);

                // Song has changed, so we reset the has_scrobbled value
                self.has_scrobbled = false;
                self.report_now_playing(&data.playback);

                if let Some(queued) = data.queued_entry(*item) {
                    data.start_playback(queued.item, queued.origin, progress.to_owned());
                    self.update_media_control_playback(&data.playback);
                    self.update_media_control_metadata(&data.playback);
                    if let Some(now_playing) = &data.playback.now_playing {
                        self.save_snapshot(now_playing, data.playback.state);
                        self.update_lyrics(ctx, data, now_playing);
                    }
                    if let Some(pending) = self.pending_restore.take() {
                        if let Some(now_playing) = &data.playback.now_playing {
                            let progress = pending.progress.min(now_playing.item.duration());
                            if progress > Duration::ZERO {
                                self.seek(progress);
                            }
                        }
                        if !pending.is_playing {
                            self.pause();
                        }
                    }
                } else {
                    log::warn!("played item not found in playback queue");
                }
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAYBACK_PROGRESS) => {
                let progress = cmd.get_unchecked(cmd::PLAYBACK_PROGRESS);
                data.progress_playback(progress.to_owned());

                self.report_scrobble(&data.playback);
                self.update_media_control_playback(&data.playback);
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAYBACK_PAUSING) => {
                data.pause_playback();
                if let Some(now_playing) = &data.playback.now_playing {
                    self.save_snapshot(now_playing, data.playback.state);
                }
                self.update_media_control_playback(&data.playback);
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAYBACK_RESUMING) => {
                data.resume_playback();
                if let Some(now_playing) = &data.playback.now_playing {
                    self.save_snapshot(now_playing, data.playback.state);
                }
                self.update_media_control_playback(&data.playback);
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAYBACK_BLOCKED) => {
                data.block_playback();
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAYBACK_STOPPED) => {
                data.stop_playback();
                self.update_media_control_playback(&data.playback);
                ctx.set_handled();
            }
            // Remote playback restore removed; using local snapshot file instead.
            Event::Command(cmd) if cmd.is(cmd::RESTORE_SNAPSHOT_READY) => {
                let snapshot = cmd.get_unchecked(cmd::RESTORE_SNAPSHOT_READY).clone();
                let sink = ctx.get_external_handle();
                let widget_id = ctx.widget_id();
                let snapshot_path = self.snapshot_path.clone();
                thread::spawn(move || {
                    let api = WebApi::global();
                    // Prefer cached track data if available to avoid fetch failures.
                    let from_cache = snapshot.track.clone().map(|t| {
                        let album_link = crate::data::AlbumLink {
                            id: Arc::from(t.album.id),
                            name: t.album.name,
                            images: t.album.images.into_iter().collect(),
                        };
                        let artists = t
                            .artists
                            .into_iter()
                            .map(|a| crate::data::ArtistLink {
                                id: Arc::from(a.id),
                                name: a.name,
                            })
                            .collect();
                        let track = crate::data::Track {
                            id: crate::data::TrackId(
                                spotix_core::item_id::ItemId::from_base62(
                                    &t.id,
                                    spotix_core::item_id::ItemIdType::Track,
                                )
                                .unwrap_or(spotix_core::item_id::ItemId::INVALID),
                            ),
                            name: t.name,
                            album: Some(album_link),
                            artists,
                            duration: Duration::from_millis(t.duration_ms),
                            disc_number: 1,
                            track_number: 1,
                            explicit: t.explicit,
                            is_local: t.is_local,
                            local_path: None,
                            is_playable: None,
                            popularity: None,
                            track_pos: 0,
                            lyrics: None,
                        };
                        Playable::Track(Arc::new(track))
                    });

                    let fetched = if snapshot.is_episode {
                        match api.get_episode(&snapshot.id) {
                            Ok(ep) => Some(Playable::Episode(ep)),
                            Err(err) => {
                                log::warn!(
                                    "snapshot restore failed for episode {}: {err}",
                                    snapshot.id
                                );
                                None
                            }
                        }
                    } else {
                        match api.get_track(&snapshot.id) {
                            Ok(track) => Some(Playable::Track(track)),
                            Err(err) => {
                                log::warn!(
                                    "snapshot restore failed for track {}: {err}",
                                    snapshot.id
                                );
                                None
                            }
                        }
                    };

                    let playable = fetched.or(from_cache);

                    if let Some(playable) = playable {
                        let entry = QueueEntry {
                            item: playable,
                            origin: snapshot.origin,
                        };
                        log::info!("restoring playback snapshot for id {}", snapshot.id);
                        let _ = sink.submit_command(
                            cmd::RESTORE_SNAPSHOT_RESOLVED,
                            (entry, snapshot.progress_ms, snapshot.is_playing),
                            widget_id,
                        );
                    } else {
                        log::warn!(
                            "failed to resolve snapshot id {}, skipping restore",
                            snapshot.id
                        );
                        if let Some(path) = snapshot_path {
                            let _ = fs::remove_file(path);
                        }
                    }
                });
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::RESTORE_SNAPSHOT_RESOLVED) => {
                let (entry, progress_ms, is_playing) =
                    cmd.get_unchecked(cmd::RESTORE_SNAPSHOT_RESOLVED);
                let mut queue = Vector::new();
                queue.push_back(entry.clone());
                data.playback.queue = queue;
                self.pending_restore = Some(PendingRestore {
                    progress: Duration::from_millis(*progress_ms),
                    is_playing: *is_playing,
                });
                self.play(&data.playback.queue, 0);
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAY_TRACKS) => {
                let payload = cmd.get_unchecked(cmd::PLAY_TRACKS);
                data.playback.queue = payload
                    .items
                    .iter()
                    .map(|item| QueueEntry {
                        origin: payload.origin.to_owned(),
                        item: item.to_owned(),
                    })
                    .collect();

                self.play(&data.playback.queue, payload.position);
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAY_PAUSE) => {
                self.pause();
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAY_RESUME) => {
                self.resume();
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAY_PREVIOUS) => {
                self.previous();
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAY_NEXT) => {
                self.next();
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAY_STOP) => {
                self.stop();
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::ADD_TO_QUEUE) => {
                log::info!("adding to queue");
                let (entry, item) = cmd.get_unchecked(cmd::ADD_TO_QUEUE);

                self.add_to_queue(item);
                data.add_queued_entry(entry.clone());
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAY_QUEUE_BEHAVIOR) => {
                let behavior = cmd.get_unchecked(cmd::PLAY_QUEUE_BEHAVIOR);
                data.set_queue_behavior(behavior.to_owned());
                self.set_queue_behavior(behavior.to_owned());
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::PLAY_SEEK) => {
                if let Some(now_playing) = &data.playback.now_playing {
                    let fraction = cmd.get_unchecked(cmd::PLAY_SEEK);
                    let position = Duration::from_secs_f64(
                        now_playing.item.duration().as_secs_f64() * fraction,
                    );
                    self.seek(position);
                }
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::SKIP_TO_POSITION) => {
                let location = cmd.get_unchecked(cmd::SKIP_TO_POSITION);
                self.seek(Duration::from_millis(*location));
                ctx.set_handled();
            }
            // Keyboard shortcuts.
            Event::KeyDown(key) if key.code == Code::Space => {
                self.pause_or_resume();
                ctx.set_handled();
            }
            Event::KeyDown(key) if key.code == Code::ArrowRight => {
                if key.mods.shift() {
                    self.next();
                } else {
                    self.seek_relative(data, true);
                }
                ctx.set_handled();
            }
            Event::KeyDown(key) if key.code == Code::ArrowLeft => {
                if key.mods.shift() {
                    self.previous();
                } else {
                    self.seek_relative(data, false);
                }
                ctx.set_handled();
            }
            Event::KeyDown(key) if key.key == KbKey::Character("+".to_string()) => {
                data.playback.volume = (data.playback.volume + 0.1).min(1.0);
                ctx.set_handled();
            }
            Event::KeyDown(key) if key.key == KbKey::Character("-".to_string()) => {
                data.playback.volume = (data.playback.volume - 0.1).max(0.0);
                ctx.set_handled();
            }
            _ => child.event(ctx, event, data, env),
        }
    }

    fn lifecycle(
        &mut self,
        child: &mut W,
        ctx: &mut LifeCycleCtx,
        event: &LifeCycle,
        data: &AppState,
        env: &Env,
    ) {
        match event {
            LifeCycle::WidgetAdded => {
                self.open_audio_output_and_start_threads(
                    data.session.clone(),
                    data.config.playback(),
                    ctx.get_external_handle(),
                    ctx.widget_id(),
                    ctx.window(),
                );

                // Initialize values loaded from the config.
                self.set_volume(data.playback.volume);
                self.set_queue_behavior(data.playback.queue_behavior);
                self.load_snapshot(ctx.get_external_handle(), ctx.widget_id());

                // Request focus so we can receive keyboard events.
                ctx.submit_command(cmd::SET_FOCUS.to(ctx.widget_id()));
            }
            LifeCycle::Internal(InternalLifeCycle::RouteFocusChanged { new: None, .. }) => {
                // Druid doesn't have any "ambient focus" concept, so we catch the situation
                // when the focus is being lost and sign up to get focused ourselves.
                ctx.submit_command(cmd::SET_FOCUS.to(ctx.widget_id()));
            }
            _ => {}
        }
        if self.startup {
            self.startup = false;
            self.scrobbler = init_scrobbler_instance(data);
        }
        child.lifecycle(ctx, event, data, env);
    }

    fn update(
        &mut self,
        child: &mut W,
        ctx: &mut UpdateCtx,
        old_data: &AppState,
        data: &AppState,
        env: &Env,
    ) {
        if !old_data.playback.volume.same(&data.playback.volume) {
            self.set_volume(data.playback.volume);
        }

        let lastfm_changed = old_data.config.lastfm_api_key != data.config.lastfm_api_key
            || old_data.config.lastfm_api_secret != data.config.lastfm_api_secret
            || old_data.config.lastfm_session_key != data.config.lastfm_session_key
            || old_data.config.lastfm_enable != data.config.lastfm_enable;

        if lastfm_changed {
            self.scrobbler = init_scrobbler_instance(data);
        }

        child.update(ctx, old_data, data, env);
    }
}

// This uses the current system time to generate a random lowercase string of a given length.
fn random_lowercase_string(len: usize) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut n = now;
    let mut chars = Vec::new();
    while n > 0 && chars.len() < len {
        let c = ((n % 26) as u8 + b'a') as char;
        chars.push(c);
        n /= 26;
    }
    while chars.len() < len {
        chars.push('a');
    }
    chars.into_iter().rev().collect()
}
