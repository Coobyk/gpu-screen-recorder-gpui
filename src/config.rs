//! Configuration: the same on-disk format as the Qt frontend
//! (`$XDG_CONFIG_HOME/gpu-screen-recorder/config`), with one `key value`
//! pair per line.

use std::collections::HashMap;
use std::path::PathBuf;

/// A global shortcut, stored as `keysym modifiers`.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ConfigHotkey {
    pub keysym: i64,
    pub modifiers: u32,
}

#[derive(Clone, Debug)]
pub struct MainConfig {
    pub record_area_option: String,
    pub record_area_width: i32,
    pub record_area_height: i32,
    pub video_width: i32,
    pub video_height: i32,
    pub fps: i32,
    pub video_bitrate: i32,
    pub merge_audio_tracks: bool,
    pub record_app_audio_inverted: bool,
    pub change_video_resolution: bool,
    pub audio_input: Vec<String>,
    pub color_range: String,
    pub quality: String,
    pub codec: String,
    pub audio_codec: String,
    pub framerate_mode: String,
    pub advanced_view: bool,
    pub overclock: bool,
    pub show_recording_started_notifications: bool,
    pub show_recording_stopped_notifications: bool,
    pub show_recording_saved_notifications: bool,
    pub record_cursor: bool,
    pub hide_window_when_recording: bool,
    pub software_encoding_warning_shown: bool,
    pub hevc_amd_bug_warning_shown: bool,
    pub av1_amd_bug_warning_shown: bool,
    pub restore_portal_session: bool,
    pub use_new_ui: bool,
    pub installed_gsr_global_hotkeys_version: i32,
}

impl Default for MainConfig {
    fn default() -> Self {
        Self {
            record_area_option: String::new(),
            record_area_width: 0,
            record_area_height: 0,
            video_width: 0,
            video_height: 0,
            fps: 60,
            video_bitrate: 15000,
            merge_audio_tracks: true,
            record_app_audio_inverted: false,
            change_video_resolution: false,
            audio_input: Vec::new(),
            color_range: String::new(),
            quality: String::new(),
            codec: String::new(),
            audio_codec: String::new(),
            framerate_mode: String::new(),
            advanced_view: false,
            overclock: false,
            show_recording_started_notifications: false,
            show_recording_stopped_notifications: false,
            show_recording_saved_notifications: true,
            record_cursor: true,
            hide_window_when_recording: false,
            software_encoding_warning_shown: false,
            hevc_amd_bug_warning_shown: false,
            av1_amd_bug_warning_shown: false,
            restore_portal_session: true,
            use_new_ui: false,
            installed_gsr_global_hotkeys_version: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct YoutubeStreamConfig {
    pub stream_key: String,
}

#[derive(Clone, Debug, Default)]
pub struct TwitchStreamConfig {
    pub stream_key: String,
}

#[derive(Clone, Debug, Default)]
pub struct CustomStreamConfig {
    pub url: String,
    pub container: String,
}

#[derive(Clone, Debug, Default)]
pub struct StreamingConfig {
    pub streaming_service: String,
    pub youtube: YoutubeStreamConfig,
    pub twitch: TwitchStreamConfig,
    pub custom: CustomStreamConfig,
    pub start_stop_recording_hotkey: ConfigHotkey,
}

#[derive(Clone, Debug, Default)]
pub struct RecordConfig {
    pub save_directory: String,
    pub container: String,
    pub start_stop_recording_hotkey: ConfigHotkey,
    pub pause_unpause_recording_hotkey: ConfigHotkey,
}

#[derive(Clone, Debug)]
pub struct ReplayConfig {
    pub save_directory: String,
    pub container: String,
    pub replay_time: i32,
    pub start_stop_recording_hotkey: ConfigHotkey,
    pub save_recording_hotkey: ConfigHotkey,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            save_directory: String::new(),
            container: String::new(),
            replay_time: 30,
            start_stop_recording_hotkey: ConfigHotkey::default(),
            save_recording_hotkey: ConfigHotkey::default(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub main_config: MainConfig,
    pub streaming_config: StreamingConfig,
    pub record_config: RecordConfig,
    pub replay_config: ReplayConfig,
}

pub fn get_home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn get_config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| get_home_dir().join(".config"));
    base.join("gpu-screen-recorder")
}

/// Reads `~/.config/user-dirs.dirs` and returns the `XDG_*_DIR` variables.
fn get_xdg_variables() -> HashMap<String, String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| get_home_dir().join(".config"));

    let mut result = HashMap::new();
    let Ok(content) = std::fs::read_to_string(base.join("user-dirs.dirs")) else {
        return result;
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
            continue;
        };
        let value = if let Some(rest) = value.strip_prefix("$HOME/") {
            get_home_dir().join(rest).to_string_lossy().into_owned()
        } else {
            value.to_owned()
        };
        result.insert(key.trim().to_owned(), value);
    }
    result
}

