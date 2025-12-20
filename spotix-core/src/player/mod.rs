pub mod file;
pub mod item;
pub mod queue;
mod storage;
mod worker;

use std::{mem, thread, thread::JoinHandle, time::Duration};

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::{
    audio::{
        equalizer::EqConfig,
        output::{AudioOutput, AudioSink, DefaultAudioOutput, DefaultAudioSink},
    },
    cache::CacheHandle,
    cdn::CdnHandle,
    error::Error,
    session::SessionService,
};

use self::{
    file::MediaPath,
    item::{LoadedPlaybackItem, PlaybackItem},
    queue::{Queue, QueueBehavior},
    worker::PlaybackManager,
};

const PREVIOUS_TRACK_THRESHOLD: Duration = Duration::from_secs(3);
const STOP_AFTER_CONSECUTIVE_LOADING_FAILURES: usize = 3;

#[derive(Clone)]
pub struct PlaybackConfig {
    pub bitrate: usize,
    pub pregain: f32,
    pub audio_cache_limit: Option<u64>,
    pub crossfade_duration: Duration,
    pub mono_audio: bool,
    pub eq: EqConfig,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            bitrate: 320,
            pregain: 3.0,
            audio_cache_limit: None,
            crossfade_duration: Duration::from_secs(0),
            mono_audio: false,
            eq: EqConfig::default(),
        }
    }
}

pub struct Player {
    state: PlayerState,
    preload: PreloadState,
    session: SessionService,
    cdn: CdnHandle,
    cache: CacheHandle,
    config: PlaybackConfig,
    queue: Queue,
    sender: Sender<PlayerEvent>,
    receiver: Receiver<PlayerEvent>,
    audio_output_sink: DefaultAudioSink,
    playback_mgr: PlaybackManager,
    consecutive_loading_failures: usize,
    ignore_end_of_track: bool,
}

impl Player {
    pub fn new(
        session: SessionService,
        cdn: CdnHandle,
        cache: CacheHandle,
        config: PlaybackConfig,
        audio_output: &DefaultAudioOutput,
    ) -> Self {
        let (sender, receiver) = unbounded();
        Self {
            playback_mgr: PlaybackManager::new(audio_output.sink(), sender.clone()),
            session,
            cdn,
            cache,
            config,
            sender,
            receiver,
            audio_output_sink: audio_output.sink(),
            state: PlayerState::Stopped,
            preload: PreloadState::None,
            queue: Queue::new(),
            consecutive_loading_failures: 0,
            ignore_end_of_track: false,
        }
    }

    pub fn sender(&self) -> Sender<PlayerEvent> {
        self.sender.clone()
    }

    pub fn receiver(&self) -> Receiver<PlayerEvent> {
        self.receiver.clone()
    }

