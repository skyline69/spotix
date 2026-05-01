use crate::{
    cmd,
    data::{AppState, Nav, PromiseState, SpotifyUrl},
    ui::{album, artist, home, library, lyrics, playlist, recommend, search, show},
};
use std::time::Duration;

use druid::widget::{Controller, prelude::*};
use druid::{Code, Target, TimerToken};

#[derive(Default)]
pub struct NavController {
    scroll_timer: Option<TimerToken>,
    rate_limit_timer: Option<TimerToken>,
}

impl NavController {
    fn load_route_data(&mut self, ctx: &mut EventCtx, data: &mut AppState) {
        let _ = matches!(
            &data.nav,
            Nav::Home
                | Nav::SavedTracks
                | Nav::SavedAlbums
                | Nav::Shows
                | Nav::SearchResults(_)
                | Nav::AlbumDetail(_, _)
                | Nav::ArtistDetail(_)
                | Nav::PlaylistDetail(_)
                | Nav::ShowDetail(_)
                | Nav::Recommendations(_)
        );
        match &data.nav {
            Nav::Home => {
                if data.home_detail.made_for_you.state() == PromiseState::Empty {
                    ctx.submit_command(home::LOAD_MADE_FOR_YOU);
                }
                if data.home_detail.user_top_artists.state() == PromiseState::Empty {
                    ctx.submit_command(home::LOAD_USER_TOP_ARTISTS);
                }
                if data.home_detail.user_top_tracks.state() == PromiseState::Empty {
                    ctx.submit_command(home::LOAD_USER_TOP_TRACKS);
                }
            }
            Nav::Lyrics => {}
            Nav::SavedTracks => {
                if data.library.saved_tracks.state() == PromiseState::Empty {
                    ctx.submit_command(library::LOAD_TRACKS);
                }
            }
            Nav::SavedAlbums => {
                if data.library.saved_albums.state() == PromiseState::Empty {
                    ctx.submit_command(library::LOAD_ALBUMS);
                }
            }
            Nav::Shows => {
                if data.library.saved_shows.state() == PromiseState::Empty {
                    ctx.submit_command(library::LOAD_SHOWS);
                }
            }
            Nav::SearchResults(query) => {
                if let Some(link) = SpotifyUrl::parse(query) {
                    ctx.submit_command(search::OPEN_LINK.with(link));
                } else if data.search.results.deferred()
                    != Some(&(query.clone(), data.search.topic))
                {
                    ctx.submit_command(
                        search::LOAD_RESULTS.with((query.to_owned(), data.search.topic)),
                    );
                }
            }
            Nav::AlbumDetail(link, _) => {
                if data.album_detail.album.deferred() != Some(link) {
                    ctx.submit_command(album::LOAD_DETAIL.with(link.to_owned()));
                }
            }
            Nav::ArtistDetail(link) => {
                if data.artist_detail.top_tracks.deferred() != Some(link) {
                    ctx.submit_command(artist::LOAD_DETAIL.with(link.to_owned()));
                }
            }
            Nav::PlaylistDetail(link) => {
                if data.playlist_detail.playlist.deferred() != Some(link) {
                    ctx.submit_command(playlist::LOAD_DETAIL.with((
                        link.to_owned(),
                        data.config.sort_criteria,
                        data.config.sort_order,
                        data.config.enable_pagination,
                    )));
                }
            }
            Nav::ShowDetail(link) => {
                if data.show_detail.show.deferred() != Some(link) {
                    ctx.submit_command(show::LOAD_DETAIL.with(link.to_owned()));
                }
            }
            Nav::Recommendations(request) => {
                if data.recommend.results.deferred() != Some(request) {
                    ctx.submit_command(recommend::LOAD_RESULTS.with(request.clone()));
                }
            }
        }
    }
}

impl<W> Controller<AppState, W> for NavController
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
            Event::Timer(token) if self.scroll_timer == Some(*token) => {
                self.scroll_timer = None;
                ctx.submit_command(lyrics::SCROLL_ACTIVE_LYRIC.to(Target::Window(ctx.window_id())));
                ctx.set_handled();
            }
            Event::Timer(token) if self.rate_limit_timer == Some(*token) => {
                self.rate_limit_timer = None;
                self.load_route_data(ctx, data);
                ctx.set_handled();
            }
            Event::Command(cmd) if cmd.is(cmd::NAVIGATE) => {
                let nav = cmd.get_unchecked(cmd::NAVIGATE);
                data.navigate(nav);
                ctx.set_handled();
                self.load_route_data(ctx, data);
            }
            Event::Command(cmd) if cmd.is(cmd::NAVIGATE_BACK) => {
                let count = cmd.get_unchecked(cmd::NAVIGATE_BACK);
                for _ in 0..*count {
                    data.navigate_back();
                }
                ctx.set_handled();
                self.load_route_data(ctx, data);
            }
            Event::Command(cmd) if cmd.is(cmd::NAVIGATE_REFRESH) => {
                data.refresh_playlist();
                ctx.set_handled();
                self.load_route_data(ctx, data);
            }
            Event::Command(cmd) if cmd.is(cmd::TOGGLE_LYRICS) => {
                match data.nav {
                    Nav::Lyrics => data.navigate_back(),
                    _ => {
                        data.navigate(&Nav::Lyrics);
                        if let Some(np) = data.playback.now_playing.as_ref() {
                            ctx.submit_command(lyrics::SHOW_LYRICS.with(np.clone()));
                        }
                        self.scroll_timer = Some(ctx.request_timer(Duration::from_millis(50)));
                    }
                }
                ctx.set_handled();
                self.load_route_data(ctx, data);
            }
            Event::MouseDown(cmd) if cmd.button.is_x1() => {
                data.navigate_back();
                ctx.set_handled();
                self.load_route_data(ctx, data);
            }
            Event::KeyDown(key) if key.mods.ctrl() && key.code == Code::KeyR => {
                data.refresh_all();
                ctx.set_handled();
                self.load_route_data(ctx, data);
            }
            _ => {
                child.event(ctx, event, data, env);
            }
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
        if let LifeCycle::WidgetAdded = event {
            // Load the last route, or the default.
            ctx.submit_command(
                cmd::NAVIGATE.with(data.config.last_route.to_owned().unwrap_or_default()),
            );
        }
        child.lifecycle(ctx, event, data, env)
    }
}
