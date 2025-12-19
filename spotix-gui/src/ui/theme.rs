use std::fs;

use druid::{Color, Env, FontDescriptor, FontFamily, FontWeight, Insets, Key, Size};
use log::warn;
use serde::Deserialize;

pub use druid::theme::*;

use crate::data::{AppState, Config, Theme};

pub fn grid(m: f64) -> f64 {
    GRID * m
}

pub const GRID: f64 = 8.0;

pub const GREY_000: Key<Color> = Key::new("app.grey_000");
pub const GREY_100: Key<Color> = Key::new("app.grey_100");
pub const GREY_200: Key<Color> = Key::new("app.grey_200");
pub const GREY_300: Key<Color> = Key::new("app.grey_300");
pub const GREY_400: Key<Color> = Key::new("app.grey_400");
pub const GREY_500: Key<Color> = Key::new("app.grey_500");
pub const GREY_600: Key<Color> = Key::new("app.grey_600");
pub const GREY_700: Key<Color> = Key::new("app.grey_700");
pub const BLUE_100: Key<Color> = Key::new("app.blue_100");
pub const BLUE_200: Key<Color> = Key::new("app.blue_200");

pub const RED: Key<Color> = Key::new("app.red");

pub const MENU_BUTTON_BG_ACTIVE: Key<Color> = Key::new("app.menu-bg-active");
pub const MENU_BUTTON_BG_INACTIVE: Key<Color> = Key::new("app.menu-bg-inactive");
pub const MENU_BUTTON_FG_ACTIVE: Key<Color> = Key::new("app.menu-fg-active");
pub const MENU_BUTTON_FG_INACTIVE: Key<Color> = Key::new("app.menu-fg-inactive");
pub const PLAYBACK_TOGGLE_BG_ACTIVE: Key<Color> = Key::new("app.playback-toggle-bg-active");
pub const PLAYBACK_TOGGLE_BG_INACTIVE: Key<Color> = Key::new("app.playback-toggle-bg-inactive");
pub const PLAYBACK_TOGGLE_FG_ACTIVE: Key<Color> = Key::new("app.playback-toggle-fg-active");

pub const UI_FONT_MEDIUM: Key<FontDescriptor> = Key::new("app.ui-font-medium");
pub const UI_FONT_MONO: Key<FontDescriptor> = Key::new("app.ui-font-mono");
pub const TEXT_SIZE_SMALL: Key<f64> = Key::new("app.text-size-small");

pub const ICON_COLOR: Key<Color> = Key::new("app.icon-color");
pub const ICON_SIZE_TINY: Size = Size::new(12.0, 12.0);
pub const ICON_SIZE_SMALL: Size = Size::new(14.0, 14.0);
pub const ICON_SIZE_MEDIUM: Size = Size::new(16.0, 16.0);
pub const ICON_SIZE_LARGE: Size = Size::new(22.0, 22.0);
pub const LYRIC_HIGHLIGHT: Key<Color> = Key::new("app.lyric-highlight");
pub const LYRIC_PAST: Key<Color> = Key::new("app.lyric-past");

pub const LINK_HOT_COLOR: Key<Color> = Key::new("app.link-hot-color");
pub const LINK_ACTIVE_COLOR: Key<Color> = Key::new("app.link-active-color");
pub const LINK_COLD_COLOR: Key<Color> = Key::new("app.link-cold-color");

