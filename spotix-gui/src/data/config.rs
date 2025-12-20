use std::{
    env::{self, VarError},
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(target_family = "unix")]
use std::os::unix::fs::OpenOptionsExt;

use druid::{Data, Lens, Size};
use platform_dirs::AppDirs;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use spotix_core::{
    audio::equalizer::EqConfig,
    cache::{CacheHandle, mkdir_if_not_exists},
    connection::Credentials,
    player::PlaybackConfig,
    session::{SessionConfig, SessionConnection},
};

use super::{Nav, Promise, QueueBehavior, SliderScrollScale};
use crate::ui::theme;

#[derive(Clone, Debug, Data, Lens)]
pub struct Preferences {
    pub active: PreferencesTab,
    #[data(ignore)]
    pub cache: Option<CacheHandle>,
    pub cache_usage: Promise<CacheUsage, (), ()>,
    pub auth: Authentication,
    pub lastfm_auth_result: Option<String>,
}

impl Preferences {
    pub fn reset(&mut self) {
        self.cache_usage.clear();
        self.auth.result.clear();
        self.auth.lastfm_api_key_input.clear();
        self.auth.lastfm_api_secret_input.clear();
    }

    pub fn measure_cache_usage() -> Option<CacheUsage> {
        let path = Config::cache_dir()?;
        let mut usage = CacheUsage::default();

        let entries = fs::read_dir(&path).ok()?;
        for entry in entries.flatten() {
            let entry_path = entry.path();
            let size = entry_path
                .metadata()
                .ok()
                .map(|meta| {
                    if meta.is_dir() {
                        get_dir_size(&entry_path).unwrap_or(0)
                    } else {
                        meta.len()
                    }
                })
                .unwrap_or(0);

            if entry_path.is_dir() {
                match entry_path.file_name().and_then(|name| name.to_str()) {
                    Some("audio") => usage.audio += size,
                    Some("track") | Some("episode") | Some("key") => usage.metadata += size,
                    _ => usage.webapi += size,
                }
            } else {
                usage.other += size;
            }
        }

        usage.total = usage.audio + usage.metadata + usage.webapi + usage.other;
        Some(usage)
    }
}

#[derive(Clone, Debug, Data, Lens, Default)]
pub struct CacheUsage {
    pub total: u64,
    pub audio: u64,
    pub metadata: u64,
    pub webapi: u64,
    pub other: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Data)]
pub enum PreferencesTab {
    General,
    Playback,
    Account,
    Cache,
    About,
}

#[derive(Clone, Debug, Data, Lens)]
pub struct Authentication {
    pub username: String,
    pub password: String,
    pub access_token: String,
    pub result: Promise<(), (), String>,
    #[data(ignore)]
    pub lastfm_api_key_input: String,
    #[data(ignore)]
    pub lastfm_api_secret_input: String,
}

impl Authentication {
    pub fn new() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            access_token: String::new(),
            result: Promise::Empty,
            lastfm_api_key_input: String::new(),
            lastfm_api_secret_input: String::new(),
        }
    }

    pub fn session_config(&self) -> SessionConfig {
        SessionConfig {
            login_creds: if !self.access_token.is_empty() {
                Credentials::from_access_token(self.access_token.clone())
            } else {
                Credentials::from_username_and_password(
                    self.username.clone(),
                    self.password.clone(),
                )
            },
            proxy_url: Config::proxy(),
        }
    }

    pub fn authenticate_and_get_credentials(config: SessionConfig) -> Result<Credentials, String> {
        let connection = SessionConnection::open(config).map_err(|err| err.to_string())?;
        Ok(connection.credentials)
    }

    pub fn clear(&mut self) {
        self.username.clear();
        self.password.clear();
    }
}

const APP_NAME: &str = "Spotix";
const CONFIG_FILENAME: &str = "config.json";
const PROXY_ENV_VAR: &str = "SOCKS_PROXY";

