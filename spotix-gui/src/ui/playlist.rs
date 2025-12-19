use std::{cell::RefCell, rc::Rc, sync::Arc};

use druid::{
    Data, Insets, Lens, LensExt, LocalizedString, Menu, MenuItem, Selector, Size, UnitPoint,
    Widget, WidgetExt, WindowDesc,
    im::Vector,
    widget::{
        Button, Either, Flex, Label, LensWrap, LineBreaking, List, Spinner, TextBox, ViewSwitcher,
    },
};

use crate::{
    cmd,
    data::{
        AppState, Ctx, Library, Nav, Playlist, PlaylistAddTrack, PlaylistDetail, PlaylistLink,
        PlaylistRemoveTrack, PlaylistTracks, Promise, Track, WithCtx,
        config::{SortCriteria, SortOrder},
    },
    error::Error,
    ui::menu,
    webapi::WebApi,
    widget::{Async, Empty, MyWidgetExt, RemoteImage, ThemeScope},
};

use super::{playable, theme, track, utils};

pub const LOAD_LIST: Selector = Selector::new("app.playlist.load-list");
pub const LOAD_DETAIL: Selector<(PlaylistLink, SortCriteria, SortOrder, bool)> =
    Selector::new("app.playlist.load-detail");
pub const LOAD_MORE_TRACKS: Selector<(PlaylistLink, usize)> =
    Selector::new("app.playlist.load-more-tracks");
const PAGE_SIZE: usize = 100;

fn sort_playlist_tracks(tracks: &mut PlaylistTracks, criteria: SortCriteria, order: SortOrder) {
    let mut items: Vec<(usize, Arc<Track>)> = tracks.tracks.iter().cloned().enumerate().collect();

    let cmp_str = |a: &str, b: &str| a.to_lowercase().cmp(&b.to_lowercase());

    items.sort_by(|(idx_a, a), (idx_b, b)| {
        let mut ord = match criteria {
            SortCriteria::Title => cmp_str(&a.name, &b.name),
            SortCriteria::Artist => cmp_str(&a.artist_name(), &b.artist_name()),
            SortCriteria::Album => cmp_str(&a.album_name(), &b.album_name()),
            SortCriteria::Duration => a.duration.cmp(&b.duration),
            SortCriteria::DateAdded => idx_a.cmp(idx_b),
        };
        if order == SortOrder::Descending {
            ord = ord.reverse();
        }
        ord
    });

    tracks.tracks = items.into_iter().map(|(_, track)| track).collect();
}

#[derive(Clone, Data)]
struct PlaylistTracksPage {
    items: Vector<Arc<Track>>,
    total: usize,
    offset: usize,
    limit: usize,
}
pub const ADD_TRACK: Selector<PlaylistAddTrack> = Selector::new("app.playlist.add-track");
pub const REMOVE_TRACK: Selector<PlaylistRemoveTrack> = Selector::new("app.playlist.remove-track");

pub const FOLLOW_PLAYLIST: Selector<Playlist> = Selector::new("app.playlist.follow");
pub const UNFOLLOW_PLAYLIST: Selector<PlaylistLink> = Selector::new("app.playlist.unfollow");
pub const UNFOLLOW_PLAYLIST_CONFIRM: Selector<PlaylistLink> =
    Selector::new("app.playlist.unfollow-confirm");

pub const RENAME_PLAYLIST: Selector<PlaylistLink> = Selector::new("app.playlist.rename");
pub const RENAME_PLAYLIST_CONFIRM: Selector<PlaylistLink> =
    Selector::new("app.playlist.rename-confirm");

const SHOW_RENAME_PLAYLIST_CONFIRM: Selector<PlaylistLink> =
    Selector::new("app.playlist.show-rename");
const SHOW_UNFOLLOW_PLAYLIST_CONFIRM: Selector<UnfollowPlaylist> =
    Selector::new("app.playlist.show-unfollow-confirm");