pub fn setup(env: &mut Env, state: &AppState) {
    let tone = match &state.config.theme {
        Theme::Light => {
            setup_light_theme(env);
            ThemeTone::Light
        }
        Theme::Dark => {
            setup_dark_theme(env);
            ThemeTone::Dark
        }
        Theme::Custom(name) => setup_custom_theme(env, name).unwrap_or_else(|| {
            warn!("Theme '{name}' could not be loaded, falling back to Light.");
            setup_light_theme(env);
            ThemeTone::Light
        }),
    };

    env.set(WINDOW_BACKGROUND_COLOR, env.get(GREY_700));
    env.set(TEXT_COLOR, env.get(GREY_100));
    env.set(ICON_COLOR, env.get(GREY_400));
    env.set(PLACEHOLDER_COLOR, env.get(GREY_300));
    env.set(PRIMARY_LIGHT, env.get(BLUE_100));
    env.set(PRIMARY_DARK, env.get(BLUE_200));

    env.set(BACKGROUND_LIGHT, env.get(GREY_700));
    env.set(BACKGROUND_DARK, env.get(GREY_600));
    env.set(FOREGROUND_LIGHT, env.get(GREY_100));
    env.set(FOREGROUND_DARK, env.get(GREY_000));

    match tone {
        ThemeTone::Light => {
            env.set(BUTTON_LIGHT, env.get(GREY_700));
            env.set(BUTTON_DARK, env.get(GREY_600));
        }
        ThemeTone::Dark => {
            env.set(BUTTON_LIGHT, env.get(GREY_600));
            env.set(BUTTON_DARK, env.get(GREY_700));
        }
    }

    env.set(BORDER_LIGHT, env.get(GREY_400));
    env.set(BORDER_DARK, env.get(GREY_500));

    env.set(SELECTED_TEXT_BACKGROUND_COLOR, env.get(BLUE_200));
    env.set(SELECTION_TEXT_COLOR, env.get(GREY_700));
    env.set(LYRIC_HIGHLIGHT, env.get(BLUE_100));
    env.set(LYRIC_PAST, env.get(GREY_500));

    env.set(CURSOR_COLOR, env.get(GREY_000));

    env.set(PROGRESS_BAR_RADIUS, 4.0);
    env.set(BUTTON_BORDER_RADIUS, 4.0);
    env.set(BUTTON_BORDER_WIDTH, 1.0);

    env.set(
        UI_FONT,
        FontDescriptor::new(FontFamily::SYSTEM_UI).with_size(13.0),
    );
    env.set(
        UI_FONT_MEDIUM,
        FontDescriptor::new(FontFamily::SYSTEM_UI)
            .with_size(13.0)
            .with_weight(FontWeight::MEDIUM),
    );
    env.set(
        UI_FONT_MONO,
        FontDescriptor::new(FontFamily::MONOSPACE).with_size(13.0),
    );
    env.set(TEXT_SIZE_SMALL, 11.0);
    env.set(TEXT_SIZE_NORMAL, 13.0);
    env.set(TEXT_SIZE_LARGE, 16.0);

    env.set(BASIC_WIDGET_HEIGHT, 16.0);
    env.set(WIDE_WIDGET_WIDTH, grid(12.0));
    env.set(BORDERED_WIDGET_HEIGHT, grid(4.0));

    env.set(TEXTBOX_BORDER_RADIUS, 4.0);
    env.set(TEXTBOX_BORDER_WIDTH, 1.0);
    env.set(TEXTBOX_INSETS, Insets::uniform_xy(grid(1.2), grid(1.0)));

    env.set(SCROLLBAR_COLOR, env.get(GREY_300));
    env.set(SCROLLBAR_BORDER_COLOR, env.get(GREY_300));
    env.set(SCROLLBAR_MAX_OPACITY, 0.8);
    env.set(SCROLLBAR_FADE_DELAY, 1500u64);
    env.set(SCROLLBAR_WIDTH, 6.0);
    env.set(SCROLLBAR_PAD, 2.0);
    env.set(SCROLLBAR_RADIUS, 5.0);
    env.set(SCROLLBAR_EDGE_WIDTH, 1.0);

    env.set(WIDGET_PADDING_VERTICAL, grid(0.5));
    env.set(WIDGET_PADDING_HORIZONTAL, grid(1.0));
    env.set(WIDGET_CONTROL_COMPONENT_PADDING, grid(1.0));

    env.set(MENU_BUTTON_BG_ACTIVE, env.get(GREY_500));
    env.set(MENU_BUTTON_BG_INACTIVE, env.get(GREY_600));
    env.set(MENU_BUTTON_FG_ACTIVE, env.get(GREY_000));
    env.set(MENU_BUTTON_FG_INACTIVE, env.get(GREY_100));
    env.set(PLAYBACK_TOGGLE_BG_ACTIVE, env.get(LINK_ACTIVE_COLOR));
    env.set(PLAYBACK_TOGGLE_BG_INACTIVE, env.get(LINK_COLD_COLOR));
    env.set(PLAYBACK_TOGGLE_FG_ACTIVE, env.get(BLUE_100));
}