pub fn get_videos_dir() -> PathBuf {
    let xdg = get_xdg_variables();
    match xdg.get("XDG_VIDEOS_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => get_home_dir().join("Videos"),
    }
}

fn parse_i32(value: &str) -> i32 {
    value.trim().parse::<i32>().unwrap_or(0)
}

fn parse_bool(value: &str) -> bool {
    value == "true"
}

fn parse_hotkey(value: &str) -> ConfigHotkey {
    let mut parts = value.split_whitespace();
    let keysym = parts.next().and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    let modifiers = parts.next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    ConfigHotkey { keysym, modifiers }
}

/// Reads the config file. The returned bool is `true` when the file could not
/// be read at all (a fresh install), matching the original's `config_empty`.
pub fn read_config() -> (Config, bool) {
    let mut config = Config::default();
    let path = get_config_dir().join("config");

    let Ok(content) = std::fs::read_to_string(&path) else {
        eprintln!("Warning: Failed to read config file: {}", path.display());
        return (config, true);
    };

    for line in content.split('\n') {
        let Some((key, value)) = line.split_once(' ') else {
            if !line.is_empty() {
                eprintln!("Warning: Invalid config option format: {line}");
            }
            continue;
        };
        if key.is_empty() || value.is_empty() {
            continue;
        }
        apply_option(&mut config, key, value);
    }

    (config, false)
}

fn apply_option(config: &mut Config, key: &str, value: &str) {
    let main = &mut config.main_config;
    match key {
        "main.record_area_option" => main.record_area_option = value.to_owned(),
        "main.record_area_width" => main.record_area_width = parse_i32(value),
        "main.record_area_height" => main.record_area_height = parse_i32(value),
        "main.video_width" => main.video_width = parse_i32(value),
        "main.video_height" => main.video_height = parse_i32(value),
        "main.fps" => main.fps = parse_i32(value),
        "main.video_bitrate" => main.video_bitrate = parse_i32(value),
        "main.merge_audio_tracks" => main.merge_audio_tracks = parse_bool(value),
        "main.record_app_audio_inverted" => main.record_app_audio_inverted = parse_bool(value),
        "main.change_video_resolution" => main.change_video_resolution = parse_bool(value),
        "main.audio_input" => main.audio_input.push(value.to_owned()),
        "main.color_range" => main.color_range = value.to_owned(),
        "main.quality" => main.quality = value.to_owned(),
        "main.codec" => main.codec = value.to_owned(),
        "main.audio_codec" => main.audio_codec = value.to_owned(),
        "main.framerate_mode" => main.framerate_mode = value.to_owned(),
        "main.advanced_view" => main.advanced_view = parse_bool(value),
        "main.overclock" => main.overclock = parse_bool(value),
        "main.show_recording_started_notifications" => {
            main.show_recording_started_notifications = parse_bool(value)
        }
        "main.show_recording_stopped_notifications" => {
            main.show_recording_stopped_notifications = parse_bool(value)
        }
        "main.show_recording_saved_notifications" => {
            main.show_recording_saved_notifications = parse_bool(value)
        }
        "main.record_cursor" => main.record_cursor = parse_bool(value),
        "main.hide_window_when_recording" => main.hide_window_when_recording = parse_bool(value),
        "main.software_encoding_warning_shown" => {
            main.software_encoding_warning_shown = parse_bool(value)
        }
        "main.hevc_amd_bug_warning_shown" => main.hevc_amd_bug_warning_shown = parse_bool(value),
        "main.av1_amd_bug_warning_shown" => main.av1_amd_bug_warning_shown = parse_bool(value),
        "main.restore_portal_session" => main.restore_portal_session = parse_bool(value),
        "main.use_new_ui" => main.use_new_ui = parse_bool(value),
        "main.installed_gsr_global_hotkeys_version" => {
            main.installed_gsr_global_hotkeys_version = parse_i32(value)
        }

        "streaming.service" => config.streaming_config.streaming_service = value.to_owned(),
        "streaming.youtube.key" => config.streaming_config.youtube.stream_key = value.to_owned(),
        "streaming.twitch.key" => config.streaming_config.twitch.stream_key = value.to_owned(),
        "streaming.custom.url" => config.streaming_config.custom.url = value.to_owned(),
        "streaming.custom.container" => config.streaming_config.custom.container = value.to_owned(),
        "streaming.start_stop_recording_hotkey" => {
            config.streaming_config.start_stop_recording_hotkey = parse_hotkey(value)
        }

        "record.save_directory" => config.record_config.save_directory = value.to_owned(),
        "record.container" => config.record_config.container = value.to_owned(),
        "record.start_stop_recording_hotkey" => {
            config.record_config.start_stop_recording_hotkey = parse_hotkey(value)
        }
        "record.pause_unpause_recording_hotkey" => {
            config.record_config.pause_unpause_recording_hotkey = parse_hotkey(value)
        }

        "replay.save_directory" => config.replay_config.save_directory = value.to_owned(),
        "replay.container" => config.replay_config.container = value.to_owned(),
        "replay.time" => config.replay_config.replay_time = parse_i32(value),
        "replay.start_stop_recording_hotkey" => {
            config.replay_config.start_stop_recording_hotkey = parse_hotkey(value)
        }
        "replay.save_recording_hotkey" => {
            config.replay_config.save_recording_hotkey = parse_hotkey(value)
        }
        _ => {}
    }
}

