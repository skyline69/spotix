use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use druid::{
    BoxConstraints, Cursor, Data, Env, Event, EventCtx, LayoutCtx, LensExt, LifeCycle,
    LifeCycleCtx, Menu, MenuItem, MouseButton, PaintCtx, Point, Rect, RenderContext, Size, Target,
    UpdateCtx, Widget, WidgetExt, WidgetPod,
    im::Vector,
    kurbo::{Affine, BezPath, Circle, Line},
    lens::Map,
    widget::{
        Align, Controller, CrossAxisAlignment, Either, Flex, Label, LineBreaking, List, Painter,
        Scroll, SizedBox, Spinner, ViewSwitcher,
    },
};
use itertools::Itertools;
use spotix_core::item_id::ItemId;

use crate::{
    cmd::{
        self, ADD_TO_QUEUE, CLEAR_QUEUE, QUEUE_DRAG_BEGIN, QUEUE_DRAG_END, QUEUE_DRAG_OVER,
        REMOVE_FROM_QUEUE, SHOW_ARTWORK, TOGGLE_LYRICS, TOGGLE_QUEUE_PANEL,
    },
    controller::PlaybackController,
    data::{
        AppState, AudioAnalysis, Library, Nav, NowPlaying, Playable, Playback, PlaybackOrigin,
        PlaybackPanelTab, PlaybackState, QueueBehavior, QueueDragState, QueueEntry,
    },
    widget::{
        Empty, Maybe, MyWidgetExt, RemoteImage,
        icons::{self, SvgIcon},
    },
};

use super::{episode, library, theme, track, utils};

pub fn panel_widget() -> impl Widget<AppState> {
    let seek_bar =
        Maybe::or_empty(SeekBar::new).lens(AppState::playback.then(Playback::now_playing));
    let item_info =
        Maybe::or_empty(playing_item_widget).lens(AppState::playback.then(Playback::now_playing));
    let controls = player_widget();
    Flex::column()
        .with_child(seek_bar)
        .with_child(BarLayout::new(item_info, controls))
        .controller(PlaybackController::new())
        .on_command(ADD_TO_QUEUE, |_, _, data| {
            data.info_alert("Track added to queue.")
        })
}

fn playing_item_widget() -> impl Widget<NowPlaying> {
    let cover_art = cover_widget(theme::grid(8.0));

    let name = Label::dynamic(|item: &Playable, _| item.name().to_string())
        .with_line_break_mode(LineBreaking::Clip)
        .with_font(theme::UI_FONT_MEDIUM)
        .lens(NowPlaying::item);

    let detail = Label::dynamic(|item: &Playable, _| match item {
        Playable::Track(track) => track.artist_name().to_string(),
        Playable::Episode(episode) => episode.show.name.as_ref().to_string(),
    })
    .with_line_break_mode(LineBreaking::Clip)
    .with_text_size(theme::TEXT_SIZE_SMALL)
    .lens(NowPlaying::item);

    let origin = ViewSwitcher::new(
        |origin: &PlaybackOrigin, _| origin.clone(),
        |origin, _, _| {
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .with_flex_child(
                    Label::dynamic(|origin: &PlaybackOrigin, _| origin.to_string())
                        .with_line_break_mode(LineBreaking::Clip)
                        .with_text_size(theme::TEXT_SIZE_SMALL),
                    1.0,
                )
                .with_spacer(theme::grid(0.25))
                .with_child(
                    playback_origin_icon(origin)
                        .scale(theme::ICON_SIZE_SMALL)
                        .with_color(theme::MEDIA_CONTROL_ICON),
                )
                .boxed()
        },
    )
    .lens(NowPlaying::origin);

    Flex::row()
        .with_child(cover_art)
        .with_flex_child(
            Flex::row().with_spacer(theme::grid(2.0)).with_flex_child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_child(name)
                    .with_spacer(2.0)
                    .with_child(detail)
                    .with_spacer(2.0)
                    .with_child(origin)
                    .on_click(|ctx, now_playing, _| {
                        ctx.submit_command(cmd::NAVIGATE.with(now_playing.origin.to_nav()));
                    })
                    .context_menu(|now_playing| match &now_playing.item {
                        Playable::Track(track) => track::track_menu(
                            track,
                            &now_playing.library,
                            &now_playing.origin,
                            usize::MAX,
                        ),
                        Playable::Episode(episode) => {
                            episode::episode_menu(episode, &now_playing.library)
                        }
                    }),
                1.0,
            ),
            1.0,
        )
        .with_child(ViewSwitcher::new(
            |now_playing: &NowPlaying, _| {
                now_playing.item.track().is_some() && now_playing.library.saved_tracks.is_resolved()
            },
            |selector, _data, _env| match selector {
                true => {
                    // View is only show if now_playing's track isn't none
                    ViewSwitcher::new(
                        |now_playing: &NowPlaying, _| {
                            now_playing
                                .library
                                .contains_track(now_playing.item.track().unwrap())
                        },
                        |selector: &bool, _, _| {
                            match selector {
                                true => &icons::CIRCLE_CHECK,
                                false => &icons::CIRCLE_PLUS,
                            }
                            .scale(theme::ICON_SIZE_SMALL)
                            .with_color(theme::MEDIA_CONTROL_ICON)
                            .boxed()
                        },
                    )
                    .on_left_click(|ctx, _, now_playing, _| {
                        let track = now_playing.item.track().unwrap();
                        if now_playing.library.contains_track(track) {
                            ctx.submit_command(library::UNSAVE_TRACK.with(track.id))
                        } else {
                            ctx.submit_command(library::SAVE_TRACK.with(track.clone()))
                        }
                    })
                    .padding(theme::grid(1.0))
                    .boxed()
                }
                false => Box::new(Flex::column()),
            },
        ))
        .padding(theme::grid(1.0))
        .link()
}

