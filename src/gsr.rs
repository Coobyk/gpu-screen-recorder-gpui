//! The `gpu-screen-recorder` backend: capability discovery, audio device
//! enumeration and the recorder child process.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DisplayServer {
    #[default]
    Unknown,
    X11,
    Wayland,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GpuVendor {
    #[default]
    Unknown,
    Amd,
    Intel,
    Nvidia,
    Broadcom,
}

impl GpuVendor {
    pub fn name(self) -> &'static str {
        match self {
            GpuVendor::Amd => "AMD",
            GpuVendor::Intel => "Intel",
            GpuVendor::Nvidia => "NVIDIA",
            GpuVendor::Broadcom => "Broadcom",
            GpuVendor::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemInfo {
    pub display_server: DisplayServer,
    pub supports_app_audio: bool,
    pub is_steam_deck: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GpuInfo {
    pub vendor: GpuVendor,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SupportedVideoCodecs {
    pub h264: bool,
    pub h264_software: bool,
    pub hevc: bool,
    pub hevc_hdr: bool,
    pub hevc_10bit: bool,
    pub av1: bool,
    pub av1_hdr: bool,
    pub av1_10bit: bool,
    pub vp8: bool,
    pub vp9: bool,
}

impl SupportedVideoCodecs {
    /// True when no hardware codec was found.
    pub fn has_no_hardware_codec(&self) -> bool {
        !self.h264 && !self.hevc && !self.av1 && !self.vp8 && !self.vp9
    }
}

#[derive(Clone, Debug, Default)]
pub struct GsrMonitor {
    pub name: String,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, Default)]
pub struct SupportedCaptureOptions {
    pub window: bool,
    pub focused: bool,
    pub portal: bool,
    pub monitors: Vec<GsrMonitor>,
}

#[derive(Clone, Debug, Default)]
pub struct GsrInfo {
    pub system_info: SystemInfo,
    pub gpu_info: GpuInfo,
    pub supported_video_codecs: SupportedVideoCodecs,
    pub supported_capture_options: SupportedCaptureOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GsrInfoExitStatus {
    Ok,
    FailedToRunCommand,
    OpenGlFailed,
    NoDrmCard,
}

#[derive(Clone, Debug)]
pub struct AudioInput {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    Recording,
    Replay,
    Streaming,
}

/// A container the recorder can write into.
pub struct Container {
    pub name: &'static str,
    pub extension: &'static str,
}

pub const SUPPORTED_CONTAINERS: &[Container] = &[
    Container { name: "mp4", extension: "mp4" },
    Container { name: "flv", extension: "flv" },
    Container { name: "matroska", extension: "mkv" },
    Container { name: "mov", extension: "mov" },
    Container { name: "mpegts", extension: "ts" },
    Container { name: "hls", extension: "m3u8" },
];

pub fn is_inside_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some()
}

fn dlopen_exists(name: &str) -> bool {
    let Ok(lib) = std::ffi::CString::new(name) else {
        return false;
    };
    unsafe {
        let handle = libc::dlopen(lib.as_ptr(), libc::RTLD_LAZY);
        if handle.is_null() {
            false
        } else {
            libc::dlclose(handle);
            true
        }
    }
}

pub fn is_cuda_installed() -> bool {
    dlopen_exists("libcuda.so.1") || dlopen_exists("libcuda.so")
}

pub fn is_nvenc_installed() -> bool {
    dlopen_exists("libnvidia-encode.so.1")
}

pub fn get_date_str() -> String {
    // Days-from-civil algorithm avoids pulling in a date/time dependency.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = now.div_euclid(86_400);
    let secs = now.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        year,
        month,
        day,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn create_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

pub fn list_audio_devices() -> Vec<AudioInput> {
    let mut result = Vec::new();
    let Ok(output) = Command::new("gpu-screen-recorder")
        .arg("--list-audio-devices")
        .output()
    else {
        return result;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let Some((name, description)) = line.split_once('|') else {
            continue;
        };
        result.push(AudioInput {
            name: name.to_owned(),
            description: description.to_owned(),
        });
    }
    result
}

pub fn list_application_audio() -> Vec<String> {
    let Ok(output) = Command::new("gpu-screen-recorder")
        .arg("--list-application-audio")
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Runs `gpu-screen-recorder --info` and parses its sectioned output.
pub fn get_gpu_screen_recorder_info() -> (GsrInfo, GsrInfoExitStatus) {
    let mut info = GsrInfo::default();

    let output = match Command::new("gpu-screen-recorder").arg("--info").output() {
        Ok(output) => output,
        Err(_) => return (info, GsrInfoExitStatus::FailedToRunCommand),
    };

    #[derive(PartialEq, Eq)]
    enum Section {
        Unknown,
        SystemInfo,
        GpuInfo,
        VideoCodecs,
        CaptureOptions,
    }
    let mut section = Section::Unknown;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(name) = line.strip_prefix("section=") {
            section = match name {
                "system_info" => Section::SystemInfo,
                "gpu_info" => Section::GpuInfo,
                "video_codecs" => Section::VideoCodecs,
                "capture_options" => Section::CaptureOptions,
                _ => Section::Unknown,
            };
            continue;
        }

        let (attr, value) = match line.split_once('|') {
            Some((attr, value)) => (attr, value),
            None => (line, ""),
        };

        match section {
            Section::SystemInfo => match attr {
                "display_server" => {
                    info.system_info.display_server = match value {
                        "x11" => DisplayServer::X11,
                        "wayland" => DisplayServer::Wayland,
                        _ => DisplayServer::Unknown,
                    };
                }
                "is_steam_deck" => info.system_info.is_steam_deck = value == "yes",
                "supports_app_audio" => info.system_info.supports_app_audio = value == "yes",
                _ => {}
            },
            Section::GpuInfo => {
                if attr == "vendor" {
                    info.gpu_info.vendor = match value {
                        "amd" => GpuVendor::Amd,
                        "intel" => GpuVendor::Intel,
                        "nvidia" => GpuVendor::Nvidia,
                        "broadcom" => GpuVendor::Broadcom,
                        _ => GpuVendor::Unknown,
                    };
                }
            }
            Section::VideoCodecs => {
                let codecs = &mut info.supported_video_codecs;
                match line {
                    "h264" => codecs.h264 = true,
                    "h264_software" => codecs.h264_software = true,
                    "hevc" => codecs.hevc = true,
                    "hevc_hdr" => codecs.hevc_hdr = true,
                    "hevc_10bit" => codecs.hevc_10bit = true,
                    "av1" => codecs.av1 = true,
                    "av1_hdr" => codecs.av1_hdr = true,
                    "av1_10bit" => codecs.av1_10bit = true,
                    "vp8" => codecs.vp8 = true,
                    "vp9" => codecs.vp9 = true,
                    _ => {}
                }
            }
            Section::CaptureOptions => match line {
                "window" => info.supported_capture_options.window = true,
                "focused" => info.supported_capture_options.focused = true,
                "portal" => info.supported_capture_options.portal = true,
                "region" => {}
                _ if line.is_empty() || line.starts_with('/') => {}
                _ => {
                    let mut monitor = GsrMonitor::default();
                    if let Some((name, size)) = line.split_once('|') {
                        monitor.name = name.to_owned();
                        if let Some((w, h)) = size.split_once('x') {
                            monitor.width = w.parse().unwrap_or(0);
                            monitor.height = h.parse().unwrap_or(0);
                        }
                    } else {
                        monitor.name = line.to_owned();
                    }
                    info.supported_capture_options.monitors.push(monitor);
                }
            },
            Section::Unknown => {}
        }
    }

    let status = match output.status.code() {
        Some(0) => GsrInfoExitStatus::Ok,
        Some(22) => GsrInfoExitStatus::OpenGlFailed,
        Some(23) => GsrInfoExitStatus::NoDrmCard,
        _ => GsrInfoExitStatus::FailedToRunCommand,
    };
    (info, status)
}

/// A running `gpu-screen-recorder` process.
pub struct Recorder {
    pub child: Child,
}

impl Recorder {
    /// Spawns the recorder. `args` excludes the program name.
    pub fn spawn(args: &[String]) -> std::io::Result<Self> {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new("gpu-screen-recorder");
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());

        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut log = "info: running command: gpu-screen-recorder".to_string();
        for arg in args {
            if !arg.is_empty() {
                log.push(' ');
                log.push_str(arg);
            }
        }
        let _ = writeln!(std::io::stderr(), "{log}");

        let child = command.spawn()?;
        Ok(Self { child })
    }

    pub fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    /// Sends SIGINT to stop the recorder gracefully.
    pub fn interrupt(&self) {
        unsafe {
            libc::kill(self.pid(), libc::SIGINT);
        }
    }

    /// SIGUSR2 toggles pause/unpause inside the recorder.
    pub fn toggle_pause(&self) {
        unsafe {
            libc::kill(self.pid(), libc::SIGUSR2);
        }
    }

    /// SIGUSR1 saves the current replay buffer.
    pub fn save_replay(&self) {
        unsafe {
            libc::kill(self.pid(), libc::SIGUSR1);
        }
    }

    /// Stops the recorder and returns its exit status, if any.
    ///
    /// A recorder that has already exited is reaped without being signalled,
    /// matching the `waitpid(WNOHANG)` / `kill(SIGINT)` dance in the Qt frontend.
    pub fn stop(&mut self) -> Option<std::process::ExitStatus> {
        if let Ok(Some(status)) = self.child.try_wait() {
            return Some(status);
        }
        self.interrupt();
        self.child.wait().ok()
    }

    /// Non-blocking check for process exit.
    pub fn try_wait(&mut self) -> Option<std::process::ExitStatus> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status),
            _ => None,
        }
    }
}
