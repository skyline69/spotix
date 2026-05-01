#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::new_without_default, clippy::type_complexity)]

mod cmd;
mod controller;
mod data;
mod delegate;
mod error;
#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod tray;
mod ui;
mod webapi;
mod widget;

use druid::AppLauncher;
use env_logger::{Builder, Env};
use webapi::WebApi;

use spotix_core::cache::Cache;

use crate::{
    data::{AppState, Config},
    delegate::Delegate,
};

const ENV_LOG: &str = "SPOTIX_LOG";
const ENV_LOG_STYLE: &str = "SPOTIX_LOG_STYLE";

fn main() {
    // Setup logging from the env variables, with defaults.
    Builder::from_env(
        Env::new()
            .filter_or(ENV_LOG, "info")
            .write_style(ENV_LOG_STYLE),
    )
    .init();

    // Load configuration
    let mut config = Config::load().unwrap_or_default();
    let device_id = config.ensure_device_id();
    unsafe {
        std::env::set_var("SPOTIX_DEVICE_ID", &device_id);
    }
    if config.device_id.as_deref() != Some(&device_id) {
        config.save();
    }

    ui::theme::configure_fontconfig();
    ui::theme::ensure_preset_themes();
    ui::desktop::ensure_desktop_integration();

    let paginated_limit = config.paginated_limit;
    if config.oauth_token_clone().is_some() {
        log::info!("webapi: oauth token loaded from config");
    } else {
        log::warn!("webapi: no oauth token in config (re-auth needed for webapi)");
    }
    let mut state = AppState::default_with_config(config.clone());

    if let Some(cache_dir) = Config::cache_dir() {
        match Cache::new(cache_dir) {
            Ok(cache) => {
                state.preferences.cache = Some(cache);
            }
            Err(err) => {
                log::error!("Failed to create cache: {err}");
            }
        }
    }

    WebApi::new(
        state.session.clone(),
        Config::proxy().as_deref(),
        Config::cache_dir(),
        state.config.oauth_token_clone(),
        paginated_limit,
        config.effective_webapi_client_id().to_string(),
    )
    .install_as_global();
    let delegate;
    let launcher;
    if state.config.has_credentials() {
        // Credentials are configured, open the main window.
        let window = ui::main_window(&state.config);
        delegate = Delegate::with_main(window.id);
        launcher = AppLauncher::with_window(window).configure_env(ui::theme::setup);

        // Load user's local tracks for the WebApi.
        WebApi::global().load_local_tracks(state.config.username().unwrap());
    } else {
        // No configured credentials, open the account setup.
        let window = ui::account_setup_window();
        delegate = Delegate::with_preferences(window.id);
        launcher = AppLauncher::with_window(window).configure_env(ui::theme::setup);
    };

    launcher
        .delegate(delegate)
        .launch(state)
        .expect("Application launch");
}