fn cover_widget(size: f64) -> impl Widget<NowPlaying> {
    RemoteImage::new(utils::placeholder_widget(), move |np: &NowPlaying, _| {
        np.cover_image_url(size, size).map(|url| url.into())
    })
    .fix_size(size, size)
    .clip(Size::new(size, size).to_rounded_rect(4.0))
    .on_left_click(|ctx, _, np, _| {
        if let Some(track) = np.item.track()
            && let Some(album) = &track.album
        {
            ctx.submit_command(cmd::NAVIGATE.with(Nav::AlbumDetail(album.clone(), None)));
            return;
        }
        // Fallback: keep existing behavior if we don't have an album link.
        ctx.submit_command(SHOW_ARTWORK);
    })
}

fn playback_origin_icon(origin: &PlaybackOrigin) -> &'static SvgIcon {
    match origin {
        PlaybackOrigin::Home => &icons::HOME,
        PlaybackOrigin::Library => &icons::HEART,
        PlaybackOrigin::Album { .. } => &icons::ALBUM,
        PlaybackOrigin::Artist { .. } => &icons::ARTIST,
        PlaybackOrigin::Playlist { .. } => &icons::PLAYLIST,
        PlaybackOrigin::Show { .. } => &icons::PODCAST,
        PlaybackOrigin::Search { .. } => &icons::SEARCH,
        PlaybackOrigin::Recommendations { .. } => &icons::SEARCH,
    }
}

fn player_widget() -> impl Widget<AppState> {
    Flex::row()
        .with_child(
            small_button_widget(&icons::SKIP_BACK).on_left_click(|ctx, _, _, _| {
                ctx.submit_command(cmd::PLAY_PREVIOUS);
            }),
        )
        .with_default_spacer()
        .with_child(player_play_pause_widget().lens(AppState::playback))
        .with_default_spacer()
        .with_child(
            small_button_widget(&icons::SKIP_FORWARD).on_left_click(|ctx, _, _, _| {
                ctx.submit_command(cmd::PLAY_NEXT);
            }),
        )
        .with_default_spacer()
        .with_child(queue_behavior_widget().lens(AppState::playback))
        .with_default_spacer()
        .with_child(
            Maybe::or_empty(durations_widget).lens(AppState::playback.then(Playback::now_playing)),
        )
        .with_child(Either::new(
            |data: &AppState, _| data.playback.now_playing.is_some(),
            Empty,
            durations_placeholder_widget(),
        ))
        .with_spacer(theme::grid(1.0))
        .with_child(
            toggle_button_widget(&icons::PLAYLIST, |data, _| data.playback_panel_open)
                .padding_right(theme::grid(0.5))
                .on_left_click(|ctx, _, _, _| ctx.submit_command(TOGGLE_QUEUE_PANEL)),
        )
        .with_child(
            toggle_button_widget(&icons::MUSIC_NOTE, |data, _| {
                matches!(data.nav, Nav::Lyrics)
            })
            .align_right()
            .on_left_click(|ctx, _, _, _| {
                ctx.submit_command(TOGGLE_LYRICS);
            }),
        )
        .padding(theme::grid(2.0))
}

pub fn queue_panel_widget() -> impl Widget<AppState> {
    let tabs = Flex::row()
        .with_child(panel_tab_button("Queue", PlaybackPanelTab::Queue))
        .with_spacer(theme::grid(1.0))
        .with_child(panel_tab_button(
            "Recently Played",
            PlaybackPanelTab::RecentlyPlayed,
        ))
        .padding((theme::grid(1.5), theme::grid(1.0)));

    let content = ViewSwitcher::new(
        |data: &AppState, _| data.playback_panel_tab,
        |tab, _, _| match tab {
            PlaybackPanelTab::Queue => Scroll::new(List::new(queue_panel_row_widget))
                .vertical()
                .lens(Map::new(queue_entries, |_, _| {}))
                .boxed(),
            PlaybackPanelTab::RecentlyPlayed => Scroll::new(List::new(queue_panel_row_widget))
                .vertical()
                .lens(Map::new(|data: &AppState| recent_entries(data), |_, _| {}))
                .boxed(),
        },
    );

    Flex::column()
        .with_child(tabs.background(theme::BACKGROUND_DARK))
        .with_child(queue_tabs_divider())
        .with_flex_child(
            content.padding(0.0).background(theme::BACKGROUND_LIGHT),
            1.0,
        )
        .fix_width(theme::grid(36.0))
        .background(theme::BACKGROUND_DARK)
}

fn panel_tab_button(label: &'static str, tab: PlaybackPanelTab) -> impl Widget<AppState> {
    Label::new(label)
        .with_font(theme::UI_FONT_MEDIUM)
        .with_text_color(theme::FOREGROUND_LIGHT)
        .padding((theme::grid(1.0), theme::grid(0.5)))
        .link()
        .rounded(theme::BUTTON_BORDER_RADIUS)
        .active(move |data: &AppState, _| data.playback_panel_tab == tab)
        .on_left_click(move |_, _, data: &mut AppState, _| {
            data.playback_panel_tab = tab;
        })
}

#[derive(Clone, Data)]
struct QueueRow {
    entry: QueueEntry,
    position: usize,
    absolute_index: usize,
    #[data(ignore)]
    entries: Arc<Vector<QueueEntry>>,
    library: Arc<Library>,
    is_now_playing: bool,
    show_remove: bool,
    is_dragging: bool,
    is_drag_over: bool,
    drag_active: bool,
    drag_source_set: bool,
    can_drag: bool,
    insert_after: bool,
    #[data(ignore)]
    drag_start: Option<Point>,
}

#[derive(Clone, Data)]
struct QueueHeader {
    title: Arc<str>,
    subtitle: Option<Arc<str>>,
}

#[derive(Clone, Data)]
enum QueuePanelRow {
    Header(QueueHeader),
    Item(QueueRow),
    Divider(QueueDivider),
}

#[derive(Clone, Data)]
struct QueueDivider;

