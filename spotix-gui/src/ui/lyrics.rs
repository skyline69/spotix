use druid::piet::{Text, TextLayout, TextLayoutBuilder};
use druid::widget::Controller;
use druid::{
    BoxConstraints, Data, Event, EventCtx, LayoutCtx, LensExt, LifeCycle, LifeCycleCtx, PaintCtx,
    Point, RenderContext, Selector, Size, UpdateCtx, Widget, WidgetExt,
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
        || List::new(|| LyricLine::default()),
        || Label::new("No lyrics found for this track").center(),
    )
    .lens(Ctx::make(AppState::common_ctx, AppState::lyrics).then(Ctx::in_promise()))
    .on_command_async(
        SHOW_LYRICS,
        |t| WebApi::global().get_lyrics(t.item.id().to_base62()),
        |_, data, _| data.lyrics.defer(()),
        |_, data, r| {
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
        if let druid::Event::Command(cmd) = event {
            if cmd.is(cmd::PLAYBACK_PROGRESS) {
                ctx.request_paint();
            }
        }
        child.event(ctx, event, data, env);
    }
}

#[derive(Default)]
struct LyricLine {
    hovered: bool,
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
            Event::MouseDown(mouse) if mouse.button.is_left() => {
                if let Ok(ms) = data.data.start_time_ms.parse::<u64>() {
                    if ms != 0 {
                        ctx.submit_command(cmd::SKIP_TO_POSITION.with(ms));
                    }
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
        _data: &WithCtx<TrackLines>,
        _env: &druid::Env,
    ) {
        if let LifeCycle::HotChanged(hot) = event {
            self.hovered = *hot;
            ctx.request_paint();
        }
    }

    fn update(
        &mut self,
        ctx: &mut UpdateCtx,
        old_data: &WithCtx<TrackLines>,
        data: &WithCtx<TrackLines>,
        _env: &druid::Env,
    ) {
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
            .font(
                env.get(theme::UI_FONT).family.clone(),
                env.get(theme::TEXT_SIZE_LARGE),
            )
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
            end = start.saturating_add(2000); // short grace window when missing
        } else {
            end = end.saturating_add(500); // slight linger past end
        }
        let active = progress_ms >= start && progress_ms < end;
        let past = progress_ms >= end;

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
            .font(
                env.get(theme::UI_FONT).family.clone(),
                env.get(theme::TEXT_SIZE_LARGE),
            )
            .default_attribute(druid::piet::TextAttribute::Weight(weight))
            .text_color(text_color)
            .max_width(ctx.size().width - padding.0 * 2.0)
            .alignment(TextAlignment::Start)
            .build()
            .unwrap();
        ctx.draw_text(&layout, Point::new(padding.0, padding.1));
    }
}
