use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use druid::piet::{Text, TextLayout, TextLayoutBuilder};
use druid::widget::Controller;
use druid::{
    BoxConstraints, Cursor, Data, Event, EventCtx, LayoutCtx, LensExt, LifeCycle, LifeCycleCtx,
    PaintCtx, Point, RenderContext, Selector, Size, Target, TimerToken, UpdateCtx, Vec2, Widget,
    WidgetExt, WidgetId,
    piet::{LinearGradient, UnitPoint},
    text::TextAlignment,
    widget::{Container, CrossAxisAlignment, Flex, Label, List, Painter, Scroll},
};

use crate::cmd;
use crate::data::config::LyricsAppearance;
use crate::data::{AppState, Ctx, NowPlaying, Playable, TrackLines, WithCtx};
use crate::widget::MyWidgetExt;
use crate::{webapi::WebApi, widget::Async};

use super::palette::{self, AlbumPalette};
use super::theme;
use super::utils;

pub const SHOW_LYRICS: Selector<NowPlaying> = Selector::new("app.home.show_lyrics");
const SCROLL_LYRIC_TO: Selector<f64> = Selector::new("app.lyrics.scroll-to");
pub const SCROLL_ACTIVE_LYRIC: Selector = Selector::new("app.lyrics.scroll-active");
static LYRICS_SCROLL_ID: OnceLock<WidgetId> = OnceLock::new();

/// Shared palette cache: (track_image_url, extracted_palette).
/// Avoids re-running k-means on every repaint.
static PALETTE_CACHE: OnceLock<Mutex<Option<(Arc<str>, AlbumPalette)>>> = OnceLock::new();

fn cached_palette(data: &AppState) -> AlbumPalette {
    let cache = PALETTE_CACHE.get_or_init(|| Mutex::new(None));

    // Use the same small cover size as the playback bar so we hit the
    // LRU image cache. If that's not cached, try any available size.
    let np = data.playback.now_playing.as_ref();
    let image_url: Option<Arc<str>> = np
        .and_then(|np| {
            // Try the playback bar size first (most likely cached)
            np.cover_image_url(64.0, 64.0)
                .or_else(|| np.cover_image_url(32.0, 32.0))
                .or_else(|| np.cover_image_url(300.0, 300.0))
        })
        .map(Arc::from);

    let Some(url) = image_url else {
        return AlbumPalette::default();
    };

    let mut guard = cache.lock().unwrap();
    if let Some((ref cached_url, ref palette)) = *guard
        && *cached_url == url
    {
        return palette.clone();
    }

    // Try to get the image from the in-memory cache first
    let image_buf = WebApi::global().get_cached_image(&url).or_else(|| {
        log::debug!("lyrics palette: image not in LRU cache, fetching: {url}");
        // Not cached -- fetch it synchronously (fast, typically <100ms)
        match WebApi::global().get_image(url.clone()) {
            Ok(buf) => {
                log::debug!(
                    "lyrics palette: fetched image {}x{}",
                    buf.size().width,
                    buf.size().height
                );
                Some(buf)
            }
            Err(e) => {
                log::warn!("lyrics palette: failed to fetch image: {e}");
                None
            }
        }
    });

    if let Some(image_buf) = image_buf {
        let palette = palette::extract_palette(&image_buf);
        log::info!(
            "lyrics palette: extracted from artwork - dominant=({:.0},{:.0},{:.0}), highlight=({:.0},{:.0},{:.0})",
            palette.dominant.as_rgba().0 * 255.0,
            palette.dominant.as_rgba().1 * 255.0,
            palette.dominant.as_rgba().2 * 255.0,
            palette.highlight.as_rgba().0 * 255.0,
            palette.highlight.as_rgba().1 * 255.0,
            palette.highlight.as_rgba().2 * 255.0,
        );
        *guard = Some((url, palette.clone()));
        palette
    } else {
        log::warn!("lyrics palette: no image available, using default");
        AlbumPalette::default()
    }
}

