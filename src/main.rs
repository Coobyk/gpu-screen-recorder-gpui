//! Entry point.
//!
//! Mirrors the Qt frontend's `main()`: refuse to run as root, read the config,
//! probe `gpu-screen-recorder`, run the startup validation, then bring up the
//! tray, the global-shortcut portal and the window that hosts the four pages.

mod actions;
mod app;
mod config;
mod events;
mod gsr;
mod notify;
mod shortcuts;
mod text_input;
mod theme;
mod tray;
mod widgets;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    App, Bounds, Context, Global, IntoElement, QuitMode, Render, SharedString, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};

use crate::app::GsrApp;
use crate::events::{EventChannel, ShortcutId};
use crate::gsr::{DisplayServer, GpuVendor, GsrInfo, GsrInfoExitStatus};
use crate::tray::TrayState;

/// Application id (reverse-DNS of coobyk.com), used as the Wayland app id and
/// to match the installed `.desktop` file. The Qt frontend used the upstream
/// author's namespace; this rewrite lives under its own.
const APP_ID: &str = "com.coobyk.gpu_screen_recorder";

/// Set by the termination signals and drained by the UI pump, which then runs
/// the same graceful shutdown as the tray's "Exit".
static TERMINATE_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn request_termination(_signal: libc::c_int) {
    // Only async-signal-safe work: flipping an atomic.
    TERMINATE_REQUESTED.store(true, Ordering::SeqCst);
}

/// Turns Ctrl+C and `kill` into an orderly shutdown instead of an abrupt exit,
/// so a running recording is finalized and the config is written out.
///
/// Installed only once the main window exists: the startup-error window has no
/// pump to drain the flag, so it keeps the default (terminating) disposition.
fn install_termination_handler() {
    let handler = request_termination as *const () as libc::sighandler_t;
    unsafe {
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGHUP, handler);
    }
}

/// Keeps the portal session alive for the process' lifetime.
///
/// `ShortcutService` closes the D-Bus session when dropped, and the closure
/// passed to `Application::run` returns before the event loop starts, so the
/// service has to live in a global instead of a local.
struct Shortcuts(#[allow(dead_code)] shortcuts::ShortcutService);

impl Global for Shortcuts {}

fn main() {
    // The Qt frontend pins the C locale so that `sscanf`/`atoi` parsing of the
    // `gpu-screen-recorder --info` output is locale independent. Numeral parsing
    // in Rust is already locale independent, but the neighbouring C libraries
    // loaded into the process (fontconfig, xkbcommon) still consult it.
    unsafe {
        libc::setlocale(libc::LC_ALL, c"C".as_ptr());
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let use_old_ui = args.iter().any(|arg| arg == "use-old-ui");
    let launched_by_daemon = args.iter().any(|arg| arg == "gsr-ui");

    if unsafe { libc::geteuid() } == 0 {
        eprintln!("Error: don't run as root");
        std::process::exit(1);
    }

    let (mut config, config_empty) = config::read_config();
    let (gsr_info, gsr_status) = gsr::get_gpu_screen_recorder_info();
    let flatpak = gsr::is_inside_flatpak();

    if use_old_ui {
        config.main_config.use_new_ui = false;
        config::save_config(&config);
    }

    let startup_error = validate_startup(&gsr_info, gsr_status, flatpak);

    gpui_platform::application().run(move |cx: &mut App| {
        // The window close is routed through the pump's graceful shutdown, so
        // the app must also be able to end explicitly (the tray's "Exit" item,
        // the termination signals, or the error dialog). `LastWindowClosed` is
        // the safety net: if a close ever gets through without the pump running
        // (the callback is not reliable on every backend), the process exits
        // instead of lingering with no window and no way to reach it.
        cx.set_quit_mode(QuitMode::LastWindowClosed);

        // Backspace, arrows, clipboard and select-all inside the text fields.
        text_input::bind_keys(cx);

        if let Some(error) = startup_error {
            open_error_window(cx, error);
            return;
        }

        install_termination_handler();

        let events = EventChannel::new();
        let tray_state = Arc::new(Mutex::new(TrayState::default()));
        let tray_handle = tray::spawn(tray_state.clone(), events.sender.clone());
        let notifier = notify::spawn();

        // A global shortcut portal session only exists on Wayland; on X11 the
        // Qt frontend never registered shortcuts either.
        if gsr_info.system_info.display_server == DisplayServer::Wayland {
            let service = shortcuts::spawn(events.sender.clone());
            service.bind(ShortcutId::ALL);
            cx.set_global(Shortcuts(service));
        }

        let app = cx.new(|cx| {
            GsrApp::new(
                config,
                config_empty,
                gsr_info,
                events,
                tray_state,
                tray_handle,
                notifier,
                cx,
            )
        });

        let bounds = Bounds::centered(None, size(px(640.), px(760.)), cx);
        let _window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(560.), px(640.))),
                    app_id: Some(APP_ID.to_owned()),
                    // Launched by the daemon (tray-less session startup): come
                    // up without stealing focus from whatever the user is doing.
                    focus: !launched_by_daemon,
                    ..Default::default()
                },
                |window, cx| {
                    // A close request from the compositor (the window's "X", or
                    // a desktop keybinding such as Hyprland's SUPER+W) runs the
                    // same orderly shutdown as the tray's "Exit": the recorder
                    // is stopped so the file is finalized, the config is written
                    // and the app quits.
                    //
                    // The window is not hidden instead because GPUI has no
                    // `hide()` on Linux and its `minimize_window()` is a no-op
                    // under compositors that ignore `xdg_toplevel.set_minimized`
                    // (Hyprland, sway), which left the window on screen and the
                    // process running with no obvious way out.
                    //
                    // Deferred to the pump rather than running the shutdown
                    // here: `cx.quit()` does not take effect when called from
                    // inside the platform's close callback, and the window must
                    // survive until the pump (which is bound to it) has run.
                    window.on_window_should_close(cx, |_window, _cx| {
                        TERMINATE_REQUESTED.store(true, Ordering::SeqCst);
                        false
                    });

                    // Out-of-band threads (tray, portal, notifications) cannot
                    // touch GPUI state, so a foreground task drains their channel
                    // and advances the recorder timers. It is detached because a
                    // dropped `Task` is cancelled.
                    let pump_app = app.downgrade();
                    window
                        .spawn(cx, async move |cx| {
                            let Ok(timer) = cx.update(|_, cx| cx.background_executor().clone())
                            else {
                                return;
                            };
                            loop {
                                timer.timer(Duration::from_millis(200)).await;
                                let Ok(true) = cx.update(|window, cx| {
                                    let Some(app) = pump_app.upgrade() else {
                                        return false;
                                    };
                                    if TERMINATE_REQUESTED.swap(false, Ordering::SeqCst) {
                                        app.update(cx, |app, cx| app.shutdown(cx));
                                    } else {
                                        app.update(cx, |app, cx| app.pump(window, cx));
                                    }
                                    true
                                }) else {
                                    break;
                                };
                            }
                        })
                        .detach();

                    app
                },
            )
            .expect("failed to open the application window");

        cx.activate(true);
    });
}