    pub fn handle(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::Command(cmd) => self.handle_command(cmd),
            PlayerEvent::Loaded { item, result } => self.handle_loaded(item, result),
            PlayerEvent::Preloaded { item, result } => self.handle_preloaded(item, result),
            PlayerEvent::Position { position, path } => self.handle_position(position, path),
            PlayerEvent::EndOfTrack => self.handle_end_of_track(),
            PlayerEvent::Loading { .. }
            | PlayerEvent::Playing { .. }
            | PlayerEvent::Pausing { .. }
            | PlayerEvent::Resuming { .. }
            | PlayerEvent::Stopped
            | PlayerEvent::Blocked { .. } => {}
        };
    }

    fn handle_command(&mut self, cmd: PlayerCommand) {
        match cmd {
            PlayerCommand::LoadQueue { items, position } => self.load_queue(items, position),
            PlayerCommand::LoadAndPlay { item } => self.load_and_play(item),
            PlayerCommand::Preload { item } => self.preload(item),
            PlayerCommand::Pause => self.pause(),
            PlayerCommand::Resume => self.resume(),
            PlayerCommand::PauseOrResume => self.pause_or_resume(),
            PlayerCommand::Previous => self.previous(),
            PlayerCommand::Next => self.next(),
            PlayerCommand::Stop => self.stop(),
            PlayerCommand::Seek { position } => self.seek(position),
            PlayerCommand::Configure { config } => self.configure(config),
            PlayerCommand::SetQueueBehavior { behavior } => self.queue.set_behaviour(behavior),
            PlayerCommand::AddToQueue { item } => self.queue.add(item),
            PlayerCommand::AddNext { item } => self.queue.add_next(item),
            PlayerCommand::ReplaceQueue { items } => self.queue.replace(items),
            PlayerCommand::SetVolume { volume } => self.set_volume(volume),
        }
    }

    fn handle_loaded(&mut self, item: PlaybackItem, result: Result<LoadedPlaybackItem, Error>) {
        match self.state {
            PlayerState::Loading {
                item: requested_item,
                ..
            } if item == requested_item => match result {
                Ok(loaded_item) => {
                    self.consecutive_loading_failures = 0;
                    self.play_loaded(loaded_item);
                }
                Err(err) => {
                    self.consecutive_loading_failures += 1;
                    if self.consecutive_loading_failures < STOP_AFTER_CONSECUTIVE_LOADING_FAILURES {
                        log::error!("skipping, error while loading: {err}");
                        self.next();
                    } else {
                        log::error!("stopping, error while loading: {err}");
                        self.stop();
                    }
                }
            },
            _ => {
                log::info!("stale load result received, ignoring");
            }
        }
    }

    fn handle_preloaded(&mut self, item: PlaybackItem, result: Result<LoadedPlaybackItem, Error>) {
        match self.preload {
            PreloadState::Preloading {
                item: requested_item,
                ..
            } if item == requested_item => match result {
                Ok(loaded_item) => {
                    log::info!("preloaded audio file");
                    self.preload = PreloadState::Preloaded { item, loaded_item };
                }
                Err(err) => {
                    log::error!("failed to preload audio file, error while opening: {err}");
                    self.preload = PreloadState::None;
                }
            },
            _ => {
                log::info!("stale preload result received, ignoring");

                // We are not preloading this item, but because we sometimes extract the
                // preloading thread and use it for loading, let's check if the item is not
                // being loaded now.
                self.handle_loaded(item, result);
            }
        }
    }

    fn handle_position(&mut self, new_position: Duration, reported_path: MediaPath) {
        let current_path = match &mut self.state {
            PlayerState::Playing { path, position } | PlayerState::Paused { path, position } => {
                if path.item_id != reported_path.item_id || path.file_id != reported_path.file_id {
                    log::debug!("ignoring stale position report");
                    return;
                }
                *position = new_position;
                *path
            }
            _ => {
                log::warn!("received unexpected position report");
                return;
            }
        };
        const PRELOAD_BEFORE_END_OF_TRACK: Duration = Duration::from_secs(30);
        let time_until_end_of_track = current_path
            .duration
            .checked_sub(new_position)
            .unwrap_or_default();
        if time_until_end_of_track <= PRELOAD_BEFORE_END_OF_TRACK
            && let Some(&item_to_preload) = self.queue.get_following()
        {
            self.preload(item_to_preload);
        }

        if matches!(self.state, PlayerState::Playing { .. }) {
            self.maybe_start_crossfade(new_position, current_path);
        }
    }

    fn handle_end_of_track(&mut self) {
        if self.ignore_end_of_track {
            self.ignore_end_of_track = false;
            return;
        }
        self.queue.skip_to_following();
        if let Some(&item) = self.queue.get_current() {
            self.load_and_play(item);
        } else {
            self.stop();
        }
    }

    fn load_queue(&mut self, items: Vec<PlaybackItem>, position: usize) {
        self.queue.fill(items, position);
        if let Some(&item) = self.queue.get_current() {
            self.load_and_play(item);
        } else {
            self.stop();
        }
    }

    fn load_and_play(&mut self, item: PlaybackItem) {
        // Make sure to stop the sink, so any current audio source is cleared and the
        // playback stopped.
        self.audio_output_sink.stop();

        // Check if the item is already in the preloader state.
        let loading_handle = match mem::replace(&mut self.preload, PreloadState::None) {
            PreloadState::Preloaded {
                item: preloaded_item,
                loaded_item,
            } if preloaded_item == item => {
                // This item is already loaded in the preloader state.
                self.play_loaded(loaded_item);
                return;
            }

            PreloadState::Preloading {
                item: preloaded_item,
                loading_handle,
            } if preloaded_item == item => {
                // This item is being preloaded. Take it out of the preloader state.
                loading_handle
            }

            preloading_other_file_or_none => {
                self.preload = preloading_other_file_or_none;
                // Item is not preloaded yet, load it in a background thread.
                thread::spawn({
                    let sender = self.sender.clone();
                    let session = self.session.clone();
                    let cdn = self.cdn.clone();
                    let cache = self.cache.clone();
                    let config = self.config.clone();
                    move || {
                        let result = item.load(&session, cdn, cache, &config);
                        sender.send(PlayerEvent::Loaded { item, result }).unwrap();
                    }
                })
            }
        };

        self.sender.send(PlayerEvent::Loading { item }).unwrap();
        self.state = PlayerState::Loading {
            item,
            _loading_handle: loading_handle,
        };
    }

    fn preload(&mut self, item: PlaybackItem) {
        if self.is_in_preload(item) {
            return;
        }
        let loading_handle = thread::spawn({
            let sender = self.sender.clone();
            let session = self.session.clone();
            let cdn = self.cdn.clone();
            let cache = self.cache.clone();
            let config = self.config.clone();
            move || {
                let result = item.load(&session, cdn, cache, &config);
                sender
                    .send(PlayerEvent::Preloaded { item, result })
                    .unwrap();
            }
        });
        self.preload = PreloadState::Preloading {
            item,
            loading_handle,
        };
    }

    fn set_volume(&mut self, volume: f64) {
        self.audio_output_sink.set_volume(volume as f32);
    }

    fn play_loaded(&mut self, loaded_item: LoadedPlaybackItem) {
        log::info!("starting playback");
        let path = loaded_item.file.path();
        let position = Duration::default();
        self.playback_mgr
            .play(loaded_item, self.config.mono_audio, self.config.eq.clone());
        self.state = PlayerState::Playing { path, position };
        self.sender
            .send(PlayerEvent::Playing { path, position })
            .unwrap();
    }

    fn pause(&mut self) {
        match mem::replace(&mut self.state, PlayerState::Invalid) {
            PlayerState::Playing { path, position } | PlayerState::Paused { path, position } => {
                log::info!("pausing playback");
                self.audio_output_sink.pause();
                self.sender
                    .send(PlayerEvent::Pausing { path, position })
                    .unwrap();
                self.state = PlayerState::Paused { path, position };
            }
            _ => {
                log::warn!("invalid state transition");
            }
        }
    }

    fn resume(&mut self) {
        match mem::replace(&mut self.state, PlayerState::Invalid) {
            PlayerState::Playing { path, position } | PlayerState::Paused { path, position } => {
                log::info!("resuming playback");
                self.audio_output_sink.resume();
                self.sender
                    .send(PlayerEvent::Resuming { path, position })
                    .unwrap();
                self.state = PlayerState::Playing { path, position };
            }
            _ => {
                log::warn!("invalid state transition");
            }
        }
    }

    fn pause_or_resume(&mut self) {
        match &self.state {
            PlayerState::Playing { .. } => self.pause(),
            PlayerState::Paused { .. } => self.resume(),
            _ => {
                // Do nothing.
            }
        }
    }

    fn previous(&mut self) {
        if self.is_near_playback_start() {
            self.queue.skip_to_previous();
            if let Some(&item) = self.queue.get_current() {
                self.load_and_play(item);
            } else {
                self.stop();
            }
        } else {
            self.seek(Duration::default());
        }
    }

    fn next(&mut self) {
        self.queue.skip_to_next();
        if let Some(&item) = self.queue.get_current() {
            self.load_and_play(item);
        } else {
            self.stop();
        }
    }

    fn stop(&mut self) {
        self.sender.send(PlayerEvent::Stopped).unwrap();
        self.audio_output_sink.stop();
        self.state = PlayerState::Stopped;
        self.queue.clear();
        self.consecutive_loading_failures = 0;
    }

    fn seek(&mut self, position: Duration) {
        self.playback_mgr.seek(position);
    }

    fn configure(&mut self, config: PlaybackConfig) {
        self.config = config;
    }

    fn maybe_start_crossfade(&mut self, position: Duration, path: MediaPath) {
        if self.config.crossfade_duration.is_zero() {
            return;
        }
        let time_until_end = path.duration.checked_sub(position).unwrap_or_default();
        if time_until_end > self.config.crossfade_duration {
            return;
        }
        let next_item = match self.queue.get_following() {
            Some(&item) => item,
            None => return,
        };
        let loaded_item = match mem::replace(&mut self.preload, PreloadState::None) {
            PreloadState::Preloaded {
                item: preloaded_item,
                loaded_item,
            } if preloaded_item == next_item => loaded_item,
            other => {
                self.preload = other;
                return;
            }
        };

        let next_path = loaded_item.file.path();
        if !self.playback_mgr.start_crossfade(
            loaded_item,
            self.config.crossfade_duration,
            self.config.mono_audio,
            self.config.eq.clone(),
        ) {
            self.preload(next_item);
            return;
        }

        self.queue.skip_to_following();
        self.consecutive_loading_failures = 0;
        self.ignore_end_of_track = true;
        let position = Duration::default();
        self.state = PlayerState::Playing {
            path: next_path,
            position,
        };
        self.sender
            .send(PlayerEvent::Playing {
                path: next_path,
                position,
            })
            .unwrap();
    }

    fn is_near_playback_start(&self) -> bool {
        match self.state {
            PlayerState::Playing { position, .. } | PlayerState::Paused { position, .. } => {
                position < PREVIOUS_TRACK_THRESHOLD
            }
            _ => false,
        }
    }

    fn is_in_preload(&self, item: PlaybackItem) -> bool {
        match self.preload {
            PreloadState::Preloading { item: p_item, .. }
            | PreloadState::Preloaded { item: p_item, .. } => p_item == item,
            _ => false,
        }
    }
}

