//! The system tray icon (StatusNotifierItem via `ksni`).
//!
//! The original Qt frontend mirrored its tray menu on the current page: only the
//! actions that make sense for the page the user is on are visible. The same menu
//! is built here, driven by a small shared [`TrayState`] that the UI thread keeps
//! up to date.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
use ksni::{ToolTip, Tray};

use crate::events::AppEvent;

/// Which page the window is showing. Mirrors the Qt frontend's `SystrayPage`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrayPage {
    #[default]
    Front,
    Streaming,
    Recording,
    Replay,
}

/// The slice of application state the tray menu depends on.
#[derive(Clone, Debug, Default)]
pub struct TrayState {
    pub window_hidden: bool,
    pub hide_window_when_recording: bool,
    pub recording: bool,
    pub paused: bool,
    pub replaying: bool,
    pub streaming: bool,
    pub page: TrayPage,
}

pub struct GsrTray {
    state: Arc<Mutex<TrayState>>,
    events: Sender<AppEvent>,
}

impl GsrTray {
    fn item(&self, label: &str, enabled: bool, visible: bool, event: AppEvent) -> MenuItem<Self> {
        let events = self.events.clone();
        StandardItem {
            label: label.to_owned(),
            enabled,
            visible,
            activate: Box::new(move |_tray: &mut GsrTray| {
                let _ = events.send(event.clone());
            }),
            ..Default::default()
        }
        .into()
    }
}

impl Tray for GsrTray {
    fn id(&self) -> String {
        "com.coobyk.gpu_screen_recorder".to_owned()
    }

    fn title(&self) -> String {
        "GPU Screen Recorder".to_owned()
    }

    fn icon_name(&self) -> String {
        let state = self.state.lock().unwrap();
        let status = if state.paused {
            "tray-paused"
        } else if state.recording || state.replaying || state.streaming {
            "tray-recording"
        } else {
            "tray-idle"
        };
        format!("com.coobyk.gpu_screen_recorder.{status}")
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: self.icon_name(),
            title: "GPU Screen Recorder".to_owned(),
            description: String::new(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = self.state.lock().unwrap().clone();

        let mut items = vec![self.item(
            if state.window_hidden {
                "Show window"
            } else {
                "Hide window"
            },
            true,
            true,
            AppEvent::ToggleWindow,
        )];

        items.push(
            SubMenu {
                label: "Options".to_owned(),
                enabled: true,
                visible: true,
                submenu: {
                    let events = self.events.clone();
                    vec![CheckmarkItem {
                        label: "Hide window when recording starts".to_owned(),
                        checked: state.hide_window_when_recording,
                        enabled: true,
                        visible: true,
                        activate: Box::new(move |_tray: &mut GsrTray| {
                            let _ = events
                                .send(AppEvent::SetHideWindowWhenRecording(
                                    !state.hide_window_when_recording,
                                ));
                        }),
                        ..Default::default()
                    }
                    .into()]
                },
                ..Default::default()
            }
            .into(),
        );

        items.push(MenuItem::Separator);

        let page = state.page;
        items.push(self.item(
            if state.streaming {
                "Stop streaming"
            } else {
                "Start streaming"
            },
            true,
            page == TrayPage::Streaming,
            AppEvent::ToggleStreaming,
        ));
        items.push(self.item(
            if state.recording {
                "Stop recording"
            } else {
                "Start recording"
            },
            true,
            page == TrayPage::Recording,
            AppEvent::ToggleRecording,
        ));
        items.push(self.item(
            if state.paused {
                "Unpause recording"
            } else {
                "Pause recording"
            },
            state.recording,
            page == TrayPage::Recording,
            AppEvent::TogglePause,
        ));
        items.push(self.item(
            if state.replaying {
                "Stop replay"
            } else {
                "Start replay"
            },
            true,
            page == TrayPage::Replay,
            AppEvent::ToggleReplay,
        ));
        items.push(self.item(
            "Save replay",
            state.replaying,
            page == TrayPage::Replay,
            AppEvent::SaveReplay,
        ));

        items.push(MenuItem::Separator);
        items.push(self.item("Exit", true, true, AppEvent::QuitApplication));

        items
    }
}

/// True when a StatusNotifierItem host is present on the session bus.
///
/// `spawn` below is given `assume_sni_available(true)` so it keeps serving even
/// without a host, which also means its return value cannot be used to tell
/// whether an icon will actually appear. The window uses this check to decide
/// whether "hide to tray" (the Qt frontend's `closeEvent`) would leave the user
/// a way back, or whether closing should simply quit.
pub fn host_available() -> bool {
    let Ok(connection) = zbus::blocking::Connection::session() else {
        return false;
    };
    connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &("org.kde.StatusNotifierWatcher",),
        )
        .ok()
        .and_then(|reply| reply.body().deserialize::<bool>().ok())
        .unwrap_or(false)
}

/// Starts the tray service on its own thread. Returns the handle used by the UI
/// thread to refresh the menu; a missing StatusNotifierWatcher is not fatal.
pub fn spawn(
    state: Arc<Mutex<TrayState>>,
    events: Sender<AppEvent>,
) -> Option<ksni::blocking::Handle<GsrTray>> {
    use ksni::blocking::TrayMethods;

    let tray = GsrTray { state, events };
    let handle = tray
        // Without a watcher (a bare X11 session, `Xvfb`, a test run) the tray is
        // simply never shown instead of taking the application down with it.
        .assume_sni_available(true)
        .spawn();

    match handle {
        Ok(handle) => Some(handle),
        Err(err) => {
            eprintln!("warning: failed to start system tray: {err}");
            None
        }
    }
}
