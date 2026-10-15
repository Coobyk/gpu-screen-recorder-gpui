//! Global shortcuts through the `org.freedesktop.portal.GlobalShortcuts` portal.
//!
//! This is the same flow the Qt frontend drives with `QDBusInterface`: create a
//! portal session, bind the three recorder shortcuts, then react to the portal's
//! `Activated` signal. It runs on a dedicated thread because the portal calls are
//! blocking, and because the signal needs to be awaited for the lifetime of the
//! application.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type as MessageType;
use zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::events::{AppEvent, ShortcutId};

const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const GLOBAL_SHORTCUTS_IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const SESSION_IFACE: &str = "org.freedesktop.portal.Session";

/// Same token length as the Qt frontend (`DBUS_RANDOM_STR_SIZE`).
const RANDOM_STR_SIZE: usize = 16;

/// Messages from the UI thread to the portal thread.
enum Command {
    Bind(Vec<ShortcutId>),
    Shutdown,
}

/// Handle to the portal thread. Dropping it closes the portal session.
pub struct ShortcutService {
    commands: Sender<Command>,
}

impl ShortcutService {
    /// Registers `shortcuts` with the compositor. The compositor may prompt the
    /// user the first time; subsequent runs reuse the binding.
    pub fn bind(&self, shortcuts: impl IntoIterator<Item = ShortcutId>) {
        let shortcuts: Vec<ShortcutId> = shortcuts.into_iter().collect();
        let _ = self.commands.send(Command::Bind(shortcuts));
    }
}

impl Drop for ShortcutService {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
    }
}

/// Starts the portal thread. Events are delivered through `events`.
pub fn spawn(events: Sender<AppEvent>) -> ShortcutService {
    let (commands, command_rx) = mpsc::channel();
    thread::Builder::new()
        .name("gsr-global-shortcuts".to_owned())
        .spawn(move || {
            if let Err(err) = run(events, command_rx) {
                eprintln!("warning: failed to init global shortcuts: {err}");
            }
        })
        .expect("failed to spawn global shortcuts thread");

    ShortcutService { commands }
}

fn run(events: Sender<AppEvent>, commands: Receiver<Command>) -> zbus::Result<()> {
    let connection = Connection::session()?;

    let unique_name = connection
        .unique_name()
        .map(ToString::to_string)
        .unwrap_or_default();
    let sender_token = unique_name.trim_start_matches(':').replace('.', "_");

    // The portal derives the request and session handles from the tokens we pass
    // in, so both paths are known before the reply arrives. That lets us install
    // the signal match before calling, which avoids missing a fast reply.
    let handle_token = format!(
        "gpu_screen_recorder_gtk_handle_{}_{}",
        random_string(RANDOM_STR_SIZE),
        0
    );
    let session_token = "gpu_screen_recorder_gtk";
    let session_handle = format!(
        "{PORTAL_PATH}/session/{sender_token}/{session_token}"
    );

    let mut options: HashMap<String, Value<'static>> = HashMap::new();
    options.insert("handle_token".to_owned(), Value::from(handle_token));
    options.insert(
        "session_handle_token".to_owned(),
        Value::from(session_token.to_owned()),
    );

    connection.call_method(
        Some(PORTAL_DEST),
        PORTAL_PATH,
        Some(GLOBAL_SHORTCUTS_IFACE),
        "CreateSession",
        &options,
    )?;

    let session = Arc::new(session_handle.clone());
    let listener = {
        let connection = connection.clone();
        let events = events.clone();
        let session = session.clone();
        thread::Builder::new()
            .name("gsr-global-shortcuts-signals".to_owned())
            .spawn(move || listen_activated(connection, events, session))
            .expect("failed to spawn global shortcuts signal thread")
    };

    while let Ok(command) = commands.recv() {
        match command {
            Command::Bind(shortcuts) => bind(&connection, &session_handle, &shortcuts),
            Command::Shutdown => break,
        }
    }

    let _ = connection.call_method(
        Some(PORTAL_DEST),
        session_handle.as_str(),
        Some(SESSION_IFACE),
        "Close",
        &(),
    );

    drop(listener);
    Ok(())
}

fn bind(connection: &Connection, session_handle: &str, shortcuts: &[ShortcutId]) {
    let mut options: HashMap<String, Value<'static>> = HashMap::new();
    options.insert(
        "handle_token".to_owned(),
        Value::from(format!(
            "gpu_screen_recorder_gtk_handle_{}_{}",
            random_string(RANDOM_STR_SIZE),
            1
        )),
    );

    let mut entries: Vec<(String, HashMap<String, Value<'static>>)> = Vec::with_capacity(shortcuts.len());
    for shortcut in shortcuts {
        let mut properties: HashMap<String, Value<'static>> = HashMap::new();
        properties.insert(
            "description".to_owned(),
            Value::from(shortcut.description().to_owned()),
        );
        // No `preferred_trigger`: the compositor picks a binding for the user to
        // confirm, which is also what the Qt frontend leaves it to.
        entries.push((shortcut.portal_id().to_owned(), properties));
    }

    let Ok(session_path) = ObjectPath::try_from(session_handle) else {
        eprintln!("warning: invalid global shortcuts session handle");
        return;
    };

    let body = (
        session_path,
        entries,
        String::new(),
        options,
    );

    if let Err(err) = connection.call_method(
        Some(PORTAL_DEST),
        PORTAL_PATH,
        Some(GLOBAL_SHORTCUTS_IFACE),
        "BindShortcuts",
        &body,
    ) {
        eprintln!("warning: failed to bind global shortcuts: {err}");
    }
}

fn listen_activated(connection: Connection, events: Sender<AppEvent>, session: Arc<String>) {
    let rule = match MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender(PORTAL_DEST)
        .and_then(|builder| builder.path(PORTAL_PATH))
        .and_then(|builder| builder.interface(GLOBAL_SHORTCUTS_IFACE))
        .and_then(|builder| builder.member("Activated"))
    {
        Ok(builder) => builder.build(),
        Err(err) => {
            eprintln!("warning: failed to build global shortcuts match rule: {err}");
            return;
        }
    };

    let mut messages = match MessageIterator::for_match_rule(rule, &connection, Some(64)) {
        Ok(messages) => messages,
        Err(err) => {
            eprintln!("warning: failed to subscribe to global shortcuts: {err}");
            return;
        }
    };

    for message in messages.by_ref() {
        let Ok(message) = message else {
            break;
        };
        // (session_handle, shortcut_id, timestamp, options)
        let Ok((handle, shortcut_id, _, _)) = message
            .body()
            .deserialize::<(OwnedObjectPath, String, u32, HashMap<String, OwnedValue>)>()
        else {
            continue;
        };
        if handle.as_str() != session.as_str() {
            continue;
        }
        let Some(id) = ShortcutId::from_portal_id(&shortcut_id) else {
            continue;
        };
        if events.send(AppEvent::ShortcutActivated(id)).is_err() {
            break;
        }
    }
}

/// A short alphanumeric token, matching the Qt frontend's `generateRandomString`
/// (which is only ever used to make D-Bus handle tokens unique).
fn random_string(length: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() ^ u64::from(elapsed.subsec_nanos()))
        .unwrap_or(0)
        ^ (u64::from(std::process::id()) << 32);

    let mut result = String::with_capacity(length);
    for _ in 0..length {
        // xorshift64; plenty for a handle token.
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        result.push(ALPHABET[(seed % 62) as usize] as char);
    }
    result
}