#[derive(Clone, Debug, Data, Lens, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[data(ignore)]
    credentials: Option<Credentials>,
    pub audio_quality: AudioQuality,
    pub theme: Theme,
    pub volume: f64,
    pub last_route: Option<Nav>,
    pub queue_behavior: QueueBehavior,
    pub show_track_cover: bool,
    pub window_size: Size,
    pub slider_scroll_scale: SliderScrollScale,
    pub sort_order: SortOrder,
    pub sort_criteria: SortCriteria,
    pub paginated_limit: usize,
    pub seek_duration: usize,
    /// Audio cache limit in megabytes. 0 = unlimited.
    pub audio_cache_limit_mb: f64,
    pub enable_pagination: bool,
    pub crossfade_duration_secs: f64,
    pub mono_audio: bool,
    pub normalization_enabled: bool,
    pub autoplay_enabled: bool,
    pub lastfm_session_key: Option<String>,
    pub lastfm_api_key: Option<String>,
    pub lastfm_api_secret: Option<String>,
    pub lastfm_enable: bool,
    pub eq: EqSettings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            credentials: Default::default(),
            audio_quality: Default::default(),
            theme: Default::default(),
            volume: 1.0,
            last_route: Default::default(),
            queue_behavior: Default::default(),
            show_track_cover: Default::default(),
            window_size: Size::new(theme::grid(80.0), theme::grid(100.0)),
            slider_scroll_scale: Default::default(),
            sort_order: Default::default(),
            sort_criteria: Default::default(),
            paginated_limit: 500,
            seek_duration: 10,
            audio_cache_limit_mb: 4096.0,
            enable_pagination: true,
            crossfade_duration_secs: 0.0,
            mono_audio: false,
            normalization_enabled: true,
            autoplay_enabled: true,
            lastfm_session_key: None,
            lastfm_api_key: None,
            lastfm_api_secret: None,
            lastfm_enable: false,
            eq: EqSettings::default(),
        }
    }
}

impl Config {
    fn app_dirs() -> Option<AppDirs> {
        const USE_XDG_ON_MACOS: bool = false;

        AppDirs::new(Some(APP_NAME), USE_XDG_ON_MACOS)
    }

    pub fn spotify_local_files_file(username: &str) -> Option<PathBuf> {
        AppDirs::new(Some("spotify"), false).map(|dir| {
            let path = format!("Users/{username}-user/local-files.bnk");
            dir.config_dir.join(path)
        })
    }

    pub fn cache_dir() -> Option<PathBuf> {
        Self::app_dirs().map(|dirs| dirs.cache_dir)
    }

    pub fn config_dir() -> Option<PathBuf> {
        Self::app_dirs().map(|dirs| dirs.config_dir)
    }

    pub fn themes_dir() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("themes"))
    }

    pub fn last_playback_path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("last_playback.json"))
    }

    fn config_path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join(CONFIG_FILENAME))
    }

    pub fn load() -> Option<Config> {
        let path = Self::config_path().expect("Failed to get config path");
        if let Ok(file) = File::open(&path) {
            log::info!("loading config: {:?}", &path);
            let reader = BufReader::new(file);
            Some(serde_json::from_reader(reader).expect("Failed to read config"))
        } else {
            None
        }
    }

    pub fn save(&self) {
        let dir = Self::config_dir().expect("Failed to get config dir");
        let path = Self::config_path().expect("Failed to get config path");
        mkdir_if_not_exists(&dir).expect("Failed to create config dir");

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(target_family = "unix")]
        options.mode(0o600);

        let file = options.open(&path).expect("Failed to create config");
        let writer = BufWriter::new(file);

        serde_json::to_writer_pretty(writer, self).expect("Failed to write config");
        log::info!("saved config: {:?}", &path);
    }

    pub fn has_credentials(&self) -> bool {
        self.credentials.is_some()
    }

    pub fn store_credentials(&mut self, credentials: Credentials) {
        self.credentials = Some(credentials);
    }

    pub fn clear_credentials(&mut self) {
        self.credentials = Default::default();
    }

    pub fn username(&self) -> Option<&str> {
        self.credentials
            .as_ref()
            .and_then(|c| c.username.as_deref())
    }

    pub fn session(&self) -> SessionConfig {
        SessionConfig {
            login_creds: self.credentials.clone().expect("Missing credentials"),
            proxy_url: Config::proxy(),
        }
    }

    pub fn playback(&self) -> PlaybackConfig {
        PlaybackConfig {
            bitrate: self.audio_quality.as_bitrate(),
            audio_cache_limit: if self.audio_cache_limit_mb <= 0.0 {
                None
            } else {
                Some((self.audio_cache_limit_mb * 1024.0 * 1024.0) as u64)
            },
            crossfade_duration: Duration::from_secs_f64(self.crossfade_duration_secs.max(0.0)),
            mono_audio: self.mono_audio,
            eq: self.eq.to_core(),
            ..PlaybackConfig::default()
        }
    }

    pub fn proxy() -> Option<String> {
        env::var(PROXY_ENV_VAR).map_or_else(
            |err| match err {
                VarError::NotPresent => None,
                VarError::NotUnicode(_) => {
                    log::error!("proxy URL is not a valid unicode");
                    None
                }
            },
            Some,
        )
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Data, Serialize, Deserialize, Default)]
pub enum AudioQuality {
    Low,
    Normal,
    #[default]
    High,
}