#[derive(Copy, Clone, Debug)]
enum ThemeTone {
    Light,
    Dark,
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    name: Option<String>,
    base: Option<String>,
    colors: Option<ThemeColors>,
}

#[derive(Debug, Deserialize)]
struct ThemeColors {
    grey_000: Option<String>,
    grey_100: Option<String>,
    grey_200: Option<String>,
    grey_300: Option<String>,
    grey_400: Option<String>,
    grey_500: Option<String>,
    grey_600: Option<String>,
    grey_700: Option<String>,
    blue_100: Option<String>,
    blue_200: Option<String>,
    red: Option<String>,
    link_hot: Option<String>,
    link_active: Option<String>,
    link_cold: Option<String>,
    lyric_highlight: Option<String>,
    lyric_past: Option<String>,
    playback_toggle_bg_active: Option<String>,
    playback_toggle_bg_inactive: Option<String>,
    playback_toggle_fg_active: Option<String>,
}

fn setup_custom_theme(env: &mut Env, name: &str) -> Option<ThemeTone> {
    let themes_dir = Config::themes_dir()?;
    let theme = load_theme_by_name(&themes_dir, name)?;

    let tone = parse_theme_tone(theme.base.as_deref());
    match tone {
        ThemeTone::Light => setup_light_theme(env),
        ThemeTone::Dark => setup_dark_theme(env),
    }

    if let Some(colors) = theme.colors.as_ref() {
        apply_theme_colors(env, colors);
    }

    Some(tone)
}

fn load_theme_by_name(dir: &std::path::Path, name: &str) -> Option<ThemeFile> {
    let entries = fs::read_dir(dir)
        .map_err(|err| {
            warn!("Failed to read themes directory {:?}: {}", dir, err);
        })
        .ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        let is_toml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("toml"))
            .unwrap_or(false);
        if !is_toml {
            continue;
        }

        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) => {
                warn!("Failed to read theme file {:?}: {}", path, err);
                continue;
            }
        };
        let theme: ThemeFile = match toml::from_str(&contents) {
            Ok(theme) => theme,
            Err(err) => {
                warn!("Failed to parse theme file {:?}: {}", path, err);
                continue;
            }
        };

        let file_name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let matches = theme
            .name
            .as_deref()
            .map(|value| value.eq_ignore_ascii_case(name))
            .unwrap_or(false)
            || file_name.eq_ignore_ascii_case(name);

        if matches {
            return Some(theme);
        }
    }

    None
}

fn parse_theme_tone(base: Option<&str>) -> ThemeTone {
    match base {
        Some(value) if value.eq_ignore_ascii_case("dark") => ThemeTone::Dark,
        Some(value) if value.eq_ignore_ascii_case("light") => ThemeTone::Light,
        Some(value) => {
            warn!("Unknown theme base '{value}', defaulting to Light.");
            ThemeTone::Light
        }
        None => ThemeTone::Light,
    }
}