pub enum PlayerCommand {
    LoadQueue {
        items: Vec<PlaybackItem>,
        position: usize,
    },
    LoadAndPlay {
        item: PlaybackItem,
    },
    Preload {
        item: PlaybackItem,
    },
    Pause,
    Resume,
    PauseOrResume,
    Previous,
    Next,
    Stop,
    Seek {
        position: Duration,
    },
    Configure {
        config: PlaybackConfig,
    },
    SetQueueBehavior {
        behavior: QueueBehavior,
    },
    AddToQueue {
        item: PlaybackItem,
    },
    AddNext {
        item: PlaybackItem,
    },
    ReplaceQueue {
        items: Vec<PlaybackItem>,
    },
    /// Change playback volume to a value in 0.0..=1.0 range.
    SetVolume {
        volume: f64,
    },
}

pub enum PlayerEvent {
    Command(PlayerCommand),
    /// Track has started loading.  `Loaded` follows.
    Loading {
        item: PlaybackItem,
    },
    /// Track loading either succeeded or failed.  `Playing` follows in case of
    /// success.
    Loaded {
        item: PlaybackItem,
        result: Result<LoadedPlaybackItem, Error>,
    },
    /// Next item in queue has been either successfully preloaded or failed to
    /// preload.
    Preloaded {
        item: PlaybackItem,
        result: Result<LoadedPlaybackItem, Error>,
    },
    /// Player has started playing new track.  `Position` events will follow.
    Playing {
        path: MediaPath,
        position: Duration,
    },
    /// Player is in a paused state.  `Resuming` might follow.
    Pausing {
        path: MediaPath,
        position: Duration,
    },
    /// Player is resuming playback of a track.  `Position` events will follow.
    Resuming {
        path: MediaPath,
        position: Duration,
    },
    /// Position of the playback head has changed.
    Position {
        path: MediaPath,
        position: Duration,
    },
    /// Player would like to continue playing, but is blocked, waiting for I/O.
    Blocked {
        path: MediaPath,
        position: Duration,
    },
    /// Player has finished playing a track.  `Loading` or `Playing` might
    /// follow if the queue is not empty, `Stopped` will follow if it is.
    EndOfTrack,
    /// The queue is empty.
    Stopped,
}

enum PlayerState {
    Loading {
        item: PlaybackItem,
        _loading_handle: JoinHandle<()>,
    },
    Playing {
        path: MediaPath,
        position: Duration,
    },
    Paused {
        path: MediaPath,
        position: Duration,
    },
    Stopped,
    Invalid,
}

enum PreloadState {
    Preloading {
        item: PlaybackItem,
        loading_handle: JoinHandle<()>,
    },
    Preloaded {
        item: PlaybackItem,
        loaded_item: LoadedPlaybackItem,
    },
    None,
}