fn queue_entries(data: &AppState) -> Vector<QueuePanelRow> {
    let mut entries = Vector::new();
    if let Some(now_playing) = &data.playback.now_playing {
        if let Some(position) = data
            .playback
            .queue
            .iter()
            .position(|entry| entry.item.id() == now_playing.item.id())
        {
            for entry in data.playback.queue.iter().skip(position) {
                entries.push_back(entry.clone());
            }
        } else {
            entries = data.playback.queue.clone();
        }
    } else {
        entries = data.playback.queue.clone();
    }
    for entry in data.added_queue.iter() {
        entries.push_back(entry.clone());
    }
    build_queue_panel_rows(data, entries)
}

fn recent_entries(data: &AppState) -> Vector<QueuePanelRow> {
    if data.recently_played.is_empty() {
        return Vector::new();
    }
    let rows = to_queue_rows(
        data.recently_played.clone(),
        Arc::clone(&data.library),
        QueueRowArgs {
            now_playing_id: None,
            drag: &data.queue_drag,
            can_drag: false,
            base_queue_index: 0,
            upcoming_len: 0,
            full_queue_len: 0,
        },
    );
    let mut items = Vector::new();
    items.push_back(QueuePanelRow::Header(QueueHeader {
        title: Arc::from("Recently played"),
        subtitle: None,
    }));
    for row in rows {
        items.push_back(QueuePanelRow::Item(row));
    }
    items
}

struct QueueRowArgs<'a> {
    now_playing_id: Option<ItemId>,
    drag: &'a QueueDragState,
    can_drag: bool,
    base_queue_index: usize,
    upcoming_len: usize,
    full_queue_len: usize,
}

fn to_queue_rows(
    entries: Vector<QueueEntry>,
    library: Arc<Library>,
    args: QueueRowArgs<'_>,
) -> Vector<QueueRow> {
    let shared = Arc::new(entries);
    let mut rows = Vector::new();
    for (position, entry) in shared.iter().enumerate() {
        let absolute_index = if args.can_drag {
            if position < args.upcoming_len {
                args.base_queue_index + position
            } else {
                args.full_queue_len + (position - args.upcoming_len)
            }
        } else {
            position
        };
        let is_now_playing = args.now_playing_id.is_some_and(|id| entry.item.id() == id);
        let drag_active = args.drag.active;
        rows.push_back(QueueRow {
            entry: entry.clone(),
            position,
            absolute_index,
            entries: Arc::clone(&shared),
            library: Arc::clone(&library),
            is_now_playing,
            show_remove: false,
            is_dragging: drag_active && args.drag.source_index == Some(absolute_index),
            is_drag_over: drag_active && args.drag.over_index == Some(absolute_index),
            drag_active,
            drag_source_set: args.drag.source_index.is_some(),
            can_drag: args.can_drag && !is_now_playing,
            insert_after: args.drag.insert_after,
            drag_start: args.drag.start_pos,
        });
    }
    rows
}

fn build_queue_panel_rows(data: &AppState, entries: Vector<QueueEntry>) -> Vector<QueuePanelRow> {
    if entries.is_empty() {
        return Vector::new();
    }
    let now_playing_id = data.playback.now_playing.as_ref().map(|np| np.item.id());
    let base_queue_index = data
        .playback
        .queue
        .iter()
        .position(|entry| now_playing_id.is_some_and(|id| entry.item.id() == id))
        .unwrap_or(0);
    let full_queue_len = data.playback.queue.len();
    let upcoming_len = full_queue_len.saturating_sub(base_queue_index);
    let rows = to_queue_rows(
        entries,
        Arc::clone(&data.library),
        QueueRowArgs {
            now_playing_id,
            drag: &data.queue_drag,
            can_drag: true,
            base_queue_index,
            upcoming_len,
            full_queue_len,
        },
    );

    let mut result = Vector::new();
    result.push_back(QueuePanelRow::Header(QueueHeader {
        title: Arc::from("Now playing"),
        subtitle: None,
    }));

    let mut remaining = Vector::new();
    if let Some(now_id) = now_playing_id {
        for row in rows {
            if row.entry.item.id() == now_id && result.len() == 1 {
                result.push_back(QueuePanelRow::Item(row));
            } else {
                remaining.push_back(row);
            }
        }
    } else {
        result.push_back(QueuePanelRow::Item(rows[0].clone()));
        for row in rows.iter().skip(1) {
            remaining.push_back(row.clone());
        }
    }

    if !remaining.is_empty() {
        let subtitle = data
            .playback
            .now_playing
            .as_ref()
            .map(|np| Arc::from(np.origin.to_string()));
        result.push_back(QueuePanelRow::Divider(QueueDivider));
        result.push_back(QueuePanelRow::Header(QueueHeader {
            title: Arc::from("Next from"),
            subtitle,
        }));
        for mut row in remaining {
            row.show_remove = true;
            result.push_back(QueuePanelRow::Item(row));
        }
    }

    result
}

fn queue_panel_row_widget() -> impl Widget<QueuePanelRow> {
    ViewSwitcher::new(
        |row: &QueuePanelRow, _| match row {
            QueuePanelRow::Header(_) => 0,
            QueuePanelRow::Item(_) => 1,
            QueuePanelRow::Divider(_) => 2,
        },
        |selector, _, _| match *selector {
            0 => queue_header_widget().boxed(),
            1 => queue_row_widget().boxed(),
            _ => queue_section_divider_widget().boxed(),
        },
    )
}

fn queue_section_divider_widget() -> impl Widget<QueuePanelRow> {
    Painter::new(|ctx, _, env| {
        let size = ctx.size();
        let line = Line::new((0.0, 0.0), (size.width, 0.0));
        ctx.stroke(line, &env.get(theme::BORDER_DARK), 1.0);
    })
    .fix_height(1.0)
    .expand_width()
    .background(theme::BACKGROUND_LIGHT)
}

