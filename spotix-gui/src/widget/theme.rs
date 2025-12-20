use std::sync::atomic::{AtomicBool, Ordering};

use crate::{data::AppState, ui::theme};
use druid::widget::prelude::*;

static FONTS_LOADED: AtomicBool = AtomicBool::new(false);

pub struct ThemeScope<W> {
    inner: W,
    cached_env: Option<Env>,
}

impl<W> ThemeScope<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            cached_env: None,
        }
    }

    fn set_env(&mut self, data: &AppState, outer_env: &Env) {
        let mut themed_env = outer_env.clone();
        theme::setup(&mut themed_env, data);
        self.cached_env.replace(themed_env);
    }

    fn load_fonts_once(&mut self, ctx: &mut LifeCycleCtx) {
        if FONTS_LOADED
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            theme::load_spotify_fonts(ctx.text());
        }
    }
}

impl<W: Widget<AppState>> Widget<AppState> for ThemeScope<W> {
    fn event(&mut self, ctx: &mut EventCtx, event: &Event, data: &mut AppState, env: &Env) {
        self.inner
            .event(ctx, event, data, self.cached_env.as_ref().unwrap_or(env))
    }

    fn lifecycle(&mut self, ctx: &mut LifeCycleCtx, event: &LifeCycle, data: &AppState, env: &Env) {
        if let LifeCycle::WidgetAdded = &event {
            self.load_fonts_once(ctx);
            self.set_env(data, env);
        }
        self.inner
            .lifecycle(ctx, event, data, self.cached_env.as_ref().unwrap_or(env))
    }

    fn update(&mut self, ctx: &mut UpdateCtx, old_data: &AppState, data: &AppState, env: &Env) {
        if !data.config.theme.same(&old_data.config.theme) {
            self.set_env(data, env);
            ctx.request_layout();
            ctx.request_paint();
        }
        self.inner
            .update(ctx, old_data, data, self.cached_env.as_ref().unwrap_or(env));
    }

    fn layout(
        &mut self,
        ctx: &mut LayoutCtx,
        bc: &BoxConstraints,
        data: &AppState,
        env: &Env,
    ) -> Size {
        self.inner
            .layout(ctx, bc, data, self.cached_env.as_ref().unwrap_or(env))
    }

    fn paint(&mut self, ctx: &mut PaintCtx, data: &AppState, env: &Env) {
        self.inner
            .paint(ctx, data, self.cached_env.as_ref().unwrap_or(env));
    }
}
