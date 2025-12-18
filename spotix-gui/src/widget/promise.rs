use druid::{Data, Point, WidgetExt, WidgetPod, widget::prelude::*};

use crate::data::{Promise, PromiseState};

#[derive(Clone, Data)]
pub struct PromiseError<E: Data, D: Data> {
    pub err: E,
    pub def: D,
}

pub struct Async<T, D: Data + Clone, E: Data + Clone> {
    def_maker: Box<dyn Fn() -> Box<dyn Widget<D>>>,
    res_maker: Box<dyn Fn() -> Box<dyn Widget<T>>>,
    err_maker: Box<dyn Fn() -> Box<dyn Widget<PromiseError<E, D>>>>,
    widget: PromiseWidget<T, D, E>,
}

#[allow(clippy::large_enum_variant)]
enum PromiseWidget<T, D: Data + Clone, E: Data + Clone> {
    Empty,
    Deferred(WidgetPod<D, Box<dyn Widget<D>>>),
    Resolved(WidgetPod<T, Box<dyn Widget<T>>>),
    Rejected(WidgetPod<PromiseError<E, D>, Box<dyn Widget<PromiseError<E, D>>>>),
}

impl<D: Data + Clone, T: Data, E: Data + Clone> Async<T, D, E> {
    pub fn new<WD, WT, WE>(
        def_maker: impl Fn() -> WD + 'static,
        res_maker: impl Fn() -> WT + 'static,
        err_maker: impl Fn() -> WE + 'static,
    ) -> Self
    where
        WD: Widget<D> + 'static,
        WT: Widget<T> + 'static,
        WE: Widget<PromiseError<E, D>> + 'static,
    {
        Self {
            def_maker: Box::new(move || def_maker().boxed()),
            res_maker: Box::new(move || res_maker().boxed()),
            err_maker: Box::new(move || err_maker().boxed()),
            widget: PromiseWidget::Empty,
        }
    }

    fn rebuild_widget(&mut self, state: PromiseState) {
        self.widget = match state {
            PromiseState::Empty => PromiseWidget::Empty,
            PromiseState::Deferred => PromiseWidget::Deferred(WidgetPod::new((self.def_maker)())),
            PromiseState::Resolved => PromiseWidget::Resolved(WidgetPod::new((self.res_maker)())),
            PromiseState::Rejected => PromiseWidget::Rejected(WidgetPod::new((self.err_maker)())),
        };
    }
}

impl<D: Data + Clone, T: Data, E: Data + Clone> Widget<Promise<T, D, E>> for Async<T, D, E> {
    fn event(&mut self, ctx: &mut EventCtx, event: &Event, data: &mut Promise<T, D, E>, env: &Env) {
        if data.state() == self.widget.state() {
            match data {
                Promise::Empty => {}
                Promise::Deferred { def } => {
                    self.widget.with_deferred(|w| w.event(ctx, event, def, env));
                }
                Promise::Resolved { val, .. } => {
                    self.widget.with_resolved(|w| w.event(ctx, event, val, env));
                }
                Promise::Rejected { err, def } => {
                    self.widget.with_rejected(|w| {
                        let mut payload = PromiseError {
                            err: err.to_owned(),
                            def: def.to_owned(),
                        };
                        w.event(ctx, event, &mut payload, env)
                    });
                }
            };
        }
    }

    fn lifecycle(
        &mut self,
        ctx: &mut LifeCycleCtx,
        event: &LifeCycle,
        data: &Promise<T, D, E>,
        env: &Env,
    ) {
        if data.state() != self.widget.state() {
            // Possible if getting lifecycle after an event that changed the data,
            // or on WidgetAdded.
            self.rebuild_widget(data.state());
        }
        assert_eq!(data.state(), self.widget.state(), "{event:?}");
        match data {
            Promise::Empty => {}
            Promise::Deferred { def } => {
                self.widget
                    .with_deferred(|w| w.lifecycle(ctx, event, def, env));
            }
            Promise::Resolved { val, .. } => {
                self.widget
                    .with_resolved(|w| w.lifecycle(ctx, event, val, env));
            }
            Promise::Rejected { err, def } => {
                self.widget.with_rejected(|w| {
                    let payload = PromiseError {
                        err: err.to_owned(),
                        def: def.to_owned(),
                    };
                    w.lifecycle(ctx, event, &payload, env)
                });
            }
        };
    }