fn queue_tabs_divider() -> impl Widget<AppState> {
    Painter::new(|ctx, _, env| {
        let size = ctx.size();
        let line = Line::new((0.0, 0.0), (size.width, 0.0));
        ctx.stroke(line, &env.get(theme::BORDER_DARK), 1.0);
    })
    .fix_height(1.0)
}

fn queue_header_widget() -> impl Widget<QueuePanelRow> {
    let title = Label::dynamic(|row: &QueuePanelRow, _| match row {
        QueuePanelRow::Header(header) => header.title.to_string(),
        _ => String::new(),
    })
    .with_font(theme::UI_FONT_MEDIUM)
    .with_text_color(theme::FOREGROUND_LIGHT);

    fn subtitle_text(row: &QueuePanelRow, _: &Env) -> String {
        match row {
            QueuePanelRow::Header(header) => header
                .subtitle
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn inline_next_from_text(row: &QueuePanelRow, _: &Env) -> String {
        match row {
            QueuePanelRow::Header(header) if header.title.as_ref() == "Next from" => header
                .subtitle
                .as_ref()
                .filter(|s| !s.is_empty())
                .map(|s| format!("Next from: {}", s))
                .unwrap_or_else(|| "Next from".to_string()),
            _ => String::new(),
        }
    }

    fn has_subtitle(row: &QueuePanelRow, _: &Env) -> bool {
        match row {
            QueuePanelRow::Header(header) => header
                .subtitle
                .as_ref()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            _ => false,
        }
    }

    let clear_button = queue_clear_button();
    Either::new(
        |row: &QueuePanelRow, _| match row {
            QueuePanelRow::Header(header) => header.title.as_ref() == "Next from",
            _ => false,
        },
        Label::dynamic(inline_next_from_text)
            .with_font(theme::UI_FONT_MEDIUM)
            .with_text_color(theme::FOREGROUND_LIGHT)
            .with_line_break_mode(LineBreaking::Clip)
            .padding((theme::grid(1.0), theme::grid(0.5)))
            .expand_width()
            .background(theme::BACKGROUND_DARK),
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Flex::row()
                    .with_flex_child(title, 1.0)
                    .with_child(clear_button),
            )
            .with_child(Either::new(
                has_subtitle,
                Label::dynamic(subtitle_text)
                    .with_text_size(theme::TEXT_SIZE_SMALL)
                    .with_text_color(theme::PLACEHOLDER_COLOR),
                Empty,
            ))
            .padding((theme::grid(1.0), theme::grid(0.5)))
            .expand_width()
            .background(theme::BACKGROUND_DARK),
    )
}

fn queue_clear_button() -> impl Widget<QueuePanelRow> {
    Either::new(
        |row: &QueuePanelRow, _| matches!(row, QueuePanelRow::Header(header) if header.title.as_ref() == "Now playing"),
        Label::new("Clear")
            .with_text_size(theme::TEXT_SIZE_SMALL)
            .with_text_color(theme::PLACEHOLDER_COLOR)
            .padding((theme::grid(0.6), theme::grid(0.2)))
            .link()
            .rounded(theme::BUTTON_BORDER_RADIUS)
            .on_left_click(|ctx, _, _, _| {
                ctx.submit_command(CLEAR_QUEUE);
            }),
        Empty,
    )
}

fn queue_row_widget() -> impl Widget<QueuePanelRow> {
    let title = Label::dynamic(|row: &QueuePanelRow, _| match row {
        QueuePanelRow::Item(item) => item.entry.item.name().to_string(),
        _ => String::new(),
    })
    .with_line_break_mode(LineBreaking::Clip)
    .with_font(theme::UI_FONT_MEDIUM);
    let duration = Label::dynamic(|row: &QueuePanelRow, _| match row {
        QueuePanelRow::Item(item) => utils::as_minutes_and_seconds(item.entry.item.duration()),
        _ => String::new(),
    })
    .with_text_size(theme::TEXT_SIZE_SMALL)
    .with_text_color(theme::PLACEHOLDER_COLOR)
    .with_line_break_mode(LineBreaking::Clip);
    let subtitle = Label::dynamic(|row: &QueuePanelRow, _| match row {
        QueuePanelRow::Item(item) => match &item.entry.item {
            Playable::Track(track) => track.artist_names(),
            Playable::Episode(episode) => episode.show.name.as_ref().to_string(),
        },
        _ => String::new(),
    })
    .with_line_break_mode(LineBreaking::Clip)
    .with_text_size(theme::TEXT_SIZE_SMALL)
    .with_text_color(theme::PLACEHOLDER_COLOR);

    let cover = queue_cover_widget(theme::grid(4.0));
    let remove_button = queue_remove_slot();
    let title_row = Flex::row()
        .with_flex_child(title, 1.0)
        .with_child(SizedBox::new(Align::right(duration)).fix_width(theme::grid(5.0)));

    Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(cover)
        .with_spacer(theme::grid(1.0))
        .with_flex_child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(title_row)
                .with_child(subtitle),
            1.0,
        )
        .with_spacer(theme::grid(1.0))
        .with_child(remove_button)
        .padding(theme::grid(1.0))
        .expand_width()
        .background(queue_row_background())
        .controller(QueueRowDragController)
        .context_menu(|row: &QueuePanelRow| match row {
            QueuePanelRow::Item(item) => match &item.entry.item {
                Playable::Track(track) => {
                    let mut menu =
                        track::track_menu(track, &item.library, &item.entry.origin, usize::MAX);
                    if item.show_remove {
                        menu = menu.entry(
                            MenuItem::new("Remove from Queue")
                                .command(REMOVE_FROM_QUEUE.with(item.absolute_index)),
                        );
                    }
                    menu
                }
                Playable::Episode(episode) => {
                    let mut menu = episode::episode_menu(episode, &item.library);
                    if item.show_remove {
                        menu = menu.entry(
                            MenuItem::new("Remove from Queue")
                                .command(REMOVE_FROM_QUEUE.with(item.absolute_index)),
                        );
                    }
                    menu
                }
            },
            _ => Menu::empty(),
        })
}