fn apply_theme_colors(env: &mut Env, colors: &ThemeColors) {
    set_color(env, GREY_000, &colors.grey_000, "grey_000");
    set_color(env, GREY_100, &colors.grey_100, "grey_100");
    set_color(env, GREY_200, &colors.grey_200, "grey_200");
    set_color(env, GREY_300, &colors.grey_300, "grey_300");
    set_color(env, GREY_400, &colors.grey_400, "grey_400");
    set_color(env, GREY_500, &colors.grey_500, "grey_500");
    set_color(env, GREY_600, &colors.grey_600, "grey_600");
    set_color(env, GREY_700, &colors.grey_700, "grey_700");
    set_color(env, BLUE_100, &colors.blue_100, "blue_100");
    set_color(env, BLUE_200, &colors.blue_200, "blue_200");
    set_color(env, RED, &colors.red, "red");
    set_color(env, LINK_HOT_COLOR, &colors.link_hot, "link_hot");
    set_color(env, LINK_ACTIVE_COLOR, &colors.link_active, "link_active");
    set_color(env, LINK_COLD_COLOR, &colors.link_cold, "link_cold");
    set_color(
        env,
        LYRIC_HIGHLIGHT,
        &colors.lyric_highlight,
        "lyric_highlight",
    );
    set_color(env, LYRIC_PAST, &colors.lyric_past, "lyric_past");
    set_color(
        env,
        PLAYBACK_TOGGLE_BG_ACTIVE,
        &colors.playback_toggle_bg_active,
        "playback_toggle_bg_active",
    );
    set_color(
        env,
        PLAYBACK_TOGGLE_BG_INACTIVE,
        &colors.playback_toggle_bg_inactive,
        "playback_toggle_bg_inactive",
    );
    set_color(
        env,
        PLAYBACK_TOGGLE_FG_ACTIVE,
        &colors.playback_toggle_fg_active,
        "playback_toggle_fg_active",
    );
}

fn set_color(env: &mut Env, key: Key<Color>, value: &Option<String>, label: &str) {
    if let Some(raw) = value {
        match parse_color(raw) {
            Some(color) => env.set(key, color),
            None => warn!("Invalid color value for {}: '{}'", label, raw),
        }
    }
}

fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    let hex = value.strip_prefix('#').unwrap_or(value);

    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::rgb8(r, g, b))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Color::rgba8(r, g, b, a))
        }
        _ => None,
    }
}

fn setup_light_theme(env: &mut Env) {
    env.set(GREY_000, Color::grey8(0x00));
    env.set(GREY_100, Color::grey8(0x33));
    env.set(GREY_200, Color::grey8(0x4f));
    env.set(GREY_300, Color::grey8(0x82));
    env.set(GREY_400, Color::grey8(0xbd));
    env.set(GREY_500, Color::from_rgba32_u32(0xe5e6e7ff));
    env.set(GREY_600, Color::from_rgba32_u32(0xf5f6f7ff));
    env.set(GREY_700, Color::from_rgba32_u32(0xffffffff));
    env.set(BLUE_100, Color::rgb8(0x5c, 0xc4, 0xff));
    env.set(BLUE_200, Color::rgb8(0x00, 0x8d, 0xdd));

    env.set(RED, Color::rgba8(0xEB, 0x57, 0x57, 0xFF));

    env.set(LINK_HOT_COLOR, Color::rgba(0.0, 0.0, 0.0, 0.06));
    env.set(LINK_ACTIVE_COLOR, Color::rgba(0.0, 0.0, 0.0, 0.04));
    env.set(LINK_COLD_COLOR, Color::rgba(0.0, 0.0, 0.0, 0.0));
}

fn setup_dark_theme(env: &mut Env) {
    env.set(GREY_000, Color::grey8(0xff));
    env.set(GREY_100, Color::grey8(0xf2));
    env.set(GREY_200, Color::grey8(0xe0));
    env.set(GREY_300, Color::grey8(0xbd));
    env.set(GREY_400, Color::grey8(0x82));
    env.set(GREY_500, Color::grey8(0x4f));
    env.set(GREY_600, Color::grey8(0x33));
    env.set(GREY_700, Color::grey8(0x28));
    env.set(BLUE_100, Color::rgb8(0x00, 0x8d, 0xdd));
    env.set(BLUE_200, Color::rgb8(0x5c, 0xc4, 0xff));

    env.set(RED, Color::rgba8(0xEB, 0x57, 0x57, 0xFF));

    env.set(LINK_HOT_COLOR, Color::rgba(1.0, 1.0, 1.0, 0.05));
    env.set(LINK_ACTIVE_COLOR, Color::rgba(1.0, 1.0, 1.0, 0.025));
    env.set(LINK_COLD_COLOR, Color::rgba(1.0, 1.0, 1.0, 0.0));
}
