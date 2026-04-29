use druid::{ExtEventSink, Target};
use ksni::blocking::TrayMethods;

use crate::cmd;

pub struct SpotixTray {
    pub sink: ExtEventSink,
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

pub fn start_tray(sink: ExtEventSink) -> Option<ksni::blocking::Handle<SpotixTray>> {
    let tray = SpotixTray { sink };
    match tray.spawn() {
        Ok(handle) => {
            log::info!("tray: system tray icon started");
            Some(handle)
        }
        Err(err) => {
            log::warn!("tray: failed to start system tray icon: {err}");
            None
        }
    }
}