    fn update(
        &mut self,
        ctx: &mut UpdateCtx,
        old_data: &Promise<T, D, E>,
        data: &Promise<T, D, E>,
        env: &Env,
    ) {
        if old_data.state() != data.state() {
            self.rebuild_widget(data.state());
            ctx.children_changed();
        } else {
            match data {
                Promise::Empty => {}
                Promise::Deferred { def } => {
                    self.widget.with_deferred(|w| w.update(ctx, def, env));
                }
                Promise::Resolved { val, .. } => {
                    self.widget.with_resolved(|w| w.update(ctx, val, env));
                }
                Promise::Rejected { err, def } => {
                    let payload = PromiseError {
                        err: err.to_owned(),
                        def: def.to_owned(),
                    };
                    self.widget.with_rejected(|w| w.update(ctx, &payload, env));
                }
            };
        }
    }

    fn layout(
        &mut self,
        ctx: &mut LayoutCtx,
        bc: &BoxConstraints,
        data: &Promise<T, D, E>,
        env: &Env,
    ) -> Size {
        match data {
            Promise::Empty => None,
            Promise::Deferred { def } => self.widget.with_deferred(|w| {
                let size = w.layout(ctx, bc, def, env);
                w.set_origin(ctx, Point::ORIGIN);
                size
            }),
            Promise::Resolved { val, .. } => self.widget.with_resolved(|w| {
                let size = w.layout(ctx, bc, val, env);
                w.set_origin(ctx, Point::ORIGIN);
                size
            }),
            Promise::Rejected { err, def } => self.widget.with_rejected(|w| {
                let payload = PromiseError {
                    err: err.to_owned(),
                    def: def.to_owned(),
                };
                let size = w.layout(ctx, bc, &payload, env);
                w.set_origin(ctx, Point::ORIGIN);
                size
            }),
        }
        .unwrap_or_default()
    }

    fn paint(&mut self, ctx: &mut PaintCtx, data: &Promise<T, D, E>, env: &Env) {
        match data {
            Promise::Empty => {}
            Promise::Deferred { def } => {
                self.widget.with_deferred(|w| w.paint(ctx, def, env));
            }
            Promise::Resolved { val, .. } => {
                self.widget.with_resolved(|w| w.paint(ctx, val, env));
            }
            Promise::Rejected { err, def } => {
                let payload = PromiseError {
                    err: err.to_owned(),
                    def: def.to_owned(),
                };
                self.widget.with_rejected(|w| w.paint(ctx, &payload, env));
            }
        };
    }
}

impl<T, D: Data + Clone, E: Data + Clone> PromiseWidget<T, D, E> {
    fn state(&self) -> PromiseState {
        match self {
            Self::Empty => PromiseState::Empty,
            Self::Deferred(_) => PromiseState::Deferred,
            Self::Resolved(_) => PromiseState::Resolved,
            Self::Rejected(_) => PromiseState::Rejected,
        }
    }

    fn with_deferred<R, F: FnOnce(&mut WidgetPod<D, Box<dyn Widget<D>>>) -> R>(
        &mut self,
        f: F,
    ) -> Option<R> {
        if let Self::Deferred(widget) = self {
            Some(f(widget))
        } else {
            None
        }
    }

    fn with_resolved<R, F: FnOnce(&mut WidgetPod<T, Box<dyn Widget<T>>>) -> R>(
        &mut self,
        f: F,
    ) -> Option<R> {
        if let Self::Resolved(widget) = self {
            Some(f(widget))
        } else {
            None
        }
    }

    fn with_rejected<
        R,
        F: FnOnce(&mut WidgetPod<PromiseError<E, D>, Box<dyn Widget<PromiseError<E, D>>>>) -> R,
    >(
        &mut self,
        f: F,
    ) -> Option<R> {
        if let Self::Rejected(widget) = self {
            Some(f(widget))
        } else {
            None
        }
    }
}
