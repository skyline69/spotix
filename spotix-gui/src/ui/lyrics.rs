use std::sync::OnceLock;
use std::time::Duration;

use druid::piet::{Text, TextLayout, TextLayoutBuilder};
use druid::widget::Controller;
use druid::{
    BoxConstraints, Data, Event, EventCtx, LayoutCtx, LensExt, LifeCycle, LifeCycleCtx, PaintCtx,
    Point, RenderContext, Selector, Size, Target, TimerToken, UpdateCtx, Vec2, Widget, WidgetExt,
    WidgetId,
    text::TextAlignment,
    widget::{Container, CrossAxisAlignment, Flex, Label, List, Scroll},
};

use crate::cmd;
use crate::data::{AppState, Ctx, NowPlaying, Playable, TrackLines, WithCtx};
use crate::widget::MyWidgetExt;
use crate::{webapi::WebApi, widget::Async};

use super::theme;
use super::utils;

pub const SHOW_LYRICS: Selector<NowPlaying> = Selector::new("app.home.show_lyrics");
const SCROLL_LYRIC_TO: Selector<f64> = Selector::new("app.lyrics.scroll-to");
pub const SCROLL_ACTIVE_LYRIC: Selector = Selector::new("app.lyrics.scroll-active");
static LYRICS_SCROLL_ID: OnceLock<WidgetId> = OnceLock::new();

pub fn lyrics_widget() -> impl Widget<AppState> {
    Scroll::new(
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
    .with_id(lyrics_scroll_id())
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
        if let Event::Command(cmd) = event
            && cmd.is(SCROLL_LYRIC_TO)
        {
            let line_center = *cmd.get_unchecked(SCROLL_LYRIC_TO);
            let view_center = ctx.window_origin().y + ctx.size().height * 0.5;
            let delta = line_center - view_center;
            if delta.abs() > 1.0 {
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
        let text = data.data.words.as_str();
        let layout = _ctx
            .text()
            .new_text_layout(text.to_string())
            .font(env.get(theme::UI_FONT).family.clone(), lyric_text_size())
            .max_width(bc.max().width)
            .alignment(TextAlignment::Start)
            .build()
            .unwrap();
        let padding = Size::new(theme::grid(1.0), theme::grid(0.75));
        let height = layout.size().height + padding.height * 2.0;
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
        } else if past {
            (env.get(theme::GREY_500), druid::piet::FontWeight::REGULAR)
        } else if self.hovered {
            (env.get(theme::GREY_000), druid::piet::FontWeight::REGULAR)
        } else {
            (env.get(theme::GREY_100), druid::piet::FontWeight::REGULAR)
        };

        let padding = (theme::grid(1.0), theme::grid(0.75));
        let layout = ctx
            .text()
            .new_text_layout(data.data.words.to_string())
            .font(env.get(theme::UI_FONT).family.clone(), lyric_text_size())
            .default_attribute(druid::piet::TextAttribute::Weight(weight))
            .text_color(text_color)
            .max_width(ctx.size().width - padding.0 * 2.0)
            .alignment(TextAlignment::Start)
            .build()
            .unwrap();
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

fn lyric_text_size() -> f64 {
    32.0
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