fn queue_remove_slot() -> impl Widget<QueuePanelRow> {
    let width = theme::grid(4.0);
    let button = queue_remove_icon()
        .fix_size(theme::ICON_SIZE_SMALL.width, theme::ICON_SIZE_SMALL.height)
        .padding(theme::grid(1.0))
        .link()
        .circle()
        .on_left_click(|ctx, _, row: &mut QueuePanelRow, _| {
            if let QueuePanelRow::Item(item) = row {
                ctx.submit_command(REMOVE_FROM_QUEUE.with(item.absolute_index));
                ctx.set_handled();
            }
        });
    let button = SizedBox::new(button).fix_width(width);
    let spacer = SizedBox::empty().fix_width(width);
    Either::new(
        |row: &QueuePanelRow, _| matches!(row, QueuePanelRow::Item(item) if item.show_remove),
        button,
        spacer,
    )
}

fn queue_remove_icon() -> impl Widget<QueuePanelRow> {
    Painter::new(|ctx, _, env| {
        let size = ctx.size();
        let center = size.to_rect().center();
        let radius = (size.width.min(size.height) * 0.5) - 1.0;
        let color = env.get(theme::MEDIA_CONTROL_ICON_MUTED);
        ctx.stroke(Circle::new(center, radius), &color, 1.0);
        let half = radius * 0.6;
        let line = Line::new((center.x - half, center.y), (center.x + half, center.y));
        ctx.stroke(line, &color, 1.4);
    })
}

#[derive(Default)]
struct QueueRowDragController;

impl<W> Controller<QueuePanelRow, W> for QueueRowDragController
where
    W: Widget<QueuePanelRow>,
{
    fn event(
        &mut self,
        child: &mut W,
        ctx: &mut EventCtx,
        event: &Event,
        data: &mut QueuePanelRow,
        env: &Env,
    ) {
        match event {
            Event::MouseDown(mouse) if mouse.button == MouseButton::Left => {
                if let QueuePanelRow::Item(item) = data
                    && item.can_drag
                    && !queue_remove_hitbox(item, mouse.pos, ctx.size())
                {
                    ctx.submit_command(
                        QUEUE_DRAG_BEGIN
                            .with(cmd::QueueDragBegin {
                                index: item.absolute_index,
                                start_pos: mouse.window_pos,
                            })
                            .to(Target::Global),
                    );
                }
            }
            Event::MouseMove(mouse) => {
                if let QueuePanelRow::Item(item) = data {
                    let dragging =
                        item.drag_source_set && mouse.buttons.contains(MouseButton::Left);
                    let cursor = queue_drag_cursor(dragging);
                    ctx.set_cursor(&cursor);
                    if item.can_drag {
                        if !mouse.buttons.contains(MouseButton::Left) {
                            child.event(ctx, event, data, env);
                            return;
                        }
                        if !item.drag_active
                            && item.drag_source_set
                            && let Some(start) = item.drag_start
                        {
                            let delta = mouse.window_pos - start;
                            if delta.hypot() <= 4.0 {
                                child.event(ctx, event, data, env);
                                return;
                            }
                        }
                        let insert_after = mouse.pos.y > ctx.size().height * 0.5;
                        if item.drag_active
                            && item.is_drag_over
                            && item.insert_after == insert_after
                        {
                            child.event(ctx, event, data, env);
                            return;
                        }
                        ctx.submit_command(
                            QUEUE_DRAG_OVER
                                .with(cmd::QueueDragOver {
                                    index: item.absolute_index,
                                    insert_after,
                                })
                                .to(Target::Global),
                        );
                    }
                }
            }
            Event::MouseUp(mouse) if mouse.button == MouseButton::Left => {
                child.event(ctx, event, data, env);
                if ctx.is_handled() {
                    return;
                }
                if let QueuePanelRow::Item(item) = data
                    && item.drag_active
                {
                    ctx.submit_command(QUEUE_DRAG_END.to(Target::Global));
                } else if let QueuePanelRow::Item(item) = data
                    && ctx.is_hot()
                {
                    ctx.submit_command(cmd::PLAY_QUEUE_ENTRIES.with(cmd::QueuePlayRequest {
                        entries: (*item.entries).clone(),
                        position: item.position,
                    }));
                }
                return;
            }
            _ => {}
        }
        child.event(ctx, event, data, env);
    }
}

fn queue_remove_hitbox(item: &QueueRow, mouse_pos: Point, size: Size) -> bool {
    if !item.show_remove {
        return false;
    }
    let hit_width = theme::grid(4.0);
    mouse_pos.x >= size.width - hit_width
}

#[allow(deprecated)]
fn queue_drag_cursor(dragging: bool) -> Cursor {
    if dragging {
        Cursor::OpenHand
    } else {
        Cursor::Pointer
    }
}