pub fn lyrics_widget() -> impl Widget<AppState> {
    let inner = Scroll::new(
        Container::new(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .with_default_spacer()
                .with_child(track_info_widget())
                .with_spacer(theme::grid(2.0))
                .with_child(track_lyrics_widget()),
        )
        .padding((theme::grid(2.0), 0.0)),
    )
    .vertical()
    .controller(LyricsScrollController::default())
    .with_id(lyrics_scroll_id());

    // Wrap with dynamic Spotify-styled background when enabled
    let bg = Painter::new(|ctx, data: &AppState, _env| {
        if data.config.lyrics_appearance != LyricsAppearance::SpotifyStyled {
            return;
        }
        let palette = cached_palette(data);
        let rect = ctx.size().to_rect();
        let gradient = LinearGradient::new(
            UnitPoint::TOP,
            UnitPoint::BOTTOM,
            (palette.dominant, palette.secondary),
        );
        ctx.fill(rect, &gradient);
    });

    inner.background(bg).env_scope(|env, data: &AppState| {
        if data.config.lyrics_appearance != LyricsAppearance::SpotifyStyled {
            return;
        }
        let palette = cached_palette(data);
        env.set(theme::LYRIC_HIGHLIGHT, palette.highlight);
        env.set(theme::LYRIC_HOVER, palette.text);
        env.set(theme::GREY_500, palette.past);
        env.set(theme::GREY_100, palette.text);
        // Override text colors for track info
        env.set(theme::PLACEHOLDER_COLOR, palette.past);
    })
}

fn track_info_widget() -> impl Widget<AppState> {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(
            Label::dynamic(|data: &AppState, _| {
                data.playback.now_playing.as_ref().map_or_else(
                    || "No track playing".to_string(),
                    |np| match &np.item {
                        Playable::Track(track) => track.name.clone().to_string(),
                        _ => "Unknown track".to_string(),
                    },
                )
            })
            .with_font(theme::UI_FONT_MEDIUM)
            .with_text_size(theme::TEXT_SIZE_LARGE),
        )
        .with_spacer(theme::grid(0.5))
        .with_child(
            Label::dynamic(|data: &AppState, _| {
                data.playback.now_playing.as_ref().map_or_else(
                    || "".to_string(),
                    |np| match &np.item {
                        Playable::Track(track) => {
                            format!("{} - {}", track.artist_name(), track.album_name())
                        }
                        _ => "".to_string(),
                    },
                )
            })
            .with_text_size(theme::TEXT_SIZE_SMALL)
            .with_text_color(theme::PLACEHOLDER_COLOR),
        )
}

fn track_lyrics_widget() -> impl Widget<AppState> {
    Async::new(
        utils::spinner_widget,
        || List::new(LyricLine::default),
        || Label::new("No lyrics found for this track").center(),
    )
    .lens(Ctx::make(AppState::common_ctx, AppState::lyrics).then(Ctx::in_promise()))
    .on_command_async(
        SHOW_LYRICS,
        |t| WebApi::global().get_lyrics(t.item.id().to_base62()),
        |_, data, _| data.lyrics.defer(()),
        |ctx, data, r| {
            let processed = r.1.map(|mut lines| {
                for i in 0..lines.len() {
                    let next_start = lines
                        .get(i + 1)
                        .and_then(|l| l.start_time_ms.parse::<u64>().ok());
                    if let Some(ns) = next_start {
                        lines[i].next_start_ms = Some(ns);
                    }
                }
                lines
            });
            data.lyrics.update(((), processed));
            ctx.submit_command(SCROLL_ACTIVE_LYRIC.to(Target::Window(ctx.window_id())));
        },
    )
    .controller(LyricsProgressController)
}

struct LyricsProgressController;

impl<W: Widget<AppState>> Controller<AppState, W> for LyricsProgressController {
    fn event(
        &mut self,
        child: &mut W,
        ctx: &mut druid::EventCtx,
        event: &druid::Event,
        data: &mut AppState,
        env: &druid::Env,
    ) {
        if let druid::Event::Command(cmd) = event
            && cmd.is(cmd::PLAYBACK_PROGRESS)
        {
            ctx.request_paint();
        }
        child.event(ctx, event, data, env);
    }
}

#[derive(Default)]
struct LyricsScrollController {
    scroll_timer: Option<TimerToken>,
    scroll_retries: u8,
    last_user_scroll: Option<Instant>,
}

impl<W: Widget<AppState>> Controller<AppState, Scroll<AppState, W>> for LyricsScrollController {
    fn lifecycle(
        &mut self,
        child: &mut Scroll<AppState, W>,
        ctx: &mut LifeCycleCtx,
        event: &LifeCycle,
        data: &AppState,
        env: &druid::Env,
    ) {
        if matches!(event, LifeCycle::WidgetAdded) {
            self.scroll_retries = 3;
            self.scroll_timer = Some(ctx.request_timer(Duration::from_millis(30)));
        }
        child.lifecycle(ctx, event, data, env);
    }