pub fn list_widget() -> impl Widget<AppState> {
    Async::new(
        utils::spinner_widget,
        || {
            List::new(|| {
                Label::raw()
                    .with_line_break_mode(LineBreaking::WordWrap)
                    .with_text_size(theme::TEXT_SIZE_SMALL)
                    .lens(Ctx::data().then(Playlist::name))
                    .expand_width()
                    .padding(Insets::uniform_xy(theme::grid(2.0), theme::grid(0.6)))
                    .link()
                    .on_left_click(|ctx, _, playlist, _| {
                        ctx.submit_command(
                            cmd::NAVIGATE.with(Nav::PlaylistDetail(playlist.data.link())),
                        );
                    })
                    .context_menu(playlist_menu_ctx)
            })
        },
        || utils::retry_error_widget(LOAD_LIST),
    )
    .lens(
        Ctx::make(
            AppState::common_ctx,
            AppState::library.then(Library::playlists.in_arc()),
        )
        .then(Ctx::in_promise()),
    )
    .on_command_async(
        LOAD_LIST,
        |_| WebApi::global().get_playlists(),
        |_, data, d| data.with_library_mut(|l| l.playlists.defer(d)),
        |_, data, r| data.with_library_mut(|l| l.playlists.update(r)),
    )
    .on_command_async(
        ADD_TRACK,
        |d| {
            WebApi::global().add_track_to_playlist(
                &d.link.id,
                &d.track_id
                    .0
                    .to_uri()
                    .ok_or_else(|| Error::WebApiError("Item doesn't have URI".to_string()))?,
            )
        },
        |_, data, d| {
            data.with_library_mut(|library| library.increment_playlist_track_count(&d.link))
        },
        |_, data, (_, r)| {
            if let Err(err) = r {
                data.error_alert(err);
            } else {
                data.info_alert("Added to playlist.");
            }
        },
    )
    .on_command_async(
        UNFOLLOW_PLAYLIST,
        |link| WebApi::global().unfollow_playlist(link.id.as_ref()),
        |_, data: &mut AppState, d| data.with_library_mut(|l| l.remove_from_playlist(&d.id)),
        |_, data, (_, r)| {
            if let Err(err) = r {
                data.error_alert(err);
            } else {
                data.info_alert("Playlist removed from library.");
            }
        },
    )
    .on_command_async(
        FOLLOW_PLAYLIST,
        |link| WebApi::global().follow_playlist(link.id.as_ref()),
        |_, data: &mut AppState, d| data.with_library_mut(|l| l.add_playlist(d)),
        |_, data: &mut AppState, (_, r)| {
            if let Err(err) = r {
                data.error_alert(err);
            } else {
                data.info_alert("Playlist added to library.")
            }
        },
    )
    .on_command_async(
        RENAME_PLAYLIST,
        |link| WebApi::global().change_playlist_details(link.id.as_ref(), link.name.as_ref()),
        |_, data: &mut AppState, link| data.with_library_mut(|l| l.rename_playlist(link)),
        |_, data: &mut AppState, (_, r)| {
            if let Err(err) = r {
                data.error_alert(err);
            } else {
                data.info_alert("Playlist renamed.")
            }
        },
    )
    .on_command(SHOW_UNFOLLOW_PLAYLIST_CONFIRM, |ctx, msg, _| {
        let window = unfollow_confirm_window(msg.clone());
        ctx.new_window(window);
    })
    .on_command(SHOW_RENAME_PLAYLIST_CONFIRM, |ctx, link, _| {
        let window = rename_playlist_window(link.clone());
        ctx.new_window(window);
    })
    .on_command_async(
        REMOVE_TRACK,
        |d| WebApi::global().remove_track_from_playlist(&d.link.id, d.track_pos),
        |_, data, d| {
            data.with_library_mut(|library| library.decrement_playlist_track_count(&d.link))
        },
        |e, data, (p, r)| {
            if let Err(err) = r {
                data.error_alert(err);
            } else {
                data.info_alert("Removed from playlist.");
            }
            // Re-submit the `LOAD_DETAIL` command to reload the playlist data.
            e.submit_command(LOAD_DETAIL.with((
                p.link,
                data.config.sort_criteria,
                data.config.sort_order,
                data.config.enable_pagination,
            )))
        },
    )
}

fn unfollow_confirm_window(msg: UnfollowPlaylist) -> WindowDesc<AppState> {
    let win = WindowDesc::new(unfollow_playlist_confirm_widget(msg))
        .window_size((theme::grid(45.0), theme::grid(25.0)))
        .title("Unfollow playlist")
        .resizable(false)
        .show_titlebar(false)
        .transparent(true);
    if cfg!(target_os = "macos") {
        win.menu(menu::main_menu)
    } else {
        win
    }
}

