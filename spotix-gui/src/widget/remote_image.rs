use std::{sync::Arc, time::Duration};

use druid::{
    Color, Data, ImageBuf, Point, Selector, TimerToken, WidgetPod,
    widget::{FillStrat, Image, prelude::*},
};

use crate::webapi::WebApi;

pub const REQUEST_DATA: Selector<Arc<str>> = Selector::new("remote-image.request-data");
pub const PROVIDE_DATA: Selector<ImagePayload> = Selector::new("remote-image.provide-data");

/// Duration of the fade-in animation when an image arrives.
const FADE_DURATION_SECS: f64 = 0.2;

#[derive(Clone)]
pub struct ImagePayload {
    pub location: Arc<str>,
    pub image_buf: ImageBuf,
}

pub struct RemoteImage<T> {
    placeholder: WidgetPod<T, Box<dyn Widget<T>>>,
    image: Option<WidgetPod<T, Image>>,
    locator: Box<dyn Fn(&T, &Env) -> Option<Arc<str>>>,
    location: Option<Arc<str>>,
    request_timer: Option<TimerToken>,
    pending_request: Option<Arc<str>>,
    /// 0.0 = image just arrived, 1.0 = fully visible.
    fade_progress: f64,
    /// Whether we're currently animating a fade-in.
    fading: bool,
}

impl<T: Data> RemoteImage<T> {
    pub fn new(
        placeholder: impl Widget<T> + 'static,
        locator: impl Fn(&T, &Env) -> Option<Arc<str>> + 'static,
    ) -> Self {
        Self {
            placeholder: WidgetPod::new(placeholder).boxed(),
            locator: Box::new(locator),
            location: None,
            image: None,
            request_timer: None,
            pending_request: None,
            fade_progress: 1.0,
            fading: false,
        }
    }
}

impl<T: Data> Widget<T> for RemoteImage<T> {
    fn event(&mut self, ctx: &mut EventCtx, event: &Event, data: &mut T, env: &Env) {
        if let Event::Command(cmd) = event
            && let Some(payload) = cmd.get(PROVIDE_DATA)
        {
            if Some(&payload.location) == self.location.as_ref() {
                self.image.replace(WidgetPod::new(
                    Image::new(payload.image_buf.clone()).fill_mode(FillStrat::Cover),
                ));
                self.fade_progress = 0.0;
                self.fading = true;
                ctx.children_changed();
                ctx.request_anim_frame();
            }
            return;
        }
        if let Event::AnimFrame(interval) = event {
            if self.fading {
                self.fade_progress += (*interval as f64) * 1e-9 / FADE_DURATION_SECS;
                if self.fade_progress >= 1.0 {
                    self.fade_progress = 1.0;
                    self.fading = false;
                } else {
                    ctx.request_anim_frame();
                }
                ctx.request_paint();
            }
            // Forward to children (placeholder needs AnimFrame for skeleton pulse)
            if self.image.is_none() {
                self.placeholder.event(ctx, event, data, env);
            }
            return;
        }
        if let Event::Timer(token) = event
            && self.request_timer == Some(*token)
        {
            self.request_timer = None;
            if let Some(delay) = WebApi::global().rate_limit_delay() {
                self.request_timer = Some(ctx.request_timer(delay));
                return;
            }
            if let Some(location) = self.pending_request.take()
                && Some(&location) == self.location.as_ref()
            {
                ctx.submit_command(REQUEST_DATA.with(location).to(ctx.widget_id()));
            }
            return;
        }
        if let Some(image) = self.image.as_mut() {
            image.event(ctx, event, data, env);
        } else {
            self.placeholder.event(ctx, event, data, env);
        }
    }