    fn event(
        &mut self,
        child: &mut Scroll<AppState, W>,
        ctx: &mut EventCtx,
        event: &Event,
        data: &mut AppState,
        env: &druid::Env,
    ) {
        if let Event::Timer(token) = event
            && self.scroll_timer == Some(*token)
        {
            self.scroll_timer = None;
            if self.scroll_retries > 0 {
                self.scroll_retries -= 1;
                ctx.submit_command(SCROLL_ACTIVE_LYRIC.to(Target::Window(ctx.window_id())));
                if self.scroll_retries > 0 {
                    self.scroll_timer = Some(ctx.request_timer(Duration::from_millis(60)));
                }
            }
        }
        if let Event::Wheel(_) = event {
            self.last_user_scroll = Some(Instant::now());
        }
        if let Event::Command(cmd) = event
            && cmd.is(SCROLL_LYRIC_TO)
        {
            let line_center = *cmd.get_unchecked(SCROLL_LYRIC_TO);
            let view_center = ctx.window_origin().y + ctx.size().height * 0.5;
            let delta = line_center - view_center;
            let recent_manual = self
                .last_user_scroll
                .map(|t| t.elapsed() < Duration::from_secs(2))
                .unwrap_or(false);
            let near_center = delta.abs() < ctx.size().height * 0.35;
            if delta.abs() > 1.0 && (!recent_manual || near_center) {
                child.scroll_by(ctx, Vec2::new(0.0, delta));
            }
            ctx.set_handled();
        }
        child.event(ctx, event, data, env);
    }

    fn update(
        &mut self,
        child: &mut Scroll<AppState, W>,
        ctx: &mut UpdateCtx,
        old_data: &AppState,
        data: &AppState,
        env: &druid::Env,
    ) {
        if !old_data.lyrics.is_resolved() && data.lyrics.is_resolved() {
            ctx.submit_command(SCROLL_ACTIVE_LYRIC.to(Target::Window(ctx.window_id())));
        }
        child.update(ctx, old_data, data, env);
    }
}

#[derive(Default)]
struct LyricLine {
    hovered: bool,
    was_active: bool,
    scrolled_for_active: bool,
    scroll_timer: Option<TimerToken>,
}

impl Widget<WithCtx<TrackLines>> for LyricLine {
    fn event(
        &mut self,
        ctx: &mut EventCtx,
        event: &Event,
        data: &mut WithCtx<TrackLines>,
        _env: &druid::Env,
    ) {
        match event {
            Event::Command(cmd) if cmd.is(cmd::PLAYBACK_PROGRESS) => {
                self.maybe_schedule_scroll(ctx, data);
            }
            Event::Command(cmd) if cmd.is(SCROLL_ACTIVE_LYRIC) => {
                let progress_ms = data.ctx.now_playing_progress.as_millis() as u64;
                if should_scroll_line(&data.data, progress_ms) {
                    let line_center = ctx.window_origin().y + ctx.size().height * 0.5;
                    ctx.submit_command(SCROLL_LYRIC_TO.with(line_center).to(Target::Global));
                    self.scrolled_for_active = true;
                }
            }
            Event::Timer(token) if self.scroll_timer == Some(*token) => {
                self.scroll_timer = None;
                let progress_ms = data.ctx.now_playing_progress.as_millis() as u64;
                if should_scroll_line(&data.data, progress_ms) && !self.scrolled_for_active {
                    submit_scroll(ctx);
                    self.scrolled_for_active = true;
                }
            }
            Event::MouseDown(mouse) if mouse.button.is_left() => {
                if let Ok(ms) = data.data.start_time_ms.parse::<u64>()
                    && ms != 0
                {
                    ctx.submit_command(cmd::SKIP_TO_POSITION.with(ms));
                }
                ctx.set_handled();
            }
            Event::MouseMove(_) => {
                if ctx.is_hot() {
                    ctx.set_cursor(&Cursor::Pointer);
                } else {
                    ctx.clear_cursor();
                }
            }
            _ => {}
        }
    }

