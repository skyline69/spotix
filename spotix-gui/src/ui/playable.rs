use std::{f64::consts::PI, mem, sync::Arc};

use druid::{
    Data, Env, Event, EventCtx, Lens, RenderContext, Selector, Size, Widget, WidgetExt,
    im::Vector,
    kurbo::Rect,
    lens::Map,
    widget::{Controller, ControllerHost, List, ListIter, ViewSwitcher, prelude::*},
};

use crate::{
    cmd,
    data::{
        ArtistTracks, CommonCtx, FindQuery, MatchFindQuery, Nav, Playable, PlaybackOrigin,
        PlaybackPayload, PlaylistTracks, Recommendations, SavedTracks, SearchResults, ShowEpisodes,
        Track, WithCtx,
    },
    ui::theme,
};

use super::{
    episode,
    find::{Find, Findable},
    track,
};

#[derive(Copy, Clone)]
pub struct Display {
    pub track: track::Display,
}

pub fn list_widget<T>(display: Display) -> impl Widget<WithCtx<T>>
where
    T: PlayableIter + Data,
{
    ControllerHost::new(List::new(move || playable_widget(display)), PlayController)
}

pub fn list_widget_with_find<T>(
    display: Display,
    selector: Selector<Find>,
) -> impl Widget<WithCtx<T>>
where
    T: PlayableIter + Data,
{
    ControllerHost::new(
        List::new(move || Findable::new(playable_widget(display), selector)),
        PlayController,
    )
}

fn playable_widget(display: Display) -> impl Widget<PlayRow<Playable>> {
    ViewSwitcher::new(
        |row: &PlayRow<Playable>, _| mem::discriminant(&row.item),
        move |_, row: &PlayRow<Playable>, _| match row.item.clone() {
            // TODO: Do the lenses some other way.
            Playable::Track(track) => {
                let track_item = track.clone();
                track::playable_widget(track.clone(), display.track)
                    .lens(Map::new(
                        move |pb: &PlayRow<Playable>| pb.with(track_item.clone()),
                        |_, _| {
                            // Ignore mutation.
                        },
                    ))
                    .boxed()
            }
            Playable::Episode(episode) => {
                episode::playable_widget()
                    .lens(Map::new(
                        move |pb: &PlayRow<Playable>| pb.with(episode.clone()),
                        |_, _| {
                            // Ignore mutation.
                        },
                    ))
                    .boxed()
            }
        },
    )
}

/// State for the playback indicator shown on the currently playing item.
#[derive(Clone, Copy, Debug, PartialEq, Data)]
pub enum PlaybackMarker {
    /// Not the current item.
    Inactive,
    /// Current item but playback is paused.
    Paused,
    /// Current item and audio is actively playing.
    Playing,
}

/// A Spotify-style animated 3-bar equalizer indicator.
/// Shows animated bars when `Playing`, frozen short bars when `Paused`,
/// and nothing when `Inactive`.
pub struct PlaybackIndicator {
    t: f64,
}

impl PlaybackIndicator {
    pub fn new() -> Self {
        Self { t: 0.0 }
    }
}

impl Widget<PlaybackMarker> for PlaybackIndicator {
    fn event(&mut self, ctx: &mut EventCtx, event: &Event, data: &mut PlaybackMarker, _env: &Env) {
        if let Event::AnimFrame(interval) = event {
            if *data == PlaybackMarker::Playing {
                self.t += (*interval as f64) * 1e-9;
                // Wrap at a large common period so the animation never visibly resets.
                // LCM-friendly period for all three bar frequencies.
                if self.t >= 60.0 {
                    self.t -= 60.0;
                }
                ctx.request_anim_frame();
            }
            ctx.request_paint();
        }
    }