pub fn save_config(config: &Config) {
    let dir = get_config_dir();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("Warning: Failed to create config directory: {err}");
        return;
    }

    let mut out = String::with_capacity(2048);
    let main = &config.main_config;
    let mut push = |key: &str, value: String| {
        out.push_str(key);
        out.push(' ');
        out.push_str(&value);
        out.push('\n');
    };

    push("main.record_area_option", main.record_area_option.clone());
    push("main.record_area_width", main.record_area_width.to_string());
    push("main.record_area_height", main.record_area_height.to_string());
    push("main.video_width", main.video_width.to_string());
    push("main.video_height", main.video_height.to_string());
    push("main.fps", main.fps.to_string());
    push("main.video_bitrate", main.video_bitrate.to_string());
    push("main.merge_audio_tracks", bool_str(main.merge_audio_tracks));
    push(
        "main.record_app_audio_inverted",
        bool_str(main.record_app_audio_inverted),
    );
    push(
        "main.change_video_resolution",
        bool_str(main.change_video_resolution),
    );
    for input in &main.audio_input {
        push("main.audio_input", input.clone());
    }
    push("main.color_range", main.color_range.clone());
    push("main.quality", main.quality.clone());
    push("main.codec", main.codec.clone());
    push("main.audio_codec", main.audio_codec.clone());
    push("main.framerate_mode", main.framerate_mode.clone());
    push("main.advanced_view", bool_str(main.advanced_view));
    push("main.overclock", bool_str(main.overclock));
    push(
        "main.show_recording_started_notifications",
        bool_str(main.show_recording_started_notifications),
    );
    push(
        "main.show_recording_stopped_notifications",
        bool_str(main.show_recording_stopped_notifications),
    );
    push(
        "main.show_recording_saved_notifications",
        bool_str(main.show_recording_saved_notifications),
    );
    push("main.record_cursor", bool_str(main.record_cursor));
    push(
        "main.hide_window_when_recording",
        bool_str(main.hide_window_when_recording),
    );
    push(
        "main.software_encoding_warning_shown",
        bool_str(main.software_encoding_warning_shown),
    );
    push(
        "main.hevc_amd_bug_warning_shown",
        bool_str(main.hevc_amd_bug_warning_shown),
    );
    push(
        "main.av1_amd_bug_warning_shown",
        bool_str(main.av1_amd_bug_warning_shown),
    );
    push(
        "main.restore_portal_session",
        bool_str(main.restore_portal_session),
    );
    push("main.use_new_ui", bool_str(main.use_new_ui));
    push(
        "main.installed_gsr_global_hotkeys_version",
        main.installed_gsr_global_hotkeys_version.to_string(),
    );

    let streaming = &config.streaming_config;
    push("streaming.service", streaming.streaming_service.clone());
    push("streaming.youtube.key", streaming.youtube.stream_key.clone());
    push("streaming.twitch.key", streaming.twitch.stream_key.clone());
    push("streaming.custom.url", streaming.custom.url.clone());
    push(
        "streaming.custom.container",
        streaming.custom.container.clone(),
    );
    push(
        "streaming.start_stop_recording_hotkey",
        hotkey_str(streaming.start_stop_recording_hotkey),
    );

    let record = &config.record_config;
    push("record.save_directory", record.save_directory.clone());
    push("record.container", record.container.clone());
    push(
        "record.start_stop_recording_hotkey",
        hotkey_str(record.start_stop_recording_hotkey),
    );
    push(
        "record.pause_unpause_recording_hotkey",
        hotkey_str(record.pause_unpause_recording_hotkey),
    );

    let replay = &config.replay_config;
    push("replay.save_directory", replay.save_directory.clone());
    push("replay.container", replay.container.clone());
    push("replay.time", replay.replay_time.to_string());
    push(
        "replay.start_stop_recording_hotkey",
        hotkey_str(replay.start_stop_recording_hotkey),
    );
    push(
        "replay.save_recording_hotkey",
        hotkey_str(replay.save_recording_hotkey),
    );

    drop(push);

    let path = dir.join("config");
    if let Err(err) = std::fs::write(&path, out) {
        eprintln!("Warning: Failed to create config file: {err}");
    }
}

fn bool_str(value: bool) -> String {
    if value { "true" } else { "false" }.to_owned()
}

fn hotkey_str(hotkey: ConfigHotkey) -> String {
    format!("{} {}", hotkey.keysym, hotkey.modifiers)
}