    fn lifecycle(
        &mut self,
        ctx: &mut LifeCycleCtx,
        event: &LifeCycle,
        data: &WithCtx<TrackLines>,
        _env: &druid::Env,
    ) {
        match event {
            LifeCycle::HotChanged(hot) => {
                self.hovered = *hot;
                ctx.request_paint();
            }
            LifeCycle::WidgetAdded => {
                self.was_active = lyric_state(data).0;
                self.maybe_schedule_scroll(ctx, data);
            }
            _ => {}
        }
    }

    fn update(
        &mut self,
        ctx: &mut UpdateCtx,
        old_data: &WithCtx<TrackLines>,
        data: &WithCtx<TrackLines>,
        _env: &druid::Env,
    ) {
        self.maybe_schedule_scroll(ctx, data);
        if !old_data.data.same(&data.data)
            || old_data.ctx.now_playing_progress != data.ctx.now_playing_progress
        {
            // If the active/bold state changed, we need a full re-layout
            // because bold text can wrap differently and change the row height.
            let (old_active, _) = lyric_state(old_data);
            let (new_active, _) = lyric_state(data);
            if old_active != new_active {
                ctx.request_layout();
            }
            ctx.request_paint();
        }
    }

    fn layout(
        &mut self,
        _ctx: &mut LayoutCtx,
        bc: &BoxConstraints,
        data: &WithCtx<TrackLines>,
        env: &druid::Env,
    ) -> Size {
        // Use the same weight as paint() so the measured height matches
        // what will actually be rendered. Without this, bold active lines
        // can reflow to more lines than measured, overlapping the next lyric.
        let (active, _past) = lyric_state(data);
        let weight = if active {
            druid::piet::FontWeight::BOLD
        } else {
            druid::piet::FontWeight::REGULAR
        };

        let text = data.data.words.as_str();
        let padding_x = theme::grid(1.0);
        let max_width = (bc.max().width - padding_x * 2.0).max(0.0);
        let font_size = lyric_text_size_for_width(bc.max().width);
        let layout = _ctx
            .text()
            .new_text_layout(text.to_string())
            .font(env.get(theme::UI_FONT).family.clone(), font_size)
            .default_attribute(druid::piet::TextAttribute::Weight(weight))
            .max_width(max_width)
            .alignment(TextAlignment::Start)
            .build()
            .unwrap();
        let lines = lyric_line_count(layout.size().height, font_size);
        let padding_y = lyric_padding_y(lines);
        let height = layout.size().height + padding_y * 2.0;
        let width = bc.max().width;
        Size::new(width, height)
    }

    fn paint(&mut self, ctx: &mut PaintCtx, data: &WithCtx<TrackLines>, env: &druid::Env) {
        let (active, past) = lyric_state(data);

        let (text_color, weight) = if active {
            (
                env.get(theme::LYRIC_HIGHLIGHT),
                druid::piet::FontWeight::BOLD,
            )
        } else if self.hovered {
            (
                env.get(theme::LYRIC_HOVER),
                druid::piet::FontWeight::REGULAR,
            )
        } else if past {
            (env.get(theme::GREY_500), druid::piet::FontWeight::REGULAR)
        } else {
            (env.get(theme::GREY_100), druid::piet::FontWeight::REGULAR)
        };

        let padding_x = theme::grid(1.0);
        let font_size = lyric_text_size_for_width(ctx.size().width);
        let layout = ctx
            .text()
            .new_text_layout(data.data.words.to_string())
            .font(env.get(theme::UI_FONT).family.clone(), font_size)
            .default_attribute(druid::piet::TextAttribute::Weight(weight))
            .text_color(text_color)
            .max_width((ctx.size().width - padding_x * 2.0).max(0.0))
            .alignment(TextAlignment::Start)
            .build()
            .unwrap();
        let lines = lyric_line_count(layout.size().height, font_size);
        let padding = (theme::grid(1.0), lyric_padding_y(lines));
        ctx.draw_text(&layout, Point::new(padding.0, padding.1));
    }
}

impl LyricLine {
    fn maybe_schedule_scroll<C: LyricScrollCtx>(
        &mut self,
        ctx: &mut C,
        data: &WithCtx<TrackLines>,
    ) {
        let progress_ms = data.ctx.now_playing_progress.as_millis() as u64;
        let active = should_scroll_line(&data.data, progress_ms);
        if !active {
            self.scrolled_for_active = false;
            self.was_active = false;
            return;
        }

        self.was_active = active;
        if self.scrolled_for_active || self.scroll_timer.is_some() {
            return;
        }

        let token = ctx.request_scroll_timer();
        self.scroll_timer = Some(token);
    }
}

