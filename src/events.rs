//! Events raised outside the window (system tray, global shortcuts).
//!
//! GPUI owns the UI thread, so the tray thread and the D-Bus portal thread cannot
//! touch the application state directly. Instead they push an `AppEvent` into a
//! channel that a background task drains on the UI thread.

use std::sync::mpsc::{Receiver, Sender};

/// Something the tray or a global shortcut asked the application to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppEvent {
    /// Show or hide the window (tray: "Hide window" / "Show window").
    ToggleWindow,
    /// Hide the window once recording starts (tray: checkmark item).
    SetHideWindowWhenRecording(bool),
    /// Quit the application (tray: "Exit").
    QuitApplication,
    /// Start/stop whichever capture the front page's "Stream" button would start.
    ToggleStreaming,
    /// Start/stop recording.
    ToggleRecording,
    /// Pause/unpause recording.
    TogglePause,
    /// Start/stop replay.
    ToggleReplay,
    /// Save the current replay buffer.
    SaveReplay,
    /// A global shortcut was activated by the desktop portal.
    ShortcutActivated(ShortcutId),
}

/// The global shortcut ids registered with the desktop portal. These are the same
/// ids the Qt frontend uses, so an existing portal binding keeps working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutId {
    StartStopRecording,
    PauseUnpauseRecording,
    SaveReplay,
}

impl ShortcutId {
    pub const ALL: [ShortcutId; 3] = [
        ShortcutId::StartStopRecording,
        ShortcutId::PauseUnpauseRecording,
        ShortcutId::SaveReplay,
    ];

    pub fn portal_id(self) -> &'static str {
        match self {
            ShortcutId::StartStopRecording => "gpu_screen_recorder_start_stop_recording",
            ShortcutId::PauseUnpauseRecording => "gpu_screen_recorder_pause_unpause_recording",
            ShortcutId::SaveReplay => "gpu_screen_recorder_save_replay",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ShortcutId::StartStopRecording => "Start/stop recording, replay or streaming",
            ShortcutId::PauseUnpauseRecording => "Pause/unpause recording",
            ShortcutId::SaveReplay => "Save replay",
        }
    }

    pub fn from_portal_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.portal_id() == id)
    }
}

/// The channel pair connecting the out-of-band threads to the UI thread.
pub struct EventChannel {
    pub sender: Sender<AppEvent>,
    pub receiver: Receiver<AppEvent>,
}

impl EventChannel {
    pub fn new() -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        Self { sender, receiver }
    }
}

impl Default for EventChannel {
    fn default() -> Self {
        Self::new()
    }
}