fn queue_row_background() -> druid::widget::Painter<QueuePanelRow> {
    druid::widget::Painter::new(|ctx, row: &QueuePanelRow, env| {
        let mut color = if ctx.is_active() {
            env.get(theme::GREY_500)
        } else if ctx.is_hot() {
            env.get(theme::GREY_600)
        } else {
            env.get(theme::BACKGROUND_LIGHT)
        };
        if let QueuePanelRow::Item(item) = row
            && item.is_drag_over
        {
            color = env.get(theme::GREY_500);
        }
        let rect = ctx.size().to_rect();
        ctx.fill(rect, &color);

        if let QueuePanelRow::Item(item) = row
            && item.is_drag_over
        {
            let y = if item.insert_after {
                rect.y1 - 1.0
            } else {
                rect.y0 + 1.0
            };
            let line = Line::new((rect.x0 + theme::grid(6.0), y), (rect.x1, y));
            ctx.stroke(line, &env.get(theme::BLUE_100), 2.0);
        }

        if ctx.is_hot() && !matches!(row, QueuePanelRow::Item(item) if item.drag_active) {
            let padding = theme::grid(1.0);
            let cover_size = theme::grid(4.0);
            let cover_rect = Rect::from_origin_size(
                Point::new(padding, padding),
                Size::new(cover_size, cover_size),
            );
            ctx.fill(cover_rect, &env.get(theme::GREY_600).with_alpha(0.6));

            let center = cover_rect.center();
            let icon_size = theme::grid(2.0);
            let half = icon_size * 0.5;
            let mut path = BezPath::new();
            path.move_to((center.x - half * 0.4, center.y - half));
            path.line_to((center.x - half * 0.4, center.y + half));
            path.line_to((center.x + half, center.y));
            path.close_path();
            ctx.fill(path, &env.get(theme::FOREGROUND_LIGHT));
        }
    })
}

fn queue_cover_widget(size: f64) -> impl Widget<QueuePanelRow> {
    RemoteImage::new(
        utils::placeholder_widget(),
        move |row: &QueuePanelRow, _| match row {
            QueuePanelRow::Item(item) => match &item.entry.item {
                Playable::Track(track) => track
                    .album
                    .as_ref()
                    .and_then(|album| album.image(size, size).map(|img| img.url.clone())),
                Playable::Episode(episode) => episode.image(size, size).map(|img| img.url.clone()),
            },
            _ => None,
        },
    )
    .fix_size(size, size)
    .clip(Size::new(size, size).to_rounded_rect(4.0))
}

fn player_play_pause_widget() -> impl Widget<Playback> {
    ViewSwitcher::new(
        |playback: &Playback, _| playback.state,
        |state, _, _| match state {
            PlaybackState::Loading => Spinner::new()
                .with_color(theme::MEDIA_CONTROL_ICON_MUTED)
                .fix_size(theme::grid(3.0), theme::grid(3.0))
                .padding(theme::grid(1.0))
                .link()
                .circle()
                .border(theme::MEDIA_CONTROL_BORDER, 1.0)
                .on_left_click(|ctx, _, _, _| ctx.submit_command(cmd::PLAY_STOP))
                .boxed(),
            PlaybackState::Playing => icons::PAUSE
                .scale((theme::grid(3.0), theme::grid(3.0)))
                .with_color(theme::MEDIA_CONTROL_ICON)
                .padding(theme::grid(1.0))
                .link()
                .circle()
                .border(theme::MEDIA_CONTROL_BORDER, 1.0)
                .on_left_click(|ctx, _, _, _| ctx.submit_command(cmd::PLAY_PAUSE))
                .boxed(),
            PlaybackState::Paused => icons::PLAY
                .scale((theme::grid(3.0), theme::grid(3.0)))
                .with_color(theme::MEDIA_CONTROL_ICON)
                .padding(theme::grid(1.0))
                .link()
                .circle()
                .border(theme::MEDIA_CONTROL_BORDER, 1.0)
                .on_left_click(|ctx, _, _, _| ctx.submit_command(cmd::PLAY_RESUME))
                .boxed(),
            PlaybackState::Stopped => icons::PLAY
                .scale((theme::grid(3.0), theme::grid(3.0)))
                .with_color(theme::MEDIA_CONTROL_ICON_MUTED)
                .padding(theme::grid(1.0))
                .link()
                .circle()
                .border(theme::MEDIA_CONTROL_BORDER, 1.0)
                .boxed(),
        },
    )
}

fn queue_behavior_widget() -> impl Widget<Playback> {
    ViewSwitcher::new(
        |playback: &Playback, _| playback.queue_behavior,
        |behavior, _, _| {
            faded_button_widget(queue_behavior_icon(behavior))
                .on_left_click(|ctx, _, playback: &mut Playback, _| {
                    ctx.submit_command(
                        cmd::PLAY_QUEUE_BEHAVIOR
                            .with(cycle_queue_behavior(&playback.queue_behavior)),
                    );
                })
                .boxed()
        },
    )
}

fn cycle_queue_behavior(qb: &QueueBehavior) -> QueueBehavior {
    match qb {
        QueueBehavior::Sequential => QueueBehavior::Random,
        QueueBehavior::Random => QueueBehavior::LoopTrack,
        QueueBehavior::LoopTrack => QueueBehavior::LoopAll,
        QueueBehavior::LoopAll => QueueBehavior::Sequential,
    }
}

fn queue_behavior_icon(qb: &QueueBehavior) -> &'static SvgIcon {
    match qb {
        QueueBehavior::Sequential => &icons::PLAY_SEQUENTIAL,
        QueueBehavior::Random => &icons::PLAY_SHUFFLE,
        QueueBehavior::LoopTrack => &icons::PLAY_LOOP_TRACK,
        QueueBehavior::LoopAll => &icons::PLAY_LOOP_ALL,
    }
}

fn small_button_widget<T: Data>(svg: &SvgIcon) -> impl Widget<T> {
    svg.scale((theme::grid(2.0), theme::grid(2.0)))
        .with_color(theme::MEDIA_CONTROL_ICON)
        .padding(theme::grid(1.0))
        .link()
        .rounded(theme::BUTTON_BORDER_RADIUS)
}

