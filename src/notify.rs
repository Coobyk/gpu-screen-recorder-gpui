//! Desktop notifications over `org.freedesktop.Notifications`.
//!
//! The Qt frontend showed these through `QSystemTrayIcon::showMessage`, which the
//! Linux platform implements with the same D-Bus interface.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use zbus::blocking::Connection;
use zvariant::Value;

/// A notification request, mirroring `MainWindow::showNotification`.
#[derive(Clone, Debug)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub urgent: bool,
}

/// Handle used by the UI thread to post notifications.
#[derive(Clone)]
pub struct Notifier {
    requests: Sender<Notification>,
}

impl Notifier {
    /// Queues a notification. Notification failures are never fatal.
    pub fn show(&self, title: impl Into<String>, body: impl Into<String>, urgent: bool) {
        let _ = self.requests.send(Notification {
            title: title.into(),
            body: body.into(),
            urgent,
        });
    }
}

/// Starts the notification thread, which owns the D-Bus connection.
pub fn spawn() -> Notifier {
    let (requests, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("gsr-notifications".to_owned())
        .spawn(move || run(receiver))
        .expect("failed to spawn notifications thread");

    Notifier { requests }
}

fn run(requests: Receiver<Notification>) {
    let Ok(connection) = Connection::session() else {
        eprintln!("warning: failed to connect to the session bus for notifications");
        // Drain so that senders do not see a disconnected channel.
        while requests.recv().is_ok() {}
        return;
    };

    while let Ok(notification) = requests.recv() {
        let mut hints: HashMap<String, Value<'static>> = HashMap::new();
        // 1 = normal, 2 = critical (the freedesktop `urgency` hint).
        hints.insert(
            "urgency".to_owned(),
            Value::from(if notification.urgent { 2u8 } else { 1u8 }),
        );

        let expire_timeout = if notification.urgent { 10_000 } else { 3_000 };

        let body = (
            "GPU Screen Recorder".to_owned(),
            0u32,
            String::new(),
            notification.title,
            notification.body,
            Vec::<String>::new(),
            hints,
            expire_timeout,
        );

        let _ = connection.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &body,
        );
    }
}
