# GPU Screen Recorder (GPUI)

A Rust + [GPUI](https://www.gpui.rs/) frontend for
[gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder).

This is a rewrite of the Qt6 frontend
([gpu-screen-recorder-qt](https://git.dec05eba.com/gpu-screen-recorder-qt))
in Rust, keeping the same behaviour and configuration format: the same
pages, the same config file, the same system-tray menu and the same
`org.freedesktop.portal.GlobalShortcuts` hotkeys.

GPUI is the GPU-accelerated UI framework from Zed. This project builds
against the maintained standalone release published as `gpui-unofficial`,
imported as `gpui`:

```toml
gpui = { package = "gpui-unofficial", version = "=1.24.2" }
gpui_platform = { package = "gpui-platform-gpui-unofficial", version = "=1.24.2" }
```

## Dependencies

Runtime:

- [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder) (the backend)
- A working GPU driver (the UI is rendered on the GPU)
- A StatusNotifierItem host (KDE Plasma, GNOME with an extension, swaybar, …) for the tray icon
- A session bus for tray/notifications/hotkeys

Build:

- Rust (edition 2024)
- Development headers for: `libwayland`, `libxkbcommon` (and `-x11`),
  `libx11`, `libxcb` (`render0`, `shape0`, `xfixes0`), `libfontconfig`,
  `libfreetype`, `libvulkan`, `libssl`, `libEGL`/`libGL`

On Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libssl-dev \
  libfontconfig-dev libfreetype-dev libwayland-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libx11-dev libxcb1-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libvulkan-dev libegl1 libgl1-mesa-dri
```

## Build

```sh
cargo build --release
```

## Run

```sh
./target/release/gpu-screen-recorder-gpui
```

## Install

```sh
sudo install -Dm755 target/release/gpu-screen-recorder-gpui /usr/local/bin/gpu-screen-recorder-gpui
sudo install -Dm644 com.dec05eba.gpu_screen_recorder_gpui.desktop /usr/share/applications/
sudo install -Dm644 icons/hicolor/scalable/apps/com.dec05eba.gpu_screen_recorder.svg \
  /usr/share/icons/hicolor/scalable/apps/
```

## Configuration

The configuration lives in the same place as the other frontends:

```
$XDG_CONFIG_HOME/gpu-screen-recorder/config   # usually ~/.config/gpu-screen-recorder/config
```

It is the same `key value` format used by the GTK and Qt frontends, so an
existing config is picked up unchanged. The config is rewritten on exit.
Recordings default to the XDG videos directory (from `~/.config/user-dirs.dirs`).

## Global shortcuts

Hotkeys are registered through the
`org.freedesktop.portal.GlobalShortcuts` portal. The portal only supports
compositors that implement it; in practice that means **KDE Plasma on
Wayland**. On other Wayland compositors the app prints a warning and runs
without hotkeys. The hotkeys can be rebound by the user through the
compositor's shortcut settings — the app asks the portal to bind them.

## System tray

The tray icon is a StatusNotifierItem. Its menu mirrors the Qt frontend:
it shows only the actions relevant to the page you are on, plus
"Hide/Show window", the hide-when-recording option and "Exit".

## Deviations from the Qt frontend

This is intended to be behaviourally equivalent to the Qt frontend. A few
points are deliberately different:

- **Global shortcut binding.** The Qt frontend asked the portal to
  *list* shortcuts but never actually bound them, so no hotkey ever fired.
  This version performs the `BindShortcuts` call, so the hotkeys work.
- **"Recording saved" notification.** The Qt frontend kept a
  `m_prevExitStatus` field that was only refreshed when the recorder exited
  on its own. When the user stopped recording by hand the field was stale,
  so the "saved" notification (and its success path) could not fire. This
  version uses the actual exit status of the process it just reaped, so the
  saved/error notifications fire on both the manual and natural stop paths.
- **Choosing an output directory.** The Qt frontend opened a native
  `QFileDialog` to pick the directory. This version uses an editable text
  field for the save directory (for recordings and replays), which avoids a
  secondary toolkit dependency but is keyboard-edited rather than browsed.
- **Hide to tray.** The Qt frontend hid the window. This version minimizes
  the window (`Window::minimize_window`), which is the closest GPUI
  equivalent on Linux; the window is restored with `activate_window`.

Everything else — the page set, the argument builder, the exit-code
handling, the notification texts, the audio merge/split and inversion
rules, and the configuration keys (including
`streaming.start_stop_recording_hotkey`) — is ported verbatim.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