fn toggle_button_widget(
    svg: &'static SvgIcon,
    is_active: impl Fn(&AppState, &Env) -> bool + 'static,
) -> impl Widget<AppState> {
    ViewSwitcher::new(
        move |data: &AppState, env| is_active(data, env),
        move |active, _, _| {
            let icon = svg
                .scale((theme::grid(2.0), theme::grid(2.0)))
                .with_color(if *active {
                    theme::PLAYBACK_TOGGLE_FG_ACTIVE
                } else {
                    theme::MEDIA_CONTROL_ICON
                });
            let base_color = if *active {
                theme::PLAYBACK_TOGGLE_BG_ACTIVE
            } else {
                theme::PLAYBACK_TOGGLE_BG_INACTIVE
            };
            icon.padding(theme::grid(0.75))
                .background(Painter::new(move |ctx, _, env| {
                    let base_color = base_color.clone();
                    let color = if ctx.is_hot() {
                        env.get(theme::LINK_HOT_COLOR)
                    } else {
                        env.get(base_color)
                    };
                    let bounds = ctx
                        .size()
                        .to_rounded_rect(env.get(theme::BUTTON_BORDER_RADIUS));
                    ctx.fill(bounds, &color);
                }))
                .boxed()
        },
    )
}

fn faded_button_widget<T: Data>(svg: &SvgIcon) -> impl Widget<T> {
    svg.scale((theme::grid(2.0), theme::grid(2.0)))
        .with_color(theme::MEDIA_CONTROL_ICON_MUTED)
        .padding(theme::grid(1.0))
        .link()
        .rounded(theme::BUTTON_BORDER_RADIUS)
}

fn durations_widget() -> impl Widget<NowPlaying> {
    Label::dynamic(|now_playing: &NowPlaying, _| {
        format!(
            "{} / {}",
            utils::as_minutes_and_seconds(now_playing.progress),
            utils::as_minutes_and_seconds(now_playing.item.duration())
        )
    })
    .with_text_size(theme::TEXT_SIZE_SMALL)
    .with_text_color(theme::PLACEHOLDER_COLOR)
    .fix_width(theme::grid(8.0))
}

fn durations_placeholder_widget() -> impl Widget<AppState> {
    Label::new("--:-- / --:--")
        .with_text_size(theme::TEXT_SIZE_SMALL)
        .with_text_color(theme::PLACEHOLDER_COLOR)
        .fix_width(theme::grid(8.0))
}

struct BarLayout<T, I, P> {
    item: WidgetPod<T, I>,
    player: WidgetPod<T, P>,
}

impl<T, I, P> BarLayout<T, I, P>
where
    T: Data,
    I: Widget<T>,
    P: Widget<T>,
{
    fn new(item: I, player: P) -> Self {
        Self {
            item: WidgetPod::new(item),
            player: WidgetPod::new(player),
        }
    }
}

impl<T, I, P> Widget<T> for BarLayout<T, I, P>
where
    T: Data,
    I: Widget<T>,
    P: Widget<T>,
{
    fn event(&mut self, ctx: &mut EventCtx, event: &Event, data: &mut T, env: &Env) {
        self.item.event(ctx, event, data, env);
        self.player.event(ctx, event, data, env);
    }

    fn lifecycle(&mut self, ctx: &mut LifeCycleCtx, event: &LifeCycle, data: &T, env: &Env) {
        self.item.lifecycle(ctx, event, data, env);
        self.player.lifecycle(ctx, event, data, env);
    }

    fn update(&mut self, ctx: &mut UpdateCtx, _old_data: &T, data: &T, env: &Env) {
        self.item.update(ctx, data, env);
        self.player.update(ctx, data, env);
    }

    fn layout(&mut self, ctx: &mut LayoutCtx, bc: &BoxConstraints, data: &T, env: &Env) -> Size {
        let max = bc.max();

        const PLAYER_OPTICAL_CENTER: f64 = 60.0 + theme::GRID * 2.0;

        // Layout the player with loose constraints.
        let player = self.player.layout(ctx, &bc.loosen(), data, env);
        let player_centered = max.width > player.width * 2.25;

        // Layout the item to the available space.
        let item_max = if player_centered {
            Size::new(max.width * 0.5 - PLAYER_OPTICAL_CENTER, max.height)
        } else {
            Size::new(max.width - player.width, max.height)
        };
        let item = self
            .item
            .layout(ctx, &BoxConstraints::new(Size::ZERO, item_max), data, env);

        let total = Size::new(max.width, player.height.max(item.height));

        // Put the item to the top left.
        self.item.set_origin(ctx, Point::ORIGIN);

        // Put the player either to the center or to the right.
        let player_pos = if player_centered {
            Point::new(
                total.width * 0.5 - PLAYER_OPTICAL_CENTER,
                total.height * 0.5 - player.height * 0.5,
            )
        } else {
            Point::new(
                total.width - player.width,
                total.height * 0.5 - player.height * 0.5,
            )
        };
        self.player.set_origin(ctx, player_pos);

        total
    }

    fn paint(&mut self, ctx: &mut PaintCtx, data: &T, env: &Env) {
        self.item.paint(ctx, data, env);
        self.player.paint(ctx, data, env);
    }
}

struct SeekBar {
    loudness_path: BezPath,
    base_progress: Duration,
    last_tick: Option<Instant>,
}

impl SeekBar {
    fn new() -> Self {
        Self {
            loudness_path: BezPath::new(),
            base_progress: Duration::ZERO,
            last_tick: None,
        }
    }

    fn current_progress(&self, data: &NowPlaying) -> Duration {
        let mut progress = self.base_progress;
        if data.is_playing
            && let Some(last_tick) = self.last_tick
        {
            progress = progress.saturating_add(last_tick.elapsed());
        }
        progress.min(data.item.duration())
    }
}

impl Widget<NowPlaying> for SeekBar {
    fn event(&mut self, ctx: &mut EventCtx, event: &Event, data: &mut NowPlaying, _env: &Env) {
        match event {
            Event::MouseMove(_) => {
                ctx.set_cursor(&Cursor::Pointer);
            }
            Event::MouseDown(mouse) => {
                if mouse.button == MouseButton::Left {
                    ctx.set_active(true);
                }
            }
            Event::MouseUp(mouse) => {
                if ctx.is_active() && mouse.button == MouseButton::Left {
                    if ctx.is_hot() {
                        let fraction = mouse.pos.x / ctx.size().width;
                        ctx.submit_command(cmd::PLAY_SEEK.with(fraction));
                    }
                    ctx.set_active(false);
                }
            }
            Event::AnimFrame(_) => {
                if data.is_playing {
                    ctx.request_paint();
                    ctx.request_anim_frame();
                }
            }
            _ => {}
        }
    }