/// The Qt frontend's `gsr_startup_validation`: every failure is reported to the
/// user instead of starting a window the recorder cannot work under.
fn validate_startup(
    gsr_info: &GsrInfo,
    status: GsrInfoExitStatus,
    flatpak: bool,
) -> Option<(SharedString, SharedString)> {
    let error = |message: String| Some((SharedString::from("GPU Screen Recorder"), message.into()));

    match status {
        GsrInfoExitStatus::Ok => {}
        GsrInfoExitStatus::FailedToRunCommand => {
            let command = if flatpak {
                // `com.dec05eba.gpu_screen_recorder` here is the *backend's*
                // Flatpak app id on Flathub, not this app's id: leave it alone.
                "flatpak run --command=gpu-screen-recorder \
                 com.dec05eba.gpu_screen_recorder -w screen -f 60 -o video.mp4"
            } else {
                "gpu-screen-recorder -w screen -f 60 -o video.mp4"
            };
            return error(format!(
                "Failed to run 'gpu-screen-recorder'. Make sure it's installed.\nRun:\n{command}\nin a terminal."
            ));
        }
        GsrInfoExitStatus::OpenGlFailed => {
            return error(
                "Failed to get OpenGL info. Make sure your GPU drivers are installed.".to_owned(),
            );
        }
        GsrInfoExitStatus::NoDrmCard => {
            return error("Failed to find a valid DRM card.".to_owned());
        }
    }

    if gsr_info.system_info.display_server == DisplayServer::Unknown {
        return error("Neither X11 nor Wayland is running.".to_owned());
    }

    if gsr_info.gpu_info.vendor == GpuVendor::Nvidia {
        if !gsr::is_cuda_installed() {
            return error(
                "CUDA is not installed. GPU Screen Recorder requires CUDA for NVIDIA GPUs."
                    .to_owned(),
            );
        }
        if !gsr::is_nvenc_installed() {
            return error(
                "NVENC is not installed. GPU Screen Recorder requires NVENC for NVIDIA GPUs."
                    .to_owned(),
            );
        }
    }

    None
}

/// The `QMessageBox::critical` calls from the Qt frontend, as a window that
/// quits the application once it is dismissed.
fn open_error_window(cx: &mut App, error: (SharedString, SharedString)) {
    let bounds = Bounds::centered(None, size(px(460.), px(240.)), cx);
    let _window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(460.), px(240.))),
            app_id: Some(APP_ID.to_owned()),
            ..Default::default()
        },
        move |window, cx| {
            window.on_window_should_close(cx, |_window, cx| {
                cx.quit();
                true
            });
            cx.new(|_| StartupErrorView {
                title: error.0,
                message: error.1,
            })
        },
    );
}

struct StartupErrorView {
    title: SharedString,
    message: SharedString,
}

impl Render for StartupErrorView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .p_4()
            .bg(theme::BG)
            .text_color(theme::TEXT)
            .child(widgets::heading(self.title.clone()))
            .child(div().flex_1().text_sm().child(self.message.clone()))
            .child(
                div().flex().flex_row().justify_end().child(widgets::button(
                    "startup-error-ok",
                    "OK",
                    widgets::ButtonKind::Primary,
                    true,
                    |_event, _window, cx| cx.quit(),
                )),
            )
    }
}