impl AudioQuality {
    fn as_bitrate(self) -> usize {
        match self {
            AudioQuality::Low => 96,
            AudioQuality::Normal => 160,
            AudioQuality::High => 320,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Data, Serialize, Deserialize)]
pub enum EqPreset {
    Flat,
    Acoustic,
    BassBoost,
    Classical,
    Dance,
    Electronic,
    HipHop,
    Jazz,
    Pop,
    Rock,
    TrebleBoost,
    Vocal,
    SmallSpeakers,
    SpokenWord,
    Loudness,
    Custom,
}

impl EqPreset {
    pub fn label(self) -> &'static str {
        match self {
            EqPreset::Flat => "Flat",
            EqPreset::Acoustic => "Acoustic",
            EqPreset::BassBoost => "Bass Boost",
            EqPreset::Classical => "Classical",
            EqPreset::Dance => "Dance",
            EqPreset::Electronic => "Electronic",
            EqPreset::HipHop => "Hip-Hop",
            EqPreset::Jazz => "Jazz",
            EqPreset::Pop => "Pop",
            EqPreset::Rock => "Rock",
            EqPreset::TrebleBoost => "Treble Boost",
            EqPreset::Vocal => "Vocal",
            EqPreset::SmallSpeakers => "Small Speakers",
            EqPreset::SpokenWord => "Spoken Word",
            EqPreset::Loudness => "Loudness",
            EqPreset::Custom => "Custom",
        }
    }
}

#[derive(Clone, Debug, Data, Lens, Serialize, Deserialize, PartialEq)]
pub struct EqSettings {
    pub enabled: bool,
    pub preset: EqPreset,
    pub bands: EqBands,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            preset: EqPreset::Flat,
            bands: EqBands::default(),
        }
    }
}

impl EqSettings {
    pub fn to_core(&self) -> EqConfig {
        EqConfig {
            enabled: self.enabled,
            gains_db: self.bands.as_array(),
        }
    }

    pub fn apply_preset(&mut self, preset: EqPreset) {
        if preset == EqPreset::Custom {
            return;
        }
        self.bands = EqBands::from_preset(preset);
    }
}

#[derive(Clone, Debug, Data, Lens, Serialize, Deserialize, PartialEq)]
pub struct EqBands {
    pub band_31: f64,
    pub band_62: f64,
    pub band_125: f64,
    pub band_250: f64,
    pub band_500: f64,
    pub band_1k: f64,
    pub band_2k: f64,
    pub band_4k: f64,
    pub band_8k: f64,
    pub band_16k: f64,
}

impl Default for EqBands {
    fn default() -> Self {
        Self {
            band_31: 0.0,
            band_62: 0.0,
            band_125: 0.0,
            band_250: 0.0,
            band_500: 0.0,
            band_1k: 0.0,
            band_2k: 0.0,
            band_4k: 0.0,
            band_8k: 0.0,
            band_16k: 0.0,
        }
    }
}