    fn lifecycle(
        &mut self,
        ctx: &mut LifeCycleCtx,
        event: &LifeCycle,
        _data: &NowPlaying,
        _env: &Env,
    ) {
        match &event {
            LifeCycle::Size(_bounds) => {
                // self.loudness_path = compute_loudness_path(bounds, &data);
            }
            LifeCycle::HotChanged(_) => {
                ctx.request_paint();
            }
            _ => {}
        }
    }

    fn update(
        &mut self,
        ctx: &mut UpdateCtx,
        old_data: &NowPlaying,
        data: &NowPlaying,
        _env: &Env,
    ) {
        if !old_data.same(data) {
            self.base_progress = data.progress;
            self.last_tick = data.is_playing.then_some(Instant::now());
            if data.is_playing {
                ctx.request_anim_frame();
            }
            ctx.request_paint();
        } else if data.is_playing {
            // Keep animating while playing.
            ctx.request_anim_frame();
        } else {
            self.last_tick = None;
        }
    }

    fn layout(
        &mut self,
        _ctx: &mut LayoutCtx,
        bc: &BoxConstraints,
        _data: &NowPlaying,
        _env: &Env,
    ) -> Size {
        Size::new(bc.max().width, theme::grid(1.0))
    }

    fn paint(&mut self, ctx: &mut PaintCtx, data: &NowPlaying, env: &Env) {
        let progress = self.current_progress(data);
        if self.loudness_path.is_empty() {
            paint_progress_bar(ctx, data, env, progress)
        } else {
            paint_audio_analysis(ctx, data, &self.loudness_path, env, progress)
        }
    }
}

fn _compute_loudness_path_from_analysis(
    bounds: &Size,
    total_duration: &Duration,
    analysis: &AudioAnalysis,
) -> BezPath {
    let (loudness_min, loudness_max) = analysis
        .segments
        .iter()
        .map(|s| s.loudness_max)
        .minmax()
        .into_option()
        .unwrap_or((0.0, 0.0));
    let total_loudness = loudness_max - loudness_min;

    let mut path = BezPath::new();

    // We start in the middle of the vertical space and first draw the upper half of
    // the curve, then take what we have drawn, flip the y-axis and append it
    // underneath.
    let origin_y = bounds.height / 2.0;

    // Start at the origin.
    path.move_to((0.0, origin_y));

    // Because the size of the seekbar is quite small, but the number of the
    // segments can be large, we down-sample the loudness spectrum in a very
    // primitive way and only add a vertex after crossing `WIDTH_PRECISION` of
    // pixels horizontally.
    const WIDTH_PRECISION: f64 = 2.0;
    let mut last_width = 0.0;

    for seg in &analysis.segments {
        let time = seg.interval.start.as_secs_f64() + seg.loudness_max_time;
        let tfrac = time / total_duration.as_secs_f64();
        let width = bounds.width * tfrac;

        let loud = seg.loudness_max - loudness_min;
        let lfrac = loud / total_loudness;
        let height = bounds.height * lfrac;

        if width - last_width >= WIDTH_PRECISION {
            // Down-scale the height, because we will be drawing also the inverted half.
            path.line_to((width, origin_y - height / 2.0));

            // Save the X-coordinate of this vertex.
            last_width = width;
        }
    }

    // Land back at the vertical origin.
    path.line_to((bounds.width, origin_y));

    // Flip the y-axis, translate just under the origin, and append.
    let mut inverted_path = path.clone();
    let inversion_tx = Affine::FLIP_Y * Affine::translate((0.0, -bounds.height));
    inverted_path.apply_affine(inversion_tx);
    path.extend(inverted_path);

    path
}

fn paint_audio_analysis(
    ctx: &mut PaintCtx,
    data: &NowPlaying,
    path: &BezPath,
    env: &Env,
    progress: Duration,
) {
    let bounds = ctx.size();

    let elapsed_time = progress.as_secs_f64();
    let total_time = data.item.duration().as_secs_f64();
    let elapsed_frac = elapsed_time / total_time;
    let elapsed_width = bounds.width * elapsed_frac;
    let elapsed = Size::new(elapsed_width, bounds.height).to_rect();

    let (elapsed_color, remaining_color) = if ctx.is_hot() {
        (env.get(theme::GREY_200), env.get(theme::GREY_500))
    } else {
        (env.get(theme::GREY_300), env.get(theme::GREY_600))
    };

    ctx.with_save(|ctx| {
        ctx.fill(path, &remaining_color);
        ctx.clip(elapsed);
        ctx.fill(path, &elapsed_color);
    });
}

fn paint_progress_bar(ctx: &mut PaintCtx, data: &NowPlaying, env: &Env, progress: Duration) {
    let elapsed_time = progress.as_secs_f64();
    let total_time = data.item.duration().as_secs_f64();

    let (elapsed_color, remaining_color) = if ctx.is_hot() {
        (env.get(theme::GREY_200), env.get(theme::GREY_500))
    } else {
        (env.get(theme::GREY_300), env.get(theme::GREY_600))
    };
    let bounds = ctx.size();

    let elapsed_frac = elapsed_time / total_time;
    let elapsed_width = bounds.width * elapsed_frac;
    let remaining_width = bounds.width - elapsed_width;
    let elapsed = Size::new(elapsed_width, bounds.height).round();
    let remaining = Size::new(remaining_width, bounds.height).round();

    ctx.fill(
        Rect::from_origin_size(Point::ORIGIN, elapsed),
        &elapsed_color,
    );
    ctx.fill(
        Rect::from_origin_size(Point::new(elapsed.width, 0.0), remaining),
        &remaining_color,
    );
}