    fn lifecycle(
        &mut self,
        ctx: &mut LifeCycleCtx,
        event: &LifeCycle,
        data: &PlaybackMarker,
        _env: &Env,
    ) {
        if let LifeCycle::WidgetAdded = event
            && *data == PlaybackMarker::Playing
        {
            ctx.request_anim_frame();
        }
    }

    fn update(
        &mut self,
        ctx: &mut UpdateCtx,
        old_data: &PlaybackMarker,
        data: &PlaybackMarker,
        _env: &Env,
    ) {
        if old_data != data {
            if *data == PlaybackMarker::Playing {
                ctx.request_anim_frame();
            }
            ctx.request_paint();
        }
    }

    fn layout(
        &mut self,
        _ctx: &mut LayoutCtx,
        _bc: &BoxConstraints,
        _data: &PlaybackMarker,
        _env: &Env,
    ) -> Size {
        Size::new(14.0, 14.0)
    }

    fn paint(&mut self, ctx: &mut PaintCtx, data: &PlaybackMarker, env: &Env) {
        if *data == PlaybackMarker::Inactive {
            return;
        }

        let size = ctx.size();
        let bar_width = 2.5;
        let gap = 1.5;
        let total_width = bar_width * 3.0 + gap * 2.0;
        let x_offset = (size.width - total_width) / 2.0;
        let max_height = size.height * 0.85;
        let min_height = size.height * 0.15;

        let color = env.get(theme::BLUE_200);

        // Each bar has its own frequency and phase for an organic feel.
        // Using irrational-ish frequency ratios so they never align and
        // the animation looks endlessly varied.
        const BAR_PARAMS: [(f64, f64, f64); 3] = [
            // (frequency_hz, phase_offset, secondary_blend)
            (1.2, 0.0, 0.3),
            (1.5, 2.1, 0.25),
            (1.0, 4.3, 0.35),
        ];

        for i in 0..3 {
            let x = x_offset + (i as f64) * (bar_width + gap);

            let bar_height = if *data == PlaybackMarker::Playing {
                let (freq, phase, blend) = BAR_PARAMS[i];
                // Primary wave
                let primary = ((self.t * freq * 2.0 * PI + phase).sin() + 1.0) / 2.0;
                // Secondary harmonic at ~1.7x frequency for variation
                let secondary = ((self.t * freq * 1.7 * 2.0 * PI + phase * 0.6).sin() + 1.0) / 2.0;
                let wave = primary * (1.0 - blend) + secondary * blend;
                min_height + wave * (max_height - min_height)
            } else {
                // Paused: frozen short bars at different heights
                let heights = [0.35, 0.55, 0.25];
                min_height + heights[i] * (max_height - min_height)
            };

            let y = size.height - bar_height;
            let rect = Rect::new(x, y, x + bar_width, size.height);
            let rounded = rect.to_rounded_rect(1.0);
            ctx.fill(rounded, &color);
        }
    }
}

#[derive(Clone, Data, Lens)]
pub struct PlayRow<T> {
    pub item: T,
    pub ctx: Arc<CommonCtx>,
    pub origin: Arc<PlaybackOrigin>,
    pub position: usize,
    pub playback_marker: PlaybackMarker,
    pub selection_enabled: bool,
    pub selected: bool,
}

impl<T> PlayRow<T> {
    fn with<U>(&self, item: U) -> PlayRow<U> {
        PlayRow {
            item,
            ctx: self.ctx.clone(),
            origin: self.origin.clone(),
            position: self.position,
            playback_marker: self.playback_marker,
            selection_enabled: self.selection_enabled,
            selected: self.selected,
        }
    }

    /// Legacy accessor for backward compat.
    pub fn is_playing(&self) -> bool {
        self.playback_marker != PlaybackMarker::Inactive
    }
}

