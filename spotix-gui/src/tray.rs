use std::sync::{Arc, Mutex};
use std::thread;

use druid::{ExtEventSink, Target};
use ksni::blocking::{Handle, TrayMethods};

use crate::cmd;

pub struct SpotixTray {
    sink: ExtEventSink,
}

impl ksni::Tray for SpotixTray {
    fn id(&self) -> String {
        "spotix-tray".into()
    }

    fn title(&self) -> String {
        "Spotix".into()
    }

    fn icon_name(&self) -> String {
        "spotix".into()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Open Spotix".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this
                        .sink
                        .submit_command(cmd::TRAY_SHOW_WINDOW, (), Target::Global);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this
                        .sink
                        .submit_command(cmd::QUIT_APP_WITH_SAVE, (), Target::Global);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Send-and-Sync wrapper for the tray handle so it can travel across
/// a Druid `Selector` payload. The inner `Option` lets the quit path
/// `take()` the handle and call `shutdown()` exactly once.
pub type TrayHandle = Arc<Mutex<Option<Handle<SpotixTray>>>>;

/// Spawn the StatusNotifierItem on a worker thread so the synchronous
/// D-Bus negotiation inside `ksni` does not block the GUI's first paint.
/// On success, the handle is delivered back to the main thread via
/// [`cmd::TRAY_STARTED`]; on failure, the error is logged and no command
/// is sent (the close-to-tray preference will silently stay inert).
pub fn start_tray_async(sink: ExtEventSink) {
    thread::Builder::new()
        .name("spotix-tray".into())
        .spawn(move || {
            let tray = SpotixTray { sink: sink.clone() };
            match tray.spawn() {
                Ok(handle) => {
                    log::info!("tray: system tray icon started");
                    let payload: TrayHandle = Arc::new(Mutex::new(Some(handle)));
                    let _ = sink.submit_command(cmd::TRAY_STARTED, payload, Target::Global);
                }
                Err(err) => {
                    log::warn!("tray: failed to start system tray icon: {err}");
                }
            }
        })
        .expect("failed to spawn tray worker thread");
}