fn unfollow_playlist_confirm_widget(msg: UnfollowPlaylist) -> impl Widget<AppState> {
    let link = msg.link;

    let information_section = if msg.created_by_user {
        information_section(
            format!("Delete {} from Library?", link.name),
            "This will delete the playlist from Your Library".to_string(),
        )
    } else {
        information_section(
            format!("Remove {} from Library?", link.name),
            "We'll remove this playlist from Your Library, but you'll still be able to search for it on Spotify"
                .to_string(),
        )
    };

    let button_section = button_section(
        "Delete",
        UNFOLLOW_PLAYLIST_CONFIRM,
        Box::new(move || link.clone()),
    );

    ThemeScope::new(
        Flex::column()
            .with_child(information_section)
            .with_flex_spacer(2.0)
            .with_child(button_section)
            .with_flex_spacer(2.0)
            .background(theme::BACKGROUND_DARK),
    )
}

fn rename_playlist_window(link: PlaylistLink) -> WindowDesc<AppState> {
    let win = WindowDesc::new(rename_playlist_widget(link))
        .window_size((theme::grid(45.0), theme::grid(30.0)))
        .title("Rename playlist")
        .resizable(false)
        .show_titlebar(false)
        .transparent(true);
    if cfg!(target_os = "macos") {
        win.menu(menu::main_menu)
    } else {
        win
    }
}

#[derive(Clone, Lens)]
struct TextInput {
    input: Rc<RefCell<String>>,
}

impl Lens<AppState, String> for TextInput {
    fn with<V, F: FnOnce(&String) -> V>(&self, _data: &AppState, f: F) -> V {
        f(&self.input.borrow())
    }

    fn with_mut<V, F: FnOnce(&mut String) -> V>(&self, _data: &mut AppState, f: F) -> V {
        f(&mut self.input.borrow_mut())
    }
}

fn rename_playlist_widget(link: PlaylistLink) -> impl Widget<AppState> {
    let text_input = TextInput {
        input: Rc::new(RefCell::new(link.name.to_string())),
    };

    let information_section = information_section(
        "Rename playlist?".to_string(),
        "Please enter a new name for your playlist".to_string(),
    );
    let input_section = LensWrap::new(
        TextBox::new()
            .padding_horizontal(theme::grid(2.0))
            .expand_width(),
        text_input.clone(),
    );
    let button_section = button_section(
        "Rename",
        RENAME_PLAYLIST_CONFIRM,
        Box::new(move || PlaylistLink {
            id: link.id.clone(),
            name: Arc::from(text_input.input.borrow().clone().into_boxed_str()),
        }),
    );

    ThemeScope::new(
        Flex::column()
            .with_child(information_section)
            .with_child(input_section)
            .with_flex_spacer(2.0)
            .with_child(button_section)
            .with_flex_spacer(2.0)
            .background(theme::BACKGROUND_DARK),
    )
}

fn button_section(
    action_button_name: &str,
    selector: Selector<PlaylistLink>,
    link_extractor: Box<dyn Fn() -> PlaylistLink>,
) -> impl Widget<AppState> {
    let action_button = Button::new(action_button_name)
        .fix_height(theme::grid(5.0))
        .fix_width(theme::grid(9.0))
        .on_click(move |ctx, _, _| {
            ctx.submit_command(selector.with(link_extractor()));
            ctx.window().close();
        });
    let cancel_button = Button::new("Cancel")
        .fix_height(theme::grid(5.0))
        .fix_width(theme::grid(8.0))
        .padding_left(theme::grid(3.0))
        .padding_right(theme::grid(2.0))
        .on_click(|ctx, _, _| ctx.window().close());

    Flex::row()
        .with_child(action_button)
        .with_child(cancel_button)
        .align_right()
}

fn information_section(title_msg: String, description_msg: String) -> impl Widget<AppState> {
    let title_label = Label::new(title_msg)
        .with_text_size(theme::TEXT_SIZE_LARGE)
        .align_left()
        .padding(theme::grid(2.0));

    let description_label = Label::new(description_msg)
        .with_line_break_mode(LineBreaking::WordWrap)
        .with_text_size(theme::TEXT_SIZE_NORMAL)
        .align_left()
        .padding(theme::grid(2.0));

    Flex::column()
        .with_child(title_label)
        .with_child(description_label)
}