impl MatchFindQuery for PlayRow<Playable> {
    fn matches_query(&self, q: &FindQuery) -> bool {
        match &self.item {
            Playable::Track(track) => {
                q.matches_str(&track.name)
                    || track.album.iter().any(|a| q.matches_str(&a.name))
                    || track.artists.iter().any(|a| q.matches_str(&a.name))
            }
            Playable::Episode(episode) => {
                q.matches_str(&episode.name)
                    || q.matches_str(&episode.description)
                    || q.matches_str(&episode.show.name)
            }
        }
    }
}

pub trait PlayableIter {
    fn origin(&self) -> PlaybackOrigin;
    fn count(&self) -> usize;
    fn for_each(&self, cb: impl FnMut(Playable, usize));
    fn selection_enabled(&self) -> bool {
        false
    }
    fn is_selected(&self, _item: &Playable) -> bool {
        false
    }
}

// This should change to a more specific name as it could be confusing for others
// As at the moment this is only used for the home page!
impl PlayableIter for Vector<Arc<Track>> {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Home
    }

    fn for_each(&self, mut cb: impl FnMut(Playable, usize)) {
        for (position, track) in self.iter().enumerate() {
            cb(Playable::Track(track.to_owned()), position);
        }
    }

    fn count(&self) -> usize {
        self.len()
    }
}

impl PlayableIter for PlaylistTracks {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Playlist(self.link())
    }

    fn for_each(&self, mut cb: impl FnMut(Playable, usize)) {
        for (position, track) in self.tracks.iter().enumerate() {
            cb(Playable::Track(track.to_owned()), position);
        }
    }

    fn count(&self) -> usize {
        self.tracks.len()
    }

    fn selection_enabled(&self) -> bool {
        self.selection_mode
    }

    fn is_selected(&self, item: &Playable) -> bool {
        match item {
            Playable::Track(track) => self.selected_positions.contains(&track.track_pos),
            _ => false,
        }
    }
}

impl PlayableIter for ArtistTracks {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Artist(self.link())
    }

    fn for_each(&self, mut cb: impl FnMut(Playable, usize)) {
        for (position, track) in self.tracks.iter().enumerate() {
            cb(Playable::Track(track.to_owned()), position);
        }
    }

    fn count(&self) -> usize {
        self.tracks.len()
    }
}

impl PlayableIter for SavedTracks {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Library
    }

    fn for_each(&self, mut cb: impl FnMut(Playable, usize)) {
        for (position, track) in self.tracks.iter().enumerate() {
            cb(Playable::Track(track.to_owned()), position);
        }
    }

    fn count(&self) -> usize {
        self.tracks.len()
    }
}

impl PlayableIter for SearchResults {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Search(self.query.clone())
    }

    fn for_each(&self, mut cb: impl FnMut(Playable, usize)) {
        for (position, track) in self.tracks.iter().enumerate() {
            cb(Playable::Track(track.to_owned()), position);
        }
    }

    fn count(&self) -> usize {
        self.tracks.len()
    }
}

impl PlayableIter for Recommendations {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Recommendations(self.request.clone())
    }

    fn for_each(&self, mut cb: impl FnMut(Playable, usize)) {
        for (position, track) in self.tracks.iter().enumerate() {
            cb(Playable::Track(track.to_owned()), position);
        }
    }

    fn count(&self) -> usize {
        self.tracks.len()
    }
}

impl PlayableIter for ShowEpisodes {
    fn origin(&self) -> PlaybackOrigin {
        PlaybackOrigin::Show(self.show.clone())
    }

    fn for_each(&self, mut cb: impl FnMut(Playable, usize)) {
        for (position, episode) in self.episodes.iter().enumerate() {
            cb(Playable::Episode(episode.to_owned()), position);
        }
    }

    fn count(&self) -> usize {
        self.episodes.len()
    }
}