    fn lifecycle(&mut self, ctx: &mut LifeCycleCtx, event: &LifeCycle, data: &T, env: &Env) {
        if let LifeCycle::WidgetAdded = event {
            let location = (self.locator)(data, env);
            self.image = None;
            self.fade_progress = 1.0;
            self.fading = false;
            self.location.clone_from(&location);
            self.pending_request = location;
            if let Some(loc) = &self.pending_request {
                // Fast path: if the image is already in the in-memory cache,
                // use it immediately -- no timer, no placeholder flash.
                if let Some(image_buf) = WebApi::global().get_cached_image(loc) {
                    self.image.replace(WidgetPod::new(
                        Image::new(image_buf).fill_mode(FillStrat::Cover),
                    ));
                    self.pending_request = None;
                } else {
                    let delay = WebApi::global()
                        .rate_limit_delay()
                        .unwrap_or(Duration::from_millis(250));
                    self.request_timer = Some(ctx.request_timer(delay));
                }
            }
        }
        if let Some(image) = self.image.as_mut() {
            image.lifecycle(ctx, event, data, env);
        }
        self.placeholder.lifecycle(ctx, event, data, env);
    }

    fn update(&mut self, ctx: &mut UpdateCtx, _old_data: &T, data: &T, env: &Env) {
        let location = (self.locator)(data, env);
        if location != self.location {
            self.location.clone_from(&location);
            self.pending_request = location;

            // Fast path: check in-memory image cache before going through
            // the timer + delegate round-trip.
            if let Some(loc) = &self.pending_request
                && let Some(image_buf) = WebApi::global().get_cached_image(loc)
            {
                self.image.replace(WidgetPod::new(
                    Image::new(image_buf).fill_mode(FillStrat::Cover),
                ));
                self.pending_request = None;
                self.fade_progress = 1.0;
                self.fading = false;
                self.request_timer = None;
                ctx.children_changed();
                if let Some(image) = self.image.as_mut() {
                    image.update(ctx, data, env);
                }
                self.placeholder.update(ctx, data, env);
                return;
            }

            // Slow path: image not cached, show placeholder and start timer.
            self.image = None;
            self.fade_progress = 1.0;
            self.fading = false;
            if self.pending_request.is_some() {
                let delay = WebApi::global()
                    .rate_limit_delay()
                    .unwrap_or(Duration::from_millis(250));
                self.request_timer = Some(ctx.request_timer(delay));
            } else {
                self.request_timer = None;
            }
            ctx.children_changed();
        }
        if self.request_timer.is_none() && self.pending_request.is_some() {
            let delay = WebApi::global()
                .rate_limit_delay()
                .unwrap_or(Duration::from_millis(250));
            self.request_timer = Some(ctx.request_timer(delay));
        }
        if let Some(image) = self.image.as_mut() {
            image.update(ctx, data, env);
        }
        self.placeholder.update(ctx, data, env);
    }

    fn layout(&mut self, ctx: &mut LayoutCtx, bc: &BoxConstraints, data: &T, env: &Env) -> Size {
        let size = if let Some(image) = self.image.as_mut() {
            let size = image.layout(ctx, bc, data, env);
            image.set_origin(ctx, Point::ORIGIN);
            size
        } else {
            bc.max()
        };
        let placeholder_bc = BoxConstraints::tight(size);
        self.placeholder.layout(ctx, &placeholder_bc, data, env);
        self.placeholder.set_origin(ctx, Point::ORIGIN);
        size
    }

    fn paint(&mut self, ctx: &mut PaintCtx, data: &T, env: &Env) {
        if let Some(image) = self.image.as_mut() {
            if self.fade_progress >= 1.0 {
                // Fully loaded, just paint the image.
                image.paint(ctx, data, env);
            } else {
                // Fade in: paint the image, then overlay a semi-transparent
                // scrim that matches the skeleton color. As fade_progress goes
                // from 0→1 the scrim fades from opaque to transparent, revealing
                // the image underneath.
                image.paint(ctx, data, env);
                let bg = env.get(crate::ui::theme::GREY_600);
                let (r, g, b, _) = bg.as_rgba();
                // Smooth ease-out curve for a natural reveal
                let t = 1.0 - (1.0 - self.fade_progress).powi(2);
                let scrim = Color::rgba(r, g, b, 1.0 - t);
                let rect = ctx.size().to_rect();
                ctx.fill(rect, &scrim);
            }
        } else {
            self.placeholder.paint(ctx, data, env);
        }
    }
}