pub fn playlist_widget(horizontal: bool) -> impl Widget<WithCtx<Playlist>> {
    let playlist_image_size = if horizontal {
        theme::grid(16.0)
    } else {
        theme::grid(6.0)
    };
    let playlist_image = rounded_cover_widget(playlist_image_size).lens(Ctx::data());

    let playlist_name = Label::raw()
        .with_font(theme::UI_FONT_MEDIUM)
        .with_line_break_mode(LineBreaking::Clip)
        .lens(Ctx::data().then(Playlist::name));

    let playlist_description = Label::raw()
        .with_line_break_mode(LineBreaking::WordWrap)
        .with_text_color(theme::PLACEHOLDER_COLOR)
        .with_text_size(theme::TEXT_SIZE_SMALL)
        .lens(Ctx::data().then(Playlist::description));

    let (playlist_name, playlist_description) = if horizontal {
        (
            playlist_name.fix_width(playlist_image_size).align_left(),
            playlist_description
                .fix_width(playlist_image_size)
                .align_left(),
        )
    } else {
        (
            playlist_name.align_left(),
            playlist_description.align_left(),
        )
    };

    let playlist = if horizontal {
        Flex::column()
            .with_child(playlist_image)
            .with_default_spacer()
            .with_child(
                Flex::column()
                    .with_child(playlist_name)
                    .with_spacer(2.0)
                    .with_child(playlist_description)
                    .align_horizontal(UnitPoint::CENTER)
                    .align_vertical(UnitPoint::TOP)
                    .fix_size(theme::grid(16.0), theme::grid(8.0)),
            )
            .padding(theme::grid(1.0))
    } else {
        Flex::row()
            .with_child(playlist_image)
            .with_default_spacer()
            .with_flex_child(
                Flex::column()
                    .with_child(playlist_name)
                    .with_spacer(2.0)
                    .with_child(playlist_description),
                1.0,
            )
            .padding(theme::grid(1.0))
    };

    playlist
        .link()
        .rounded(theme::BUTTON_BORDER_RADIUS)
        .on_left_click(|ctx, _, playlist, _| {
            ctx.submit_command(cmd::NAVIGATE.with(Nav::PlaylistDetail(playlist.data.link())));
        })
        .context_menu(playlist_menu_ctx)
}

fn cover_widget(size: f64) -> impl Widget<Playlist> {
    RemoteImage::new(
        utils::placeholder_widget(),
        move |playlist: &Playlist, _| playlist.image(size, size).map(|image| image.url.clone()),
    )
    .fix_size(size, size)
}

fn rounded_cover_widget(size: f64) -> impl Widget<Playlist> {
    // TODO: Take the radius from theme.
    cover_widget(size).clip(Size::new(size, size).to_rounded_rect(4.0))
}

pub fn detail_widget() -> impl Widget<AppState> {
    use druid::widget::CrossAxisAlignment;

    let playlist_top = async_playlist_info_widget().padding(theme::grid(1.0));

    let playlist_tracks = async_tracks_widget();

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacer(theme::grid(1.0))
        .with_child(playlist_top)
        .with_spacer(theme::grid(1.0))
        .with_child(playlist_tracks)
}

fn async_playlist_info_widget() -> impl Widget<AppState> {
    Async::new(utils::spinner_widget, playlist_info_widget, || Empty)
        .lens(
            Ctx::make(
                AppState::common_ctx,
                AppState::playlist_detail.then(PlaylistDetail::playlist),
            )
            .then(Ctx::in_promise()),
        )
        .on_command_async(
            LOAD_DETAIL,
            |d| WebApi::global().get_playlist(&d.0.id),
            |_, data, d| data.playlist_detail.playlist.defer(d.0),
            |_, data, (d, r)| data.playlist_detail.playlist.update((d.0, r)),
        )
}