impl<T> ListIter<PlayRow<Playable>> for WithCtx<T>
where
    T: PlayableIter + Data,
{
    fn for_each(&self, mut cb: impl FnMut(&PlayRow<Playable>, usize)) {
        let origin = Arc::new(self.data.origin());
        let filter = filter_query(self.ctx.as_ref());
        let mut position = 0;
        self.data.for_each(|item, _| {
            if let Some(query) = filter.as_deref()
                && !playable_matches_query(&item, query)
            {
                return;
            }
            let selected = self.data.is_selected(&item);
            cb(
                &PlayRow {
                    playback_marker: self.ctx.playback_marker(&item),
                    ctx: self.ctx.to_owned(),
                    origin: origin.clone(),
                    item,
                    position,
                    selection_enabled: self.data.selection_enabled(),
                    selected,
                },
                position,
            );
            position += 1;
        });
    }

    fn for_each_mut(&mut self, mut cb: impl FnMut(&mut PlayRow<Playable>, usize)) {
        let origin = Arc::new(self.data.origin());
        let filter = filter_query(self.ctx.as_ref());
        let mut position = 0;
        self.data.for_each(|item, _| {
            if let Some(query) = filter.as_deref()
                && !playable_matches_query(&item, query)
            {
                return;
            }
            let selected = self.data.is_selected(&item);
            cb(
                &mut PlayRow {
                    playback_marker: self.ctx.playback_marker(&item),
                    ctx: self.ctx.to_owned(),
                    origin: origin.clone(),
                    item,
                    position,
                    selection_enabled: self.data.selection_enabled(),
                    selected,
                },
                position,
            );
            position += 1;
        });
    }

    fn data_len(&self) -> usize {
        let filter = filter_query(self.ctx.as_ref());
        if let Some(query) = filter {
            let mut count = 0;
            self.data.for_each(|item, _| {
                if playable_matches_query(&item, &query) {
                    count += 1;
                }
            });
            count
        } else {
            self.data.count()
        }
    }
}

struct PlayController;

impl<T, W> Controller<WithCtx<T>, W> for PlayController
where
    T: PlayableIter + Data,
    W: Widget<WithCtx<T>>,
{
    fn event(
        &mut self,
        child: &mut W,
        ctx: &mut EventCtx,
        event: &Event,
        data: &mut WithCtx<T>,
        env: &Env,
    ) {
        match event {
            Event::Notification(note) => {
                if let Some(position) = note.get(cmd::PLAY) {
                    let items = filtered_items(&data.ctx, &data.data);
                    let payload = PlaybackPayload {
                        items,
                        origin: data.data.origin(),
                        position: position.to_owned(),
                    };
                    ctx.submit_command(cmd::PLAY_TRACKS.with(payload));
                    ctx.set_handled();
                }
            }
            _ => child.event(ctx, event, data, env),
        }
    }
}

fn filter_query(ctx: &CommonCtx) -> Option<String> {
    let query = ctx.library_search.trim();
    if query.is_empty() {
        return None;
    }
    if matches!(ctx.nav, Nav::PlaylistDetail(_) | Nav::SavedTracks) {
        Some(query.to_lowercase())
    } else {
        None
    }
}

fn playable_matches_query(item: &Playable, query: &str) -> bool {
    fn contains(haystack: &str, needle: &str) -> bool {
        haystack.to_lowercase().contains(needle)
    }

    match item {
        Playable::Track(track) => {
            contains(&track.name, query)
                || track
                    .album
                    .as_ref()
                    .is_some_and(|a| contains(&a.name, query))
                || track.artists.iter().any(|a| contains(&a.name, query))
        }
        Playable::Episode(episode) => {
            contains(&episode.name, query)
                || contains(&episode.description, query)
                || contains(&episode.show.name, query)
        }
    }
}

fn filtered_items<T: PlayableIter>(ctx: &Arc<CommonCtx>, data: &T) -> Vector<Playable> {
    let mut items = Vector::new();
    let filter = filter_query(ctx.as_ref());
    data.for_each(|item, _| {
        if let Some(ref query) = filter
            && !playable_matches_query(&item, query)
        {
            return;
        }
        items.push_back(item);
    });
    items
}