fn submit_scroll<C: LyricScrollCtx>(ctx: &mut C) {
    let line_center = ctx.line_center();
    ctx.submit_scroll_to_line(line_center);
}

trait LyricScrollCtx {
    fn request_scroll_timer(&mut self) -> TimerToken;
    fn submit_scroll_to_line(&mut self, line_center: f64);
    fn line_center(&self) -> f64;
}

impl LyricScrollCtx for EventCtx<'_, '_> {
    fn request_scroll_timer(&mut self) -> TimerToken {
        self.request_timer(Duration::from_millis(1))
    }

    fn submit_scroll_to_line(&mut self, line_center: f64) {
        self.submit_command(SCROLL_LYRIC_TO.with(line_center).to(Target::Global));
    }

    fn line_center(&self) -> f64 {
        self.window_origin().y + self.size().height * 0.5
    }
}

impl LyricScrollCtx for LifeCycleCtx<'_, '_> {
    fn request_scroll_timer(&mut self) -> TimerToken {
        self.request_timer(Duration::from_millis(1))
    }

    fn submit_scroll_to_line(&mut self, line_center: f64) {
        self.submit_command(SCROLL_LYRIC_TO.with(line_center).to(Target::Global));
    }

    fn line_center(&self) -> f64 {
        self.window_origin().y + self.size().height * 0.5
    }
}

impl LyricScrollCtx for UpdateCtx<'_, '_> {
    fn request_scroll_timer(&mut self) -> TimerToken {
        self.request_timer(Duration::from_millis(1))
    }

    fn submit_scroll_to_line(&mut self, line_center: f64) {
        self.submit_command(SCROLL_LYRIC_TO.with(line_center).to(Target::Global));
    }

    fn line_center(&self) -> f64 {
        self.window_origin().y + self.size().height * 0.5
    }
}

fn lyric_text_size_for_width(width: f64) -> f64 {
    if width < theme::grid(48.0) {
        20.0
    } else if width < theme::grid(60.0) {
        22.0
    } else if width < theme::grid(72.0) {
        24.0
    } else {
        26.0
    }
}

fn lyric_line_count(layout_height: f64, font_size: f64) -> usize {
    let line_height = font_size * 1.1;
    let lines = (layout_height / line_height).ceil().max(1.0);
    lines as usize
}

fn lyric_padding_y(lines: usize) -> f64 {
    let base = theme::grid(0.5);
    if lines > 1 {
        base + theme::grid(0.8) + theme::grid(0.35) * (lines.saturating_sub(2) as f64)
    } else {
        base
    }
}

fn lyric_state(data: &WithCtx<TrackLines>) -> (bool, bool) {
    let progress_ms = data
        .ctx
        .now_playing_progress
        .as_millis()
        .saturating_add(400) as u64;
    let start = data.data.start_time_ms.parse::<u64>().unwrap_or(0);
    let mut end = data.data.next_start_ms.unwrap_or_else(|| {
        data.data
            .end_time_ms
            .parse::<u64>()
            .unwrap_or(start)
            .saturating_add(1500)
    });
    if end <= start {
        end = start.saturating_add(2000);
    } else {
        end = end.saturating_add(500);
    }
    let active = progress_ms >= start && progress_ms < end;
    let past = progress_ms >= end;
    (active, past)
}

fn should_scroll_line(line: &TrackLines, progress_ms: u64) -> bool {
    if line.words.trim().is_empty() {
        return false;
    }
    line_is_active(line, progress_ms)
}

fn line_is_active(line: &TrackLines, progress_ms: u64) -> bool {
    let start = line.start_time_ms.parse::<u64>().unwrap_or(0);
    let mut end = line.next_start_ms.unwrap_or_else(|| {
        line.end_time_ms
            .parse::<u64>()
            .unwrap_or(start)
            .saturating_add(1500)
    });
    if end <= start {
        end = start.saturating_add(2000);
    } else {
        end = end.saturating_add(500);
    }
    progress_ms >= start && progress_ms < end
}

fn lyrics_scroll_id() -> WidgetId {
    *LYRICS_SCROLL_ID.get_or_init(WidgetId::next)
}