fn playlist_info_widget() -> impl Widget<WithCtx<Playlist>> {
    use druid::widget::CrossAxisAlignment;

    let size = theme::grid(10.0);
    let playlist_cover = cover_widget(size)
        .lens(Ctx::data())
        .clip(Size::new(size, size).to_rounded_rect(4.0))
        .context_menu(playlist_menu_ctx);

    let owner_label = Label::dynamic(|p: &Playlist, _| p.owner.display_name.as_ref().to_string());

    let track_count_label = Label::dynamic(|p: &Playlist, _| {
        let count = p.track_count.unwrap_or(0);
        if count == 1 {
            "1 song".to_string()
        } else {
            format!("{count} songs")
        }
    })
    .with_text_size(theme::TEXT_SIZE_SMALL);

    let description_widget = Either::new(
        |p: &Playlist, _| !p.description.is_empty(),
        Flex::column().with_default_spacer().with_child(
            Label::dynamic(|p: &Playlist, _| p.description.to_string())
                .with_line_break_mode(LineBreaking::Clip)
                .with_text_size(theme::TEXT_SIZE_SMALL)
                .with_text_color(theme::PLACEHOLDER_COLOR),
        ),
        Empty,
    );

    let visibility_widget = Either::new(
        |p: &Playlist, _| p.public.is_some() || p.collaborative,
        Flex::column().with_default_spacer().with_child(
            Label::dynamic(|p: &Playlist, _| {
                let mut parts = Vec::new();
                match p.public {
                    Some(true) => parts.push("Public"),
                    Some(false) => parts.push("Private"),
                    None => {}
                }
                if p.collaborative {
                    parts.push("Collaborative");
                }
                parts.join(" • ")
            })
            .with_line_break_mode(LineBreaking::WordWrap)
            .with_text_size(theme::TEXT_SIZE_SMALL)
            .with_text_color(theme::PLACEHOLDER_COLOR),
        ),
        Empty,
    );

    let playlist_info = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(owner_label)
        .with_default_spacer()
        .with_child(track_count_label)
        .with_child(description_widget)
        .with_child(visibility_widget);

    Flex::row()
        .with_child(playlist_cover)
        .with_default_spacer()
        .with_flex_child(playlist_info.lens(Ctx::data()), 1.0)
}

fn async_tracks_widget() -> impl Widget<AppState> {
    Async::new(utils::spinner_widget, tracks_widget, || {
        utils::retry_error_widget(LOAD_DETAIL)
    })
    .lens(
        Ctx::make(
            AppState::common_ctx,
            AppState::playlist_detail.then(PlaylistDetail::tracks),
        )
        .then(Ctx::in_promise()),
    )
    .on_command_async(
        LOAD_DETAIL,
        |(link, _criteria, _order, enable_paging): (
            PlaylistLink,
            SortCriteria,
            SortOrder,
            bool,
        )| {
            if enable_paging {
                WebApi::global()
                    .get_playlist_tracks_page(&link.id, 0, PAGE_SIZE)
                    .map(|page| PlaylistTracks::from_page(&link, page))
            } else {
                WebApi::global()
                    .get_playlist_tracks_all(&link.id)
                    .map(|tracks| PlaylistTracks::from_full(&link, tracks))
            }
        },
        |_, data, d| data.playlist_detail.tracks.defer(d.clone()),
        |_, data, (def, tracks)| {
            let (ref _link, criteria, order, _) = def;
            let mut tracks = tracks;
            if let Ok(ref mut playlist_tracks) = tracks {
                sort_playlist_tracks(playlist_tracks, criteria, order);
            }
            data.playlist_detail.tracks.update((def, tracks));
        },
    )
    .on_command_async(
        LOAD_MORE_TRACKS,
        |(link, offset): (PlaylistLink, usize)| {
            WebApi::global()
                .get_playlist_tracks_page(&link.id, offset, PAGE_SIZE)
                .map(|page| PlaylistTracksPage {
                    items: page.items,
                    total: page.total,
                    offset: page.offset,
                    limit: page.limit,
                })
        },
        |_, data: &mut AppState, _| {
            if let Promise::Resolved { val, .. } = &mut data.playlist_detail.tracks {
                val.loading_more = true;
            }
        },
        |_, data: &mut AppState, (_, result)| {
            if let Promise::Resolved { val, .. } = &mut data.playlist_detail.tracks {
                val.loading_more = false;
                match result {
                    Ok(page) => {
                        val.tracks.append(page.items);
                        val.total = page.total;
                        val.next_offset = (page.offset + page.limit).min(page.total);
                        sort_playlist_tracks(
                            val,
                            data.config.sort_criteria,
                            data.config.sort_order,
                        );
                    }
                    Err(err) => log::error!("failed to load more tracks: {err}"),
                }
            }
        },
    )
}