impl EqBands {
    pub fn from_preset(preset: EqPreset) -> Self {
        match preset {
            EqPreset::Flat | EqPreset::Custom => Self::default(),
            EqPreset::Acoustic => Self::from_db([3.0, 3.0, 2.0, 1.0, 0.0, 1.0, 2.0, 2.0, 1.0, 0.0]),
            EqPreset::BassBoost => {
                Self::from_db([6.0, 5.0, 4.0, 3.0, 1.5, 0.0, -1.0, -1.5, -2.0, -2.0])
            }
            EqPreset::Classical => {
                Self::from_db([3.0, 2.0, 1.0, 0.0, -1.0, 0.0, 2.0, 3.0, 4.0, 5.0])
            }
            EqPreset::Dance => Self::from_db([5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0]),
            EqPreset::Electronic => {
                Self::from_db([4.0, 3.0, 0.0, -2.0, -2.0, 0.0, 2.0, 3.0, 4.0, 4.0])
            }
            EqPreset::HipHop => Self::from_db([5.0, 4.0, 3.0, 1.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0]),
            EqPreset::Jazz => Self::from_db([4.0, 3.0, 1.0, 0.0, -2.0, -2.0, 0.0, 1.0, 3.0, 4.0]),
            EqPreset::Pop => Self::from_db([-1.0, 2.0, 4.0, 5.0, 3.0, 0.0, -1.0, -1.0, -1.0, -2.0]),
            EqPreset::Rock => Self::from_db([4.0, 3.0, 1.0, 0.0, -1.0, 1.5, 3.0, 3.5, 3.5, 4.0]),
            EqPreset::TrebleBoost => {
                Self::from_db([-2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 6.0])
            }
            EqPreset::Vocal => {
                Self::from_db([-2.0, -2.0, -1.0, 0.0, 3.0, 4.0, 3.0, 2.0, 0.0, -1.0])
            }
            EqPreset::SmallSpeakers => {
                Self::from_db([-4.0, -3.0, 0.0, 3.0, 5.0, 4.0, 2.0, 0.0, -1.5, -3.0])
            }
            EqPreset::SpokenWord => {
                Self::from_db([-4.0, -2.0, 0.0, 2.0, 4.0, 4.0, 2.0, 0.0, -2.0, -4.0])
            }
            EqPreset::Loudness => {
                Self::from_db([5.0, 4.0, 2.0, 0.0, -2.0, -2.0, 0.0, 2.0, 4.0, 5.0])
            }
        }
    }

    pub fn as_array(&self) -> [f32; 10] {
        [
            self.band_31 as f32,
            self.band_62 as f32,
            self.band_125 as f32,
            self.band_250 as f32,
            self.band_500 as f32,
            self.band_1k as f32,
            self.band_2k as f32,
            self.band_4k as f32,
            self.band_8k as f32,
            self.band_16k as f32,
        ]
    }

    fn from_db(values: [f64; 10]) -> Self {
        Self {
            band_31: values[0],
            band_62: values[1],
            band_125: values[2],
            band_250: values[3],
            band_500: values[4],
            band_1k: values[5],
            band_2k: values[6],
            band_4k: values[7],
            band_8k: values[8],
            band_16k: values[9],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Data, Default)]
pub enum Theme {
    #[default]
    Light,
    Dark,
    Custom(String),
}

impl Serialize for Theme {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Theme::Light => serializer.serialize_str("Light"),
            Theme::Dark => serializer.serialize_str("Dark"),
            Theme::Custom(name) => serializer.serialize_str(name),
        }
    }
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "Light" | "light" => Ok(Theme::Light),
            "Dark" | "dark" => Ok(Theme::Dark),
            other => Ok(Theme::Custom(other.to_string())),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Data, Serialize, Deserialize, Default)]
pub enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Data, Serialize, Deserialize, Default)]
pub enum SortCriteria {
    Title,
    Artist,
    Album,
    Duration,
    #[default]
    DateAdded,
}

fn get_dir_size(path: &Path) -> Option<u64> {
    fs::read_dir(path).ok()?.try_fold(0, |acc, entry| {
        let entry = entry.ok()?;
        let size = if entry.file_type().ok()?.is_dir() {
            get_dir_size(&entry.path())?
        } else {
            entry.metadata().ok()?.len()
        };
        Some(acc + size)
    })
}
