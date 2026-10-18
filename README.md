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

Building from source on Arch with `makepkg` needs `options=('!lto')`. makepkg's
`lto` option is on by default and puts `-flto=auto` into `CFLAGS`; GCC then
compiles the C shim inside `wayland-backend` as an LTO object, and rustc bundles
it into the rlib without an archive-index entry, so the final link fails with
`undefined symbol: wl_log_trampoline_to_rust_client`. Appending `-fno-lto` to
`CFLAGS` works too. The AUR package already does this.

## Run

```sh
./target/release/gpu-screen-recorder-gpui
```

## Install

On Arch, the package is in the AUR:

```sh
yay -S gpu-screen-recorder-gpui
```

Otherwise, from a built checkout:

```sh
sudo install -Dm755 target/release/gpu-screen-recorder-gpui /usr/local/bin/gpu-screen-recorder-gpui
sudo install -Dm644 com.coobyk.gpu_screen_recorder.desktop /usr/share/applications/
sudo install -d /usr/share/icons/hicolor/scalable/apps
sudo install -m644 icons/hicolor/scalable/apps/*.svg /usr/share/icons/hicolor/scalable/apps/
```

That icon directory holds the application icon plus the three tray variants
(`….tray-idle`, `….tray-recording`, `….tray-paused`) that the tray looks up by
name.

The application id is `com.coobyk.gpu_screen_recorder` — the reverse-DNS form of
the `coobyk.com` domain — and it is the same string as the `.desktop` file
name, which is what the compositor and the portal expect.

## Releases

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds on
`ubuntu-24.04` and publishes:

- `gpu-screen-recorder-gpui-<version>-x86_64-linux.tar.gz` — the binary with
  the `.desktop` and icon files; run `./install.sh` (honours `PREFIX`).
- `gpu-screen-recorder-gpui_<version>_amd64.deb` — `sudo apt install ./<file>.deb`.
- `SHA256SUMS` — checksums.

The `gpu-screen-recorder` backend is **not** bundled; install it separately.
The packages depend on the EGL/Vulkan/Wayland libraries the UI needs, but a
working GPU driver (Mesa or the NVIDIA driver) must already be installed.

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

The portal derives the application id from the process it is called by, which
it resolves to an installed `.desktop` file. Running the binary straight out
of `target/` therefore fails with `An app id is required`; install the app
(and `com.coobyk.gpu_screen_recorder.desktop`) for the hotkeys to register.

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
- **Closing the window without a tray.** Hiding to tray is only useful when
  a StatusNotifierItem host is actually running, so the app checks for one
  (`org.kde.StatusNotifierWatcher`). If none is present — a bare session, or
  `cargo run` from a terminal — closing the window quits instead of hiding,
  so the process can never be left running with no way to reach it.
- **Termination signals.** `SIGINT`/`SIGTERM`/`SIGHUP` (Ctrl+C, `kill`, a
  closing terminal) trigger the same orderly shutdown as the tray's "Exit":
  the recorder is stopped so the file is finalized, the config is written,
  and then the app quits. The Qt frontend had no handler, so these killed it
  abruptly.

Everything else — the page set, the argument builder, the exit-code
handling, the notification texts, the audio merge/split and inversion
rules, and the configuration keys (including
`streaming.start_stop_recording_hotkey`) — is ported verbatim.

## License

GPL-3.0. See [LICENSE](LICENSE).

The Qt and GTK frontends this derives from are GPL-3.0, so the whole work
stays under that license (no "or later" is granted upstream, and a
derivative cannot add one). This is a modified version: the Rust/GPUI
rewrite and every behavioural change are listed above under "Deviations
from the Qt frontend".

For the same reason the application id, `.desktop` file and icon names live
in this project's own `com.coobyk.*` namespace rather than the upstream
author's — the GPL grants no rights to the upstream name. References to the
upstream projects (`git.dec05eba.com`) remain as attribution, and the
Flatpak app id in the "failed to run" hint is the *backend's*, not this
app's.