fn tracks_widget() -> impl Widget<WithCtx<PlaylistTracks>> {
    let list = playable::list_widget_with_find(
        playable::Display {
            track: track::Display {
                title: true,
                artist: true,
                album: true,
                cover: true,
                ..track::Display::empty()
            },
        },
        cmd::FIND_IN_PLAYLIST,
    );

    let load_more = Flex::row()
        .with_child(
            ViewSwitcher::new(
                |tracks: &WithCtx<PlaylistTracks>, _| {
                    let searching = !tracks.ctx.library_search.trim().is_empty();
                    if searching {
                        (false, false)
                    } else {
                        (tracks.data.loading_more, tracks.data.has_more())
                    }
                },
                |state, _tracks: &WithCtx<PlaylistTracks>, _| match state {
                    (true, _) => Spinner::new().boxed(),
                    (false, true) => Button::new("Load more")
                        .on_left_click(|ctx, _, tracks: &mut WithCtx<PlaylistTracks>, _| {
                            let link = tracks.data.link();
                            let offset = tracks.data.next_offset;
                            ctx.submit_command(LOAD_MORE_TRACKS.with((link, offset)));
                        })
                        .boxed(),
                    _ => Empty.boxed(),
                },
            )
            .padding((0.0, theme::grid(1.0))),
        )
        .align_left();

    Flex::column().with_child(list).with_child(load_more)
}

fn playlist_menu_ctx(playlist: &WithCtx<Playlist>) -> Menu<AppState> {
    let library = &playlist.ctx.library;
    let playlist = &playlist.data;

    let mut menu = Menu::empty();

    menu = menu.entry(
        MenuItem::new(
            LocalizedString::new("menu-item-copy-link").with_placeholder("Copy Link to Playlist"),
        )
        .command(cmd::COPY.with(playlist.url())),
    );

    menu = menu.entry(
        MenuItem::new(LocalizedString::new("menu-item-play-next").with_placeholder("Play Next"))
            .command(cmd::QUEUE_INSERT_PLAYLIST.with(cmd::QueuePlaylistRequest {
                link: playlist.link(),
                mode: cmd::QueueInsertMode::Next,
            })),
    );
    menu = menu.entry(
        MenuItem::new(
            LocalizedString::new("menu-item-add-to-queue")
                .with_placeholder("Add Playlist to Queue"),
        )
        .command(cmd::QUEUE_INSERT_PLAYLIST.with(cmd::QueuePlaylistRequest {
            link: playlist.link(),
            mode: cmd::QueueInsertMode::End,
        })),
    );

    if library.contains_playlist(playlist) {
        let created_by_user = library.is_created_by_user(playlist);

        if created_by_user {
            let unfollow_msg = UnfollowPlaylist {
                link: playlist.link(),
                created_by_user,
            };
            menu = menu.entry(
                MenuItem::new(
                    LocalizedString::new("menu-unfollow-playlist")
                        .with_placeholder("Delete playlist"),
                )
                .command(SHOW_UNFOLLOW_PLAYLIST_CONFIRM.with(unfollow_msg)),
            );
            menu = menu.entry(
                MenuItem::new(
                    LocalizedString::new("menu-rename-playlist")
                        .with_placeholder("Rename playlist"),
                )
                .command(SHOW_RENAME_PLAYLIST_CONFIRM.with(playlist.link())),
            );
        } else {
            let unfollow_msg = UnfollowPlaylist {
                link: playlist.link(),
                created_by_user,
            };
            menu = menu.entry(
                MenuItem::new(
                    LocalizedString::new("menu-unfollow-playlist")
                        .with_placeholder("Remove playlist from Your Library"),
                )
                .command(SHOW_UNFOLLOW_PLAYLIST_CONFIRM.with(unfollow_msg)),
            );
        }
    } else {
        menu = menu.entry(
            MenuItem::new(
                LocalizedString::new("menu-follow-playlist").with_placeholder("Follow Playlist"),
            )
            .command(FOLLOW_PLAYLIST.with(playlist.clone())),
        );
    }

    menu
}

#[derive(Clone)]
struct UnfollowPlaylist {
    link: PlaylistLink,
    created_by_user: bool,
}
