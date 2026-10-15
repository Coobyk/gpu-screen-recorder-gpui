//! The application root view: widget state, the four pages, and the
//! `gpu-screen-recorder` child process lifecycle.
//!
//! Behaviour mirrors the Qt frontend in `src/main.cpp`, so the sections below
//! follow that file's structure: the common settings page, the recording, replay
//! and streaming pages, the argument builders, and the 500 ms timer that reaps
//! the child process and drives the elapsed-time labels.

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{
    Anchor, AnchoredPositionMode, AnyElement, App, Context, Entity, IntoElement, Render,
    SharedString, Window, anchored, deferred, div, point, prelude::*, px, rgba,
};

use crate::config::{self, Config};
use crate::events::{AppEvent, EventChannel, ShortcutId};
use crate::gsr::{self, GsrInfo, Recorder, RunMode};
use crate::notify::Notifier;
use crate::text_input::{InputFilter, TextInput};
use crate::theme;
use crate::tray::{GsrTray, TrayPage, TrayState};
use crate::widgets::{self, ButtonKind};

/// The four pages, in the same order as the Qt frontend's `QStackedWidget`
/// (0 = common settings, 1 = replay, 2 = recording, 3 = streaming).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Common,
    Record,
    Replay,
    Stream,
}

impl Page {
    fn tray_page(self) -> TrayPage {
        match self {
            Page::Common => TrayPage::Front,
            Page::Record => TrayPage::Recording,
            Page::Replay => TrayPage::Replay,
            Page::Stream => TrayPage::Streaming,
        }
    }
}

/// Identifies a dropdown, so one renderer can serve every combo box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectKey {
    RecordArea,
    View,
    Quality,
    Codec,
    AudioCodec,
    ColorRange,
    FramerateMode,
    StreamService,
    RecordContainer,
    ReplayContainer,
    CustomContainer,
    AudioDevice(usize),
    AudioApp(usize),
}

/// One row of the audio list. Mirrors the Qt frontend's `audio-track-type`
/// widget property (`device` / `app` / `app-custom`).
enum AudioRow {
    Device { selected: usize },
    App { selected: usize },
    Custom { input: Entity<TextInput> },
}

/// A modal message box, equivalent to the Qt frontend's `QMessageBox` calls.
struct Dialog {
    title: SharedString,
    message: SharedString,
}

/// How the recorder child process ended.
#[derive(Clone, Copy, Debug)]
struct ProcessExit {
    code: i32,
    success: bool,
}

impl ProcessExit {
    fn from_status(status: Option<std::process::ExitStatus>) -> Self {
        match status {
            Some(status) => Self {
                code: status.code().unwrap_or(-1),
                success: status.success(),
            },
            // No child was running: the Qt frontend treats this as success.
            None => Self {
                code: -1,
                success: true,
            },
        }
    }
}

const VIEWS: &[(&str, &str)] = &[("Simple", "simple"), ("Advanced", "advanced")];
const QUALITIES: &[(&str, &str)] = &[
    (
        "Constant bitrate (Recommended for live streaming and replay)",
        "custom",
    ),
    ("Medium", "medium"),
    ("High", "high"),
    ("Very High (Recommended for recording)", "very_high"),
    ("Ultra", "ultra"),
];
const COLOR_RANGES: &[(&str, &str)] = &[("Limited", "limited"), ("Full", "full")];
const AUDIO_CODECS: &[(&str, &str)] = &[("Opus (Recommended)", "opus"), ("AAC", "aac")];
const FRAMERATE_MODES: &[(&str, &str)] = &[
    ("Auto (Recommended)", "auto"),
    ("Constant", "cfr"),
    ("Variable", "vfr"),
];
const STREAM_SERVICES: &[(&str, &str)] =
    &[("Twitch", "twitch"), ("Youtube", "youtube"), ("Custom", "custom")];

const LABEL_WIDTH: f32 = 170.;

pub struct GsrApp {
    config: Config,
    gsr_info: GsrInfo,
    page: Page,

    // Common page state.
    record_area: usize,
    view: usize,
    quality: usize,
    codec: usize,
    audio_codec: usize,
    color_range: usize,
    framerate_mode: usize,
    change_video_resolution: bool,
    restore_portal_session: bool,
    split_audio: bool,
    invert_app_audio: bool,
    record_cursor: bool,
    overclock: bool,
    show_started_notifications: bool,
    show_stopped_notifications: bool,
    show_saved_notifications: bool,
    hide_window_when_recording: bool,

    // Recording / replay / streaming page state.
    record_container: usize,
    replay_container: usize,
    custom_container: usize,
    stream_service: usize,

    // Text fields.
    area_width: Entity<TextInput>,
    area_height: Entity<TextInput>,
    video_width: Entity<TextInput>,
    video_height: Entity<TextInput>,
    fps: Entity<TextInput>,
    video_bitrate: Entity<TextInput>,
    replay_time: Entity<TextInput>,
    record_dir: Entity<TextInput>,
    replay_dir: Entity<TextInput>,
    youtube_key: Entity<TextInput>,
    twitch_key: Entity<TextInput>,
    custom_url: Entity<TextInput>,

    audio_rows: Vec<AudioRow>,
    audio_inputs: Vec<gsr::AudioInput>,
    application_audio: Vec<String>,

    open_select: Option<SelectKey>,
    dialog: Option<Dialog>,

    // Recorder lifecycle.
    recorder: Option<Recorder>,
    recording: bool,
    paused: bool,
    replaying: bool,
    streaming: bool,
    prev_exit_status: i32,
    record_start: Option<Instant>,
    paused_offset: Duration,
    pause_started: Option<Instant>,
    current_filename: String,
    record_timer: SharedString,
    replay_timer: SharedString,
    stream_timer: SharedString,

    window_hidden: bool,

    tray_state: Arc<Mutex<TrayState>>,
    tray_handle: Option<ksni::blocking::Handle<GsrTray>>,
    notifier: Notifier,
    events: Arc<Mutex<Receiver<AppEvent>>>,
}

impl GsrApp {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: Config,
        config_empty: bool,
        gsr_info: GsrInfo,
        events: EventChannel,
        tray_state: Arc<Mutex<TrayState>>,
        tray_handle: Option<ksni::blocking::Handle<GsrTray>>,
        notifier: Notifier,
        cx: &mut Context<Self>,
    ) -> Self {
        let audio_inputs = gsr::list_audio_devices();
        let application_audio = gsr::list_application_audio();

        let main = config.main_config.clone();
        let replay = config.replay_config.clone();

        let area_width = if main.record_area_width == 0 {
            1920
        } else {
            main.record_area_width
        };
        let area_height = if main.record_area_height == 0 {
            1080
        } else {
            main.record_area_height
        };
        let video_width = if main.video_width == 0 {
            1920
        } else {
            main.video_width
        };
        let video_height = if main.video_height == 0 {
            1080
        } else {
            main.video_height
        };
        let fps = if main.fps <= 0 { 60 } else { main.fps };
        let video_bitrate = if main.video_bitrate <= 0 {
            15000
        } else {
            main.video_bitrate
        };
        let replay_time = replay.replay_time.clamp(5, 1200);

        let record_directory = if config.record_config.save_directory.is_empty() {
            config::get_videos_dir().to_string_lossy().into_owned()
        } else {
            config.record_config.save_directory.clone()
        };
        let replay_directory = if config.replay_config.save_directory.is_empty() {
            config::get_videos_dir().to_string_lossy().into_owned()
        } else {
            config.replay_config.save_directory.clone()
        };

        let mut app = Self {
            record_area: 0,
            view: index_of(VIEWS, if main.advanced_view { "advanced" } else { "simple" }),
            quality: index_of(QUALITIES, &normalize(&main.quality, QUALITIES, "very_high")),
            codec: 0,
            audio_codec: index_of(AUDIO_CODECS, &normalize(&main.audio_codec, AUDIO_CODECS, "opus")),
            color_range: index_of(
                COLOR_RANGES,
                &normalize(&main.color_range, COLOR_RANGES, "limited"),
            ),
            framerate_mode: index_of(
                FRAMERATE_MODES,
                &normalize(&main.framerate_mode, FRAMERATE_MODES, "auto"),
            ),
            change_video_resolution: main.change_video_resolution,
            restore_portal_session: main.restore_portal_session,
            split_audio: !main.merge_audio_tracks,
            invert_app_audio: main.record_app_audio_inverted,
            record_cursor: main.record_cursor,
            overclock: main.overclock,
            show_started_notifications: main.show_recording_started_notifications,
            show_stopped_notifications: main.show_recording_stopped_notifications,
            show_saved_notifications: main.show_recording_saved_notifications,
            hide_window_when_recording: main.hide_window_when_recording,
            record_container: 0,
            replay_container: 0,
            custom_container: 0,
            stream_service: index_of(
                STREAM_SERVICES,
                &config.streaming_config.streaming_service,
            ),
            area_width: new_number_field(cx, area_width),
            area_height: new_number_field(cx, area_height),
            video_width: new_number_field(cx, video_width),
            video_height: new_number_field(cx, video_height),
            fps: new_number_field(cx, fps),
            video_bitrate: new_number_field(cx, video_bitrate),
            replay_time: new_number_field(cx, replay_time),
            record_dir: new_text_field(cx, record_directory, "Where to save recordings"),
            replay_dir: new_text_field(cx, replay_directory, "Where to save replays"),
            youtube_key: new_masked_field(cx, config.streaming_config.youtube.stream_key.clone()),
            twitch_key: new_masked_field(cx, config.streaming_config.twitch.stream_key.clone()),
            custom_url: new_text_field(cx, config.streaming_config.custom.url.clone(), "rtmp://"),
            audio_rows: Vec::new(),
            audio_inputs,
            application_audio,
            open_select: None,
            dialog: None,
            recorder: None,
            recording: false,
            paused: false,
            replaying: false,
            streaming: false,
            prev_exit_status: -1,
            record_start: None,
            paused_offset: Duration::ZERO,
            pause_started: None,
            current_filename: String::new(),
            record_timer: "00:00:00".into(),
            replay_timer: "00:00:00".into(),
            stream_timer: "00:00:00".into(),
            window_hidden: false,
            tray_state,
            tray_handle,
            notifier,
            events: Arc::new(Mutex::new(events.receiver)),
            config,
            gsr_info,
            page: Page::Common,
        };

        // Record area selection (falls back to the first monitor, then portal).
        let area = if app.config.main_config.record_area_option.is_empty() {
            app.record_areas()
                .first()
                .map(|(_, value)| value.clone())
                .unwrap_or_else(|| "portal".to_owned())
        } else {
            app.config.main_config.record_area_option.clone()
        };
        app.record_area = index_of_owned(&app.record_areas(), &area);

        // Codec: clamp "auto" through the availability list.
        app.codec = {
            let options = app.codec_options();
            let wanted = app.config.main_config.codec.clone();
            let wanted = if wanted.is_empty() { "auto" } else { &wanted };
            index_of_owned(&options, wanted)
        };
        app.record_container = container_index(&app.container_options(), &app.config.record_config.container);
        app.replay_container = container_index(&app.container_options(), &app.config.replay_config.container);
        app.custom_container = container_index(
            &app.container_options(),
            &app.config.streaming_config.custom.container,
        );

        // Audio tracks.
        let inputs = app.config.main_config.audio_input.clone();
        for input in &inputs {
            if let Some(name) = input.strip_prefix("app:") {
                let matched = app
                    .application_audio
                    .iter()
                    .find(|entry| entry.eq_ignore_ascii_case(name))
                    .cloned();
                match matched {
                    Some(entry) => {
                        let selected = app
                            .application_audio
                            .iter()
                            .position(|candidate| *candidate == entry)
                            .unwrap_or(0);
                        app.audio_rows.push(AudioRow::App { selected });
                    }
                    None => app.audio_rows.push(AudioRow::Custom {
                        input: new_text_field(cx, name.to_owned(), "Application name"),
                    }),
                }
            } else if let Some(name) = input.strip_prefix("device:") {
                let selected = app
                    .audio_inputs
                    .iter()
                    .position(|candidate| candidate.name == name)
                    .unwrap_or(0);
                app.audio_rows.push(AudioRow::Device { selected });
            } else {
                let selected = app
                    .audio_inputs
                    .iter()
                    .position(|candidate| candidate.name == *input)
                    .unwrap_or(0);
                app.audio_rows.push(AudioRow::Device { selected });
            }
        }
        if config_empty && app.audio_rows.is_empty() {
            let selected = app
                .audio_inputs
                .iter()
                .position(|candidate| candidate.name == "Default output")
                .unwrap_or(0);
            app.audio_rows.push(AudioRow::Device { selected });
        }

        // First run without a hardware encoder: warn and switch to software H264,
        // exactly as the Qt frontend does.
        if !app.config.main_config.software_encoding_warning_shown
            && app.gsr_info.supported_video_codecs.has_no_hardware_codec()
        {
            app.config.main_config.software_encoding_warning_shown = true;
            app.view = index_of(VIEWS, "advanced");
            app.codec = index_of_owned(&app.codec_options(), "h264_software");
            app.dialog = Some(Dialog {
                title: "GPU Screen Recorder".into(),
                message: "Unable to find a hardware video encoder on your system, using software \
                          video encoder instead (slow!). If you have an AMD, Intel or NVIDIA GPU \
                          then your GPU driver may be too old or not installed correctly."
                    .into(),
            });
        }

        app
    }

    // ---------------------------------------------------------------- helpers

    fn record_areas(&self) -> Vec<(String, String)> {
        let mut options = Vec::new();
        for monitor in &self.gsr_info.supported_capture_options.monitors {
            options.push((
                format!("Monitor {} ({}x{})", monitor.name, monitor.width, monitor.height),
                monitor.name.clone(),
            ));
        }
        let portal_label = if self.gsr_info.supported_capture_options.portal {
            "Desktop portal (HDR not supported)"
        } else {
            "Desktop portal (Not available on your system)"
        };
        options.push((portal_label.to_owned(), "portal".to_owned()));
        options
    }

    fn container_options(&self) -> Vec<(String, String)> {
        let mut options: Vec<(String, String)> = gsr::SUPPORTED_CONTAINERS
            .iter()
            .map(|container| (container.extension.to_owned(), container.name.to_owned()))
            .collect();
        if self.gsr_info.supported_video_codecs.vp8 || self.gsr_info.supported_video_codecs.vp9 {
            options.push(("webm".to_owned(), "webm".to_owned()));
        }
        options
    }

    fn codec_options(&self) -> Vec<(String, String)> {
        let codecs = &self.gsr_info.supported_video_codecs;
        let mut options = vec![("Auto (Recommended)".to_owned(), "auto".to_owned())];
        let mut push = |label: &str, id: &str, available: bool| {
            options.push((
                if available {
                    label.to_owned()
                } else {
                    format!("{label} (Not available on your system)")
                },
                id.to_owned(),
            ));
        };
        push(
            "H264 (Largest file size, best software compatibility)",
            "h264",
            codecs.h264,
        );
        push("HEVC", "hevc", codecs.hevc);
        push("HEVC (10 bit, reduces banding)", "hevc_10bit", codecs.hevc);
        push("HEVC (HDR)", "hevc_hdr", codecs.hevc);
        push(
            "AV1 (Smallest file size, worst software compatibility)",
            "av1",
            codecs.av1,
        );
        push("AV1 (10 bit, reduces banding)", "av1_10bit", codecs.av1);
        push("AV1 (HDR)", "av1_hdr", codecs.av1);
        push("VP8", "vp8", codecs.vp8);
        push("VP9", "vp9", codecs.vp9);
        push(
            "H264 Software Encoder (Not recommended, slow)",
            "h264_software",
            codecs.h264_software,
        );
        options
    }

    fn options(&self, key: SelectKey) -> Vec<(String, String)> {
        match key {
            SelectKey::RecordArea => self.record_areas(),
            SelectKey::View => pairs(VIEWS),
            SelectKey::Quality => pairs(QUALITIES),
            SelectKey::Codec => self.codec_options(),
            SelectKey::AudioCodec => pairs(AUDIO_CODECS),
            SelectKey::ColorRange => pairs(COLOR_RANGES),
            SelectKey::FramerateMode => pairs(FRAMERATE_MODES),
            SelectKey::StreamService => pairs(STREAM_SERVICES),
            SelectKey::RecordContainer | SelectKey::ReplayContainer | SelectKey::CustomContainer => {
                self.container_options()
            }
            SelectKey::AudioDevice(_) => self
                .audio_inputs
                .iter()
                .map(|input| (input.description.clone(), input.name.clone()))
                .collect(),
            SelectKey::AudioApp(_) => self
                .application_audio
                .iter()
                .map(|name| (name.clone(), name.clone()))
                .collect(),
        }
    }

    fn index(&self, key: SelectKey) -> usize {
        match key {
            SelectKey::RecordArea => self.record_area,
            SelectKey::View => self.view,
            SelectKey::Quality => self.quality,
            SelectKey::Codec => self.codec,
            SelectKey::AudioCodec => self.audio_codec,
            SelectKey::ColorRange => self.color_range,
            SelectKey::FramerateMode => self.framerate_mode,
            SelectKey::StreamService => self.stream_service,
            SelectKey::RecordContainer => self.record_container,
            SelectKey::ReplayContainer => self.replay_container,
            SelectKey::CustomContainer => self.custom_container,
            SelectKey::AudioDevice(row) => match self.audio_rows.get(row) {
                Some(AudioRow::Device { selected }) => *selected,
                _ => 0,
            },
            SelectKey::AudioApp(row) => match self.audio_rows.get(row) {
                Some(AudioRow::App { selected }) => *selected,
                _ => 0,
            },
        }
    }

    fn set_index(&mut self, key: SelectKey, index: usize) {
        match key {
            SelectKey::RecordArea => self.record_area = index,
            SelectKey::View => {
                self.view = index;
                self.config.main_config.advanced_view = self.value_of(SelectKey::View) == "advanced";
            }
            SelectKey::Quality => self.quality = index,
            SelectKey::Codec => self.codec = index,
            SelectKey::AudioCodec => self.audio_codec = index,
            SelectKey::ColorRange => self.color_range = index,
            SelectKey::FramerateMode => self.framerate_mode = index,
            SelectKey::StreamService => self.stream_service = index,
            SelectKey::RecordContainer => self.record_container = index,
            SelectKey::ReplayContainer => self.replay_container = index,
            SelectKey::CustomContainer => self.custom_container = index,
            SelectKey::AudioDevice(row) => {
                if let Some(AudioRow::Device { selected }) = self.audio_rows.get_mut(row) {
                    *selected = index;
                }
            }
            SelectKey::AudioApp(row) => {
                if let Some(AudioRow::App { selected }) = self.audio_rows.get_mut(row) {
                    *selected = index;
                }
            }
        }
    }

    /// The option value (`findData` in Qt) of the current selection.
    fn value_of(&self, key: SelectKey) -> String {
        let options = self.options(key);
        if options.is_empty() {
            return String::new();
        }
        let index = self.index(key).min(options.len() - 1);
        options[index].1.clone()
    }

    /// The option label (`currentText` in Qt) of the current selection.
    fn label_of(&self, key: SelectKey) -> String {
        let options = self.options(key);
        if options.is_empty() {
            return String::new();
        }
        let index = self.index(key).min(options.len() - 1);
        options[index].0.clone()
    }

    fn read_int(&self, field: &Entity<TextInput>, min: i32, max: i32, fallback: i32, cx: &App) -> i32 {
        field
            .read(cx)
            .value_i32()
            .unwrap_or(fallback)
            .clamp(min, max)
    }

    fn is_advanced(&self) -> bool {
        self.value_of(SelectKey::View) == "advanced"
    }

    fn supports_app_audio(&self) -> bool {
        self.gsr_info.system_info.supports_app_audio
    }

    fn on_wayland(&self) -> bool {
        self.gsr_info.system_info.display_server == gsr::DisplayServer::Wayland
    }

    fn on_x11(&self) -> bool {
        self.gsr_info.system_info.display_server == gsr::DisplayServer::X11
    }

    // ------------------------------------------------------------- lifecycle

    fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.sync_tray(cx);
        cx.notify();
    }

    fn sync_tray(&mut self, cx: &mut Context<Self>) {
        {
            let mut state = self.tray_state.lock().unwrap();
            state.window_hidden = self.window_hidden;
            state.hide_window_when_recording = self.hide_when_recording_enabled();
            state.recording = self.recording;
            state.paused = self.paused;
            state.replaying = self.replaying;
            state.streaming = self.streaming;
            state.page = self.page.tray_page();
        }
        // The guard must be released before this call: refreshing the menu locks
        // the same state from the tray thread.
        if let Some(handle) = &self.tray_handle {
            let _ = handle.update(|_| {});
        }
        let _ = cx;
    }

    fn hide_when_recording_enabled(&self) -> bool {
        self.hide_window_when_recording
    }

    fn notify(&self, title: &str, body: impl Into<String>, urgent: bool) {
        self.notifier.show(title, body, urgent);
    }

    fn toggle_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_hidden {
            window.activate_window();
            self.window_hidden = false;
        } else {
            window.minimize_window();
            self.window_hidden = true;
        }
        self.sync_tray(cx);
    }

    /// `closeEvent` in the Qt frontend: the window is always hidden, never closed.
    pub fn on_window_hidden(&mut self, cx: &mut Context<Self>) {
        self.window_hidden = true;
        self.sync_tray(cx);
    }

    pub fn handle_event(
        &mut self,
        event: AppEvent,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        // Events that only need the window are handled first, so the interesting
        // ones can borrow it.
        let mut window = window;
        match event {
            AppEvent::ToggleWindow => {
                if let Some(window) = window.as_deref_mut() {
                    self.toggle_window(window, cx);
                }
                return;
            }
            AppEvent::QuitApplication => {
                self.shutdown(cx);
                return;
            }
            _ => {}
        }

        match event {
            AppEvent::SetHideWindowWhenRecording(value) => {
                self.hide_window_when_recording = value;
                self.sync_tray(cx);
                cx.notify();
            }
            AppEvent::ToggleStreaming => self.toggle_streaming(window, cx),
            AppEvent::ToggleRecording => self.toggle_recording(window, cx),
            AppEvent::ToggleReplay => self.toggle_replay(window, cx),
            AppEvent::TogglePause => self.toggle_pause(cx),
            AppEvent::SaveReplay => self.save_replay(cx),
            AppEvent::ShortcutActivated(id) => self.on_shortcut(id, cx),
            AppEvent::ToggleWindow | AppEvent::QuitApplication => {}
        }
    }

    /// Stops any running recorder, writes the config out and quits. This is the
    /// Qt frontend's `~MainWindow`, which sent SIGINT so the recorder could
    /// finalize the file before the process went away.
    fn shutdown(&mut self, cx: &mut Context<Self>) {
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.stop();
        }
        self.recorder = None;
        self.save_config(cx);
        cx.quit();
    }

    fn on_shortcut(&mut self, id: ShortcutId, cx: &mut Context<Self>) {
        let page = self.page;
        match id {
            ShortcutId::StartStopRecording => match page {
                Page::Record => self.toggle_recording(None, cx),
                Page::Replay => self.toggle_replay(None, cx),
                Page::Stream => self.toggle_streaming(None, cx),
                Page::Common => {}
            },
            ShortcutId::PauseUnpauseRecording => {
                if page == Page::Record {
                    self.toggle_pause(cx);
                }
            }
            ShortcutId::SaveReplay => {
                if page == Page::Replay {
                    self.save_replay(cx);
                }
            }
        }
    }

    // ------------------------------------------------------------- recording

    fn toggle_recording(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.recording {
            let exit = ProcessExit::from_status(
                self.recorder
                    .as_mut()
                    .and_then(|recorder| recorder.stop()),
            );
            self.finish_recording(exit, cx);
            return;
        }
        self.start_recording(window, cx);
    }

    fn start_recording(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.save_config(cx);

        let (args, filename) = match self.build_recorder_args(cx, RunMode::Recording) {
            Ok(result) => result,
            Err(message) => {
                self.notify("GPU Screen Recorder", message, true);
                return;
            }
        };

        match Recorder::spawn(&args) {
            Ok(recorder) => self.recorder = Some(recorder),
            Err(err) => {
                self.notify(
                    "GPU Screen Recorder",
                    format!("Failed to start recording: {err}"),
                    true,
                );
                return;
            }
        }

        self.current_filename = filename;
        self.recording = true;
        self.paused = false;
        self.record_start = Some(Instant::now());
        self.paused_offset = Duration::ZERO;
        self.record_timer = "00:00:00".into();

        if self.hide_when_recording_enabled() {
            if let Some(window) = window {
                window.minimize_window();
            }
            self.window_hidden = true;
        }

        if self.show_started_notifications {
            self.notify("GPU Screen Recorder", "Started recording", false);
        }

        self.sync_tray(cx);
        cx.notify();
    }

    fn finish_recording(&mut self, exit: ProcessExit, cx: &mut Context<Self>) {
        self.recorder = None;
        self.recording = false;
        self.paused = false;
        self.record_start = None;
        self.paused_offset = Duration::ZERO;
        self.record_timer = "00:00:00".into();
        self.prev_exit_status = exit.code;

        if exit.code == 10 {
            self.notify(
                "GPU Screen Recorder",
                "You need to have pkexec installed and have a polkit agent running to record your \
                 monitor",
                true,
            );
        } else if exit.code == 50 {
            self.notify("GPU Screen Recorder", "Desktop portal capture failed.", true);
        } else if exit.code != 60 && !exit.success {
            self.notify("GPU Screen Recorder", "Failed to save video.", true);
        } else if exit.success && exit.code == 0 && self.show_saved_notifications {
            self.notify(
                "GPU Screen Recorder",
                format!("The recording was saved to {}", self.current_filename),
                false,
            );
        }

        self.sync_tray(cx);
        cx.notify();
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if !self.recording {
            return;
        }
        let Some(recorder) = self.recorder.as_mut() else {
            return;
        };
        recorder.toggle_pause();
        self.paused = !self.paused;
        if self.paused {
            self.pause_started = Some(Instant::now());
        } else if let Some(started) = self.pause_started.take() {
            self.paused_offset += started.elapsed();
        }
        self.sync_tray(cx);
        cx.notify();
    }

    // ----------------------------------------------------------------- replay

    fn toggle_replay(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.replaying {
            let exit = ProcessExit::from_status(
                self.recorder
                    .as_mut()
                    .and_then(|recorder| recorder.stop()),
            );
            self.finish_replay(exit, cx);
            return;
        }
        self.start_replay(window, cx);
    }

    fn start_replay(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.save_config(cx);

        let args = match self.build_recorder_args(cx, RunMode::Replay) {
            Ok((args, _)) => args,
            Err(message) => {
                self.notify("GPU Screen Recorder", message, true);
                return;
            }
        };

        match Recorder::spawn(&args) {
            Ok(recorder) => self.recorder = Some(recorder),
            Err(err) => {
                self.notify(
                    "GPU Screen Recorder",
                    format!("Failed to start replay: {err}"),
                    true,
                );
                return;
            }
        }

        self.replaying = true;
        self.record_start = Some(Instant::now());
        self.replay_timer = "00:00:00".into();

        if self.hide_when_recording_enabled() {
            if let Some(window) = window {
                window.minimize_window();
            }
            self.window_hidden = true;
        }

        if self.show_started_notifications {
            self.notify("GPU Screen Recorder", "Started replay", false);
        }

        self.sync_tray(cx);
        cx.notify();
    }

    fn finish_replay(&mut self, exit: ProcessExit, cx: &mut Context<Self>) {
        self.recorder = None;
        self.replaying = false;
        self.record_start = None;
        self.replay_timer = "00:00:00".into();
        self.prev_exit_status = exit.code;

        if exit.code == 10 {
            self.notify(
                "GPU Screen Recorder",
                "You need to have pkexec installed and have a polkit agent running to record your \
                 monitor",
                true,
            );
        } else if exit.code == 50 {
            self.notify("GPU Screen Recorder", "Desktop portal capture failed.", true);
        } else if exit.code != 60 && !exit.success {
            self.notify("GPU Screen Recorder", "Failed to start replay.", true);
        } else if exit.success && exit.code == 0 && self.show_stopped_notifications {
            self.notify("GPU Screen Recorder", "Stopped replay", false);
        }

        self.sync_tray(cx);
        cx.notify();
    }

    fn save_replay(&mut self, cx: &mut Context<Self>) {
        let Some(recorder) = self.recorder.as_mut() else {
            return;
        };
        recorder.save_replay();
        if self.show_saved_notifications {
            self.notify("GPU Screen Recorder", "Saved replay", false);
        }
        let _ = cx;
    }

    // -------------------------------------------------------------- streaming

    /// The front page's "Stream" button: validate the audio setup, then switch.
    fn open_stream_page(&mut self, cx: &mut Context<Self>) {
        let num_audio_tracks = self
            .audio_rows
            .iter()
            .filter(|row| matches!(row, AudioRow::Device { .. }))
            .count();

        if num_audio_tracks > 1 && self.split_audio {
            self.dialog = Some(Dialog {
                title: "GPU Screen Recorder".into(),
                message: "Streaming doesn't work with more than 1 audio track. Please remove all \
                          audio tracks or only use 1 audio track or select to merge audio tracks."
                    .into(),
            });
            return;
        }

        self.set_page(Page::Stream, cx);
    }

    fn toggle_streaming(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.streaming {
            let exit = ProcessExit::from_status(
                self.recorder
                    .as_mut()
                    .and_then(|recorder| recorder.stop()),
            );
            self.finish_streaming(exit, cx);
            return;
        }
        self.start_streaming(window, cx);
    }

    fn start_streaming(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.save_config(cx);

        let args = match self.build_recorder_args(cx, RunMode::Streaming) {
            Ok((args, _)) => args,
            Err(message) => {
                self.notify("GPU Screen Recorder", message, true);
                return;
            }
        };

        match Recorder::spawn(&args) {
            Ok(recorder) => self.recorder = Some(recorder),
            Err(err) => {
                self.notify(
                    "GPU Screen Recorder",
                    format!("Failed to stream video: {err}"),
                    true,
                );
                return;
            }
        }

        self.streaming = true;
        self.record_start = Some(Instant::now());
        self.stream_timer = "00:00:00".into();

        if self.hide_when_recording_enabled() {
            if let Some(window) = window {
                window.minimize_window();
            }
            self.window_hidden = true;
        }

        if self.show_started_notifications {
            self.notify("GPU Screen Recorder", "Started streaming", false);
        }

        self.sync_tray(cx);
        cx.notify();
    }

    fn finish_streaming(&mut self, exit: ProcessExit, cx: &mut Context<Self>) {
        self.recorder = None;
        self.streaming = false;
        self.record_start = None;
        self.stream_timer = "00:00:00".into();
        self.prev_exit_status = exit.code;

        if exit.code == 10 {
            self.notify(
                "GPU Screen Recorder",
                "You need to have pkexec installed and have a polkit agent running to record your \
                 monitor",
                true,
            );
        } else if exit.code == 50 {
            self.notify("GPU Screen Recorder", "Desktop portal capture failed.", true);
        } else if exit.code != 60 && !exit.success {
            self.notify("GPU Screen Recorder", "Failed to stream video.", true);
        } else if exit.success && exit.code == 0 && self.show_stopped_notifications {
            self.notify("GPU Screen Recorder", "Stopped streaming", false);
        }

        self.sync_tray(cx);
        cx.notify();
    }

    // --------------------------------------------------------- argument build

    /// Builds the `gpu-screen-recorder` command line, mirroring
    /// `onStartRecordButtonClicked` / `onStartReplayButtonClicked` /
    /// `onStartStreamButtonClicked`. Returns the arguments and, for recordings,
    /// the output file name.
    fn build_recorder_args(
        &self,
        cx: &App,
        mode: RunMode,
    ) -> Result<(Vec<String>, String), String> {
        let fps = self.read_int(&self.fps, 1, 500, 60, cx);
        let area = self.value_of(SelectKey::RecordArea);
        let color_range = self.value_of(SelectKey::ColorRange);
        let quality = self.value_of(SelectKey::Quality);
        let audio_codec = self.value_of(SelectKey::AudioCodec);
        let framerate_mode = self.value_of(SelectKey::FramerateMode);
        let video_bitrate = self.read_int(&self.video_bitrate, 1, 500_000, 15_000, cx);
        let record_width = self.read_int(&self.video_width, 5, 10_000, 1920, cx);
        let record_height = self.read_int(&self.video_height, 5, 10_000, 1080, cx);

        let codecs = &self.gsr_info.supported_video_codecs;
        let mut encoder = "gpu";
        let mut video_codec = self.value_of(SelectKey::Codec);
        if video_codec == "h264_software"
            || (video_codec == "auto" && codecs.has_no_hardware_codec())
        {
            video_codec = "h264".to_owned();
            encoder = "cpu";
        }

        let mut container;
        let output;
        let mut current_filename = String::new();

        match mode {
            RunMode::Recording => {
                container = self.value_of(SelectKey::RecordContainer);
                let mut extension = self.label_of(SelectKey::RecordContainer);
                if is_webm_codec(&video_codec) && !is_webm_container(&container) {
                    eprintln!(
                        "warning: container '{container}' not compatible with '{video_codec}', \
                         using webm"
                    );
                    container = "webm".to_owned();
                    extension = "webm".to_owned();
                }
                let directory = self.record_dir.read(cx).content().to_string();
                if let Err(err) = gsr::create_directory(Path::new(&directory)) {
                    return Err(format!(
                        "Failed to start recording. Failed to create {directory}: {err}"
                    ));
                }
                current_filename = format!(
                    "{directory}/Video_{}.{extension}",
                    gsr::get_date_str()
                );
                output = current_filename.clone();
            }
            RunMode::Replay => {
                container = self.value_of(SelectKey::ReplayContainer);
                if is_webm_codec(&video_codec) && !is_webm_container(&container) {
                    container = "webm".to_owned();
                }
                let directory = self.replay_dir.read(cx).content().to_string();
                if let Err(err) = gsr::create_directory(Path::new(&directory)) {
                    return Err(format!(
                        "Failed to start replay. Failed to create {directory}: {err}"
                    ));
                }
                output = directory;
            }
            RunMode::Streaming => {
                let service = self.value_of(SelectKey::StreamService);
                container = "flv".to_owned();
                output = match service.as_str() {
                    "twitch" => {
                        format!(
                            "rtmp://live.twitch.tv/app/{}",
                            self.twitch_key.read(cx).content()
                        )
                    }
                    "youtube" => {
                        format!(
                            "rtmp://a.rtmp.youtube.com/live2/{}",
                            self.youtube_key.read(cx).content()
                        )
                    }
                    "custom" => {
                        container = self.value_of(SelectKey::CustomContainer);
                        let url = self.custom_url.read(cx).content().to_string();
                        if has_stream_scheme(&url) {
                            url
                        } else {
                            format!("rtmp://{url}")
                        }
                    }
                    _ => {
                        return Err("Failed to stream video: no streaming service selected".to_owned());
                    }
                };
                if is_webm_codec(&video_codec) && !is_webm_container(&container) {
                    container = "webm".to_owned();
                }
            }
        }

        let mut args: Vec<String> = vec![
            "-w".to_owned(),
            area,
            "-c".to_owned(),
            container,
            "-k".to_owned(),
            video_codec,
            "-ac".to_owned(),
            audio_codec,
            "-f".to_owned(),
            fps.to_string(),
            "-cursor".to_owned(),
            yes_no(self.record_cursor),
            "-restore-portal-session".to_owned(),
            yes_no(self.restore_portal_session),
            "-cr".to_owned(),
            color_range,
        ];

        match mode {
            RunMode::Replay => {
                args.push("-r".to_owned());
                args.push(self.read_int(&self.replay_time, 5, 1200, 30, cx).to_string());
            }
            RunMode::Recording | RunMode::Streaming => {}
        }

        args.push("-encoder".to_owned());
        args.push(encoder.to_owned());
        args.push("-o".to_owned());
        args.push(output);

        if quality == "custom" {
            args.push("-bm".to_owned());
            args.push("cbr".to_owned());
            args.push("-q".to_owned());
            args.push(video_bitrate.to_string());
        } else {
            args.push("-q".to_owned());
            args.push(quality);
        }

        if self.overclock {
            args.push("-oc".to_owned());
            args.push("yes".to_owned());
        }
        if framerate_mode != "auto" {
            args.push("-fm".to_owned());
            args.push(framerate_mode);
        }

        let audio_tracks = self.audio_tracks(cx);
        if self.split_audio {
            for track in &audio_tracks {
                args.push("-a".to_owned());
                args.push(track.clone());
            }
        } else if !audio_tracks.is_empty() {
            args.push("-a".to_owned());
            args.push(audio_tracks.join("|"));
        }

        if self.change_video_resolution {
            args.push("-s".to_owned());
            args.push(format!("{record_width}x{record_height}"));
        }

        Ok((args, current_filename))
    }

    /// The `-a` values, following the Qt frontend's merge/invert rules.
    fn audio_tracks(&self, cx: &App) -> Vec<String> {
        let mut tracks = Vec::new();
        let invert = self.invert_app_audio;
        let mut num_app_audio = 0;

        for (index, row) in self.audio_rows.iter().enumerate() {
            match row {
                AudioRow::Device { .. } => {
                    tracks.push(format!(
                        "device:{}",
                        self.value_of(SelectKey::AudioDevice(index))
                    ));
                }
                AudioRow::App { .. } => {
                    if !self.supports_app_audio() {
                        continue;
                    }
                    let name = self.value_of(SelectKey::AudioApp(index));
                    tracks.push(format!("{}{}", app_audio_prefix(invert), name));
                    num_app_audio += 1;
                }
                AudioRow::Custom { input } => {
                    if !self.supports_app_audio() {
                        continue;
                    }
                    tracks.push(format!(
                        "{}{}",
                        app_audio_prefix(invert),
                        input.read(cx).content()
                    ));
                    num_app_audio += 1;
                }
            }
        }

        if num_app_audio == 0 && invert {
            tracks.push("app-inverse:".to_owned());
        }

        tracks
    }

    // ------------------------------------------------------------ config I/O

    /// `saveConfigs` in the Qt frontend: copy the widget state into the config
    /// and write it out. Called before every capture starts, and on exit.
    fn save_config(&mut self, cx: &App) {
        // Take the section out while reading the widget state, so the immutable
        // readers (`value_of`, `is_advanced`, `read_int`) do not overlap with the
        // mutable borrow of `self.config`.
        let mut main = std::mem::take(&mut self.config.main_config);
        main.record_area_option = self.value_of(SelectKey::RecordArea);
        if !self.on_x11() {
            main.record_area_width = self.read_int(&self.area_width, 5, 10_000, 1920, cx);
            main.record_area_height = self.read_int(&self.area_height, 5, 10_000, 1080, cx);
        }
        main.video_width = self.read_int(&self.video_width, 5, 10_000, 1920, cx);
        main.video_height = self.read_int(&self.video_height, 5, 10_000, 1080, cx);
        main.fps = self.read_int(&self.fps, 1, 500, 60, cx);
        main.video_bitrate = self.read_int(&self.video_bitrate, 1, 500_000, 15_000, cx);
        main.merge_audio_tracks = !self.split_audio;
        main.record_app_audio_inverted = self.invert_app_audio;
        main.change_video_resolution = self.change_video_resolution;
        main.color_range = self.value_of(SelectKey::ColorRange);
        main.quality = self.value_of(SelectKey::Quality);
        main.codec = self.value_of(SelectKey::Codec);
        main.audio_codec = self.value_of(SelectKey::AudioCodec);
        main.framerate_mode = self.value_of(SelectKey::FramerateMode);
        main.advanced_view = self.is_advanced();
        main.overclock = self.overclock;
        main.show_recording_started_notifications = self.show_started_notifications;
        main.show_recording_stopped_notifications = self.show_stopped_notifications;
        main.show_recording_saved_notifications = self.show_saved_notifications;
        main.record_cursor = self.record_cursor;
        main.hide_window_when_recording = self.hide_window_when_recording;
        main.restore_portal_session = self.restore_portal_session;

        main.audio_input = self
            .audio_rows
            .iter()
            .enumerate()
            .map(|(index, row)| match row {
                AudioRow::Device { .. } => {
                    format!("device:{}", self.value_of(SelectKey::AudioDevice(index)))
                }
                AudioRow::App { .. } => {
                    format!("app:{}", self.value_of(SelectKey::AudioApp(index)))
                }
                AudioRow::Custom { input } => {
                    format!("app:{}", input.read(cx).content())
                }
            })
            .collect();

        self.config.main_config = main;

        self.config.streaming_config.streaming_service = self.value_of(SelectKey::StreamService);
        self.config.streaming_config.youtube.stream_key =
            self.youtube_key.read(cx).content().to_string();
        self.config.streaming_config.twitch.stream_key =
            self.twitch_key.read(cx).content().to_string();
        self.config.streaming_config.custom.url = self.custom_url.read(cx).content().to_string();
        self.config.streaming_config.custom.container = self.value_of(SelectKey::CustomContainer);

        self.config.record_config.save_directory = self.record_dir.read(cx).content().to_string();
        self.config.record_config.container = self.value_of(SelectKey::RecordContainer);

        self.config.replay_config.save_directory = self.replay_dir.read(cx).content().to_string();
        self.config.replay_config.container = self.value_of(SelectKey::ReplayContainer);
        self.config.replay_config.replay_time = self.read_int(&self.replay_time, 5, 1200, 30, cx);

        config::save_config(&self.config);
    }

    /// Drains the out-of-band event queue and advances the recorder timers.
    ///
    /// Called from the UI-thread background task every 200 ms; the tray and the
    /// shortcut portal have no other way to reach GPUI state.
    pub fn pump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let events: Vec<AppEvent> = match self.events.lock() {
            Ok(receiver) => receiver.try_iter().collect(),
            Err(_) => return,
        };

        for event in events {
            self.handle_event(event, Some(&mut *window), cx);
        }

        self.tick(cx);
    }

    // ------------------------------------------------------------------- tick

    /// Runs every 200 ms, like the Qt frontend's 500 ms `QTimer`: reap the
    /// recorder child and refresh the elapsed-time labels.
    fn tick(&mut self, cx: &mut Context<Self>) {
        let exited = match self.recorder.as_mut() {
            Some(recorder) => recorder.try_wait(),
            None => None,
        };

        if let Some(status) = exited {
            self.recorder = None;
            let exit = ProcessExit::from_status(Some(status));
            if self.replaying {
                self.finish_replay(exit, cx);
            } else if self.recording {
                self.finish_recording(exit, cx);
            } else if self.streaming {
                self.finish_streaming(exit, cx);
            }
        }

        let mut changed = false;

        if (self.streaming || self.replaying)
            && let Some(start) = self.record_start
        {
            let text = format_elapsed(start.elapsed());
            if self.streaming && self.stream_timer != text {
                self.stream_timer = text.clone().into();
                changed = true;
            }
            if self.replaying && self.replay_timer != text {
                self.replay_timer = text.into();
                changed = true;
            }
        }

        if self.recording && !self.paused
            && let Some(start) = self.record_start
        {
            let elapsed = start.elapsed().saturating_sub(self.paused_offset);
            let text = format_elapsed(elapsed);
            if self.record_timer != text {
                self.record_timer = text.into();
                changed = true;
            }
        }

        if changed {
            cx.notify();
        }
    }

    // --------------------------------------------------------------- rendering

    fn select(&self, key: SelectKey, cx: &mut Context<Self>) -> AnyElement {
        let options = self.options(key);
        let selected = self.index(key).min(options.len().saturating_sub(1));
        let label: SharedString = options
            .get(selected)
            .map(|(label, _)| label.clone())
            .unwrap_or_default()
            .into();
        let open = self.open_select == Some(key);

        let trigger = div()
            .id(SharedString::from(format!("select-{key:?}")))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .h(px(30.))
            .w_full()
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(if open { theme::ACCENT } else { theme::BORDER })
            .bg(theme::FIELD)
            .text_sm()
            .text_color(theme::TEXT)
            .cursor_pointer()
            .child(div().flex_1().min_w(px(0.)).truncate().child(label))
            .child(div().text_color(theme::TEXT_MUTED).child("\u{25be}"))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.open_select = if this.open_select == Some(key) {
                    None
                } else {
                    Some(key)
                };
                cx.notify();
            }));

        let mut container = div().relative().w_full().child(trigger);

        if open && !options.is_empty() {
            let items: Vec<AnyElement> = options
                .iter()
                .enumerate()
                .map(|(index, (label, _))| {
                    let label = label.clone();
                    let is_selected = index == selected;
                    div()
                        .id(SharedString::from(format!("select-{key:?}-item-{index}")))
                        .w_full()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .text_color(if is_selected {
                            theme::ACCENT
                        } else {
                            theme::TEXT
                        })
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::BORDER))
                        .child(label)
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.set_index(key, index);
                            this.open_select = None;
                            cx.notify();
                        }))
                        .into_any_element()
                })
                .collect();

            container = container.child(
                deferred(
                    anchored()
                        .anchor(Anchor::TopLeft)
                        .position(point(px(0.), px(4.)))
                        .position_mode(AnchoredPositionMode::Local)
                        .snap_to_window()
                        .child(
                            div()
                                .id("select-menu")
                                .flex()
                                .flex_col()
                                .w_full()
                                .min_w(px(240.))
                                .max_h(px(280.))
                                .overflow_y_scroll()
                                .p_1()
                                .bg(theme::PANEL_ALT)
                                .border_1()
                                .border_color(theme::BORDER)
                                .rounded_md()
                                .shadow_lg()
                                .children(items)
                                // Swallow mouse events so the trigger (and any other
                                // control) beneath the popup does not also react to a
                                // click that lands on the popup's overlap area.
                                .occlude()
                                .on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                                    this.open_select = None;
                                    cx.notify();
                                })),
                        ),
                )
                .priority(2),
            );
        }

        container.into_any_element()
    }

    fn field_row(&self, text: &str, control: AnyElement) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(LABEL_WIDTH))
                    .flex_none()
                    .child(widgets::label(text.to_owned())),
            )
            .child(div().flex_1().min_w(px(0.)).child(control))
            .into_any_element()
    }

    fn audio_row_elements(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        for (index, row) in self.audio_rows.iter().enumerate() {
            let control: AnyElement = match row {
                AudioRow::Device { .. } => self.select(SelectKey::AudioDevice(index), cx),
                AudioRow::App { .. } => self.select(SelectKey::AudioApp(index), cx),
                AudioRow::Custom { input } => {
                    div().child(input.clone()).into_any_element()
                }
            };
            let prefix = match row {
                AudioRow::Device { .. } => "Device:",
                AudioRow::App { .. } | AudioRow::Custom { .. } => "Application:",
            };
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(90.))
                            .flex_none()
                            .child(widgets::muted(prefix.to_owned())),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(control))
                    .child(widgets::button(
                        format!("audio-remove-{index}"),
                        "Remove",
                        ButtonKind::Secondary,
                        true,
                        cx.listener(move |this, _event, _window, cx| {
                            if index < this.audio_rows.len() {
                                this.audio_rows.remove(index);
                                cx.notify();
                            }
                        }),
                    ))
                    .into_any_element(),
            );
        }
        rows
    }

    fn render_common(&self, cx: &mut Context<Self>) -> AnyElement {
        let advanced = self.is_advanced();
        let area = self.value_of(SelectKey::RecordArea);
        let is_focused = area == "focused";
        let show_video_resolution = self.change_video_resolution && !is_focused;

        let mut capture: Vec<AnyElement> = vec![self.field_row(
            "Record area",
            self.select(SelectKey::RecordArea, cx),
        )];

        if !is_focused {
            capture.push(self.field_row(
                "Video resolution limit",
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(90.)).child(self.area_width.clone()))
                    .child(widgets::muted("x"))
                    .child(div().w(px(90.)).child(self.area_height.clone()))
                    .into_any_element(),
            ));
            capture.push(widgets::checkbox(
                "change-resolution",
                "Change video resolution",
                self.change_video_resolution,
                cx.listener(|this, _event, _window, cx| {
                    this.change_video_resolution = !this.change_video_resolution;
                    cx.notify();
                }),
            ));
        }

        if show_video_resolution {
            capture.push(self.field_row(
                "Video resolution",
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(90.)).child(self.video_width.clone()))
                    .child(widgets::muted("x"))
                    .child(div().w(px(90.)).child(self.video_height.clone()))
                    .into_any_element(),
            ));
        }

        if area == "portal" {
            capture.push(widgets::checkbox(
                "restore-portal",
                "Restore portal session",
                self.restore_portal_session,
                cx.listener(|this, _event, _window, cx| {
                    this.restore_portal_session = !this.restore_portal_session;
                    cx.notify();
                }),
            ));
        }

        let mut audio = self.audio_row_elements(cx);
        audio.push(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(widgets::button(
                    "add-audio-device",
                    "Add audio device",
                    ButtonKind::Secondary,
                    true,
                    cx.listener(|this, _event, _window, cx| {
                        this.audio_inputs = gsr::list_audio_devices();
                        this.audio_rows.push(AudioRow::Device { selected: 0 });
                        cx.notify();
                    }),
                ))
                .when(self.supports_app_audio(), |element| {
                    element
                        .child(widgets::button(
                            "add-app-audio",
                            "Add application audio",
                            ButtonKind::Secondary,
                            true,
                            cx.listener(|this, _event, _window, cx| {
                                this.application_audio = gsr::list_application_audio();
                                this.audio_rows.push(AudioRow::App { selected: 0 });
                                cx.notify();
                            }),
                        ))
                        .child(widgets::button(
                            "add-custom-app-audio",
                            "Add custom application audio",
                            ButtonKind::Secondary,
                            true,
                            cx.listener(|this, _event, _window, cx| {
                                let input = cx.new(|cx| {
                                    TextInput::new(cx, "", "Application name", InputFilter::Any, false)
                                });
                                this.audio_rows.push(AudioRow::Custom { input });
                                cx.notify();
                            }),
                        ))
                })
                .into_any_element(),
        );

        if advanced {
            audio.push(widgets::checkbox(
                "split-audio",
                "Split each device/app audio into separate audio tracks",
                self.split_audio,
                cx.listener(|this, _event, _window, cx| {
                    this.split_audio = !this.split_audio;
                    cx.notify();
                }),
            ));
        }

        if self.supports_app_audio() {
            audio.push(widgets::checkbox(
                "invert-app-audio",
                "Record audio from all applications except the selected ones",
                self.invert_app_audio,
                cx.listener(|this, _event, _window, cx| {
                    this.invert_app_audio = !this.invert_app_audio;
                    cx.notify();
                }),
            ));
        }

        if advanced {
            audio.push(self.field_row(
                "Audio codec",
                self.select(SelectKey::AudioCodec, cx),
            ));
        }

        let mut video = vec![self.field_row("Video quality", self.select(SelectKey::Quality, cx))];

        if self.value_of(SelectKey::Quality) == "custom" {
            video.push(self.field_row(
                "Video bitrate (kbps)",
                div().child(self.video_bitrate.clone()).into_any_element(),
            ));
        }

        if advanced {
            video.push(self.field_row("Video codec", self.select(SelectKey::Codec, cx)));
            video.push(self.field_row("Color range", self.select(SelectKey::ColorRange, cx)));
        }

        video.push(self.field_row(
            "Frame rate",
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().w(px(90.)).child(self.fps.clone()))
                .into_any_element(),
        ));

        if advanced {
            video.push(self.field_row(
                "Frame rate mode",
                self.select(SelectKey::FramerateMode, cx),
            ));
        }

        if advanced && self.overclock_available() {
            video.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(widgets::checkbox(
                        "overclock",
                        "Overclock memory transfer rate to workaround NVIDIA driver performance bug",
                        self.overclock,
                        cx.listener(|this, _event, _window, cx| {
                            this.overclock = !this.overclock;
                            cx.notify();
                        }),
                    ))
                    .child(widgets::button(
                        "overclock-help",
                        "?",
                        ButtonKind::Secondary,
                        true,
                        cx.listener(|this, _event, _window, cx| {
                            this.dialog = Some(Dialog {
                                title: "GPU Screen Recorder".into(),
                                message: "NVIDIA driver has a bug where it will downclock memory \
                                          transfer rate when a program uses CUDA. To work around \
                                          this, GPU Screen Recorder can overclock your GPU memory \
                                          transfer rate to its normal optimal level. You also need \
                                          to have \"Coolbits\" NVIDIA X setting set to \"12\" to \
                                          enable overclocking. You can set coolbits by running \
                                          \"sudo nvidia-xconfig --cool-bits=12\" and then rebooting."
                                    .into(),
                            });
                            cx.notify();
                        }),
                    ))
                    .into_any_element(),
            );
        }

        video.push(widgets::checkbox(
            "record-cursor",
            "Record cursor",
            self.record_cursor,
            cx.listener(|this, _event, _window, cx| {
                this.record_cursor = !this.record_cursor;
                cx.notify();
            }),
        ));

        let mut sections = vec![
            widgets::section("Capture target", capture),
            widgets::section("Audio", audio),
            widgets::section("Video", video),
        ];

        if advanced {
            sections.push(widgets::section(
                "Notifications",
                vec![
                    widgets::checkbox(
                        "notify-started",
                        "Show recording/streaming/replay started notification",
                        self.show_started_notifications,
                        cx.listener(|this, _event, _window, cx| {
                            this.show_started_notifications = !this.show_started_notifications;
                            cx.notify();
                        }),
                    ),
                    widgets::checkbox(
                        "notify-stopped",
                        "Show streaming/replay stopped notification",
                        self.show_stopped_notifications,
                        cx.listener(|this, _event, _window, cx| {
                            this.show_stopped_notifications = !this.show_stopped_notifications;
                            cx.notify();
                        }),
                    ),
                    widgets::checkbox(
                        "notify-saved",
                        "Show video saved notification",
                        self.show_saved_notifications,
                        cx.listener(|this, _event, _window, cx| {
                            this.show_saved_notifications = !this.show_saved_notifications;
                            cx.notify();
                        }),
                    ),
                ],
            ));
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_2()
            .child(self.field_row("View", self.select(SelectKey::View, cx)))
            .child(
                div()
                    .id("common-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(sections),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(widgets::button(
                        "open-stream",
                        "Stream",
                        ButtonKind::Secondary,
                        true,
                        cx.listener(|this, _event, _window, cx| this.open_stream_page(cx)),
                    ))
                    .child(widgets::button(
                        "open-record",
                        "Record",
                        ButtonKind::Secondary,
                        true,
                        cx.listener(|this, _event, _window, cx| this.set_page(Page::Record, cx)),
                    ))
                    .child(widgets::button(
                        "open-replay",
                        "Replay",
                        ButtonKind::Secondary,
                        true,
                        cx.listener(|this, _event, _window, cx| this.set_page(Page::Replay, cx)),
                    )),
            )
            .into_any_element()
    }

    fn overclock_available(&self) -> bool {
        self.gsr_info.gpu_info.vendor == gsr::GpuVendor::Nvidia
            && self.gsr_info.system_info.display_server != gsr::DisplayServer::Wayland
    }

    fn render_record(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut children: Vec<AnyElement> = Vec::new();

        if self.on_wayland() {
            children.push(
                div()
                    .text_sm()
                    .text_color(theme::TEXT_MUTED)
                    .child(
                        "Your Wayland compositor doesn't support global hotkeys. Use KDE Plasma \
                         on Wayland if you want to use hotkeys.",
                    )
                    .into_any_element(),
            );
        }

        children.push(widgets::separator());
        children.push(self.field_row(
            "Save video to",
            div().child(self.record_dir.clone()).into_any_element(),
        ));
        children.push(self.field_row(
            "Container",
            self.select(SelectKey::RecordContainer, cx),
        ));

        children.push(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(widgets::button(
                    "start-recording",
                    if self.recording {
                        "Stop recording"
                    } else {
                        "Start recording"
                    },
                    if self.recording {
                        ButtonKind::Danger
                    } else {
                        ButtonKind::Primary
                    },
                    true,
                    cx.listener(|this, _event, window, cx| {
                        this.toggle_recording(Some(window), cx)
                    }),
                ))
                .child(widgets::button(
                    "pause-recording",
                    if self.paused {
                        "Unpause recording"
                    } else {
                        "Pause recording"
                    },
                    ButtonKind::Secondary,
                    self.recording,
                    cx.listener(|this, _event, _window, cx| this.toggle_pause(cx)),
                ))
                .into_any_element(),
        );

        children.push(widgets::separator());
        children.push(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_color(if self.recording {
                            theme::DANGER
                        } else {
                            theme::TEXT_MUTED
                        })
                        .child("\u{25cf}"),
                )
                .child(
                    div()
                        .text_base()
                        .text_color(if self.recording {
                            theme::TEXT
                        } else {
                            theme::TEXT_MUTED
                        })
                        .child(self.record_timer.clone()),
                )
                .into_any_element(),
        );

        page_shell("Recording", Some(self.back_button("record-back", cx)), children)
    }

    fn render_replay(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut children: Vec<AnyElement> = Vec::new();

        if self.on_wayland() {
            children.push(
                div()
                    .text_sm()
                    .text_color(theme::TEXT_MUTED)
                    .child(
                        "Your Wayland compositor doesn't support global hotkeys. Use KDE Plasma \
                         on Wayland if you want to use hotkeys.",
                    )
                    .into_any_element(),
            );
        }

        children.push(widgets::separator());
        children.push(self.field_row(
            "Save replays to",
            div().child(self.replay_dir.clone()).into_any_element(),
        ));
        children.push(self.field_row(
            "Container",
            self.select(SelectKey::ReplayContainer, cx),
        ));
        children.push(self.field_row(
            "Replay time in seconds",
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().w(px(90.)).child(self.replay_time.clone()))
                .into_any_element(),
        ));

        children.push(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(widgets::button(
                    "start-replay",
                    if self.replaying {
                        "Stop replay"
                    } else {
                        "Start replay"
                    },
                    if self.replaying {
                        ButtonKind::Danger
                    } else {
                        ButtonKind::Primary
                    },
                    true,
                    cx.listener(|this, _event, window, cx| this.toggle_replay(Some(window), cx)),
                ))
                .child(widgets::button(
                    "save-replay",
                    "Save replay",
                    ButtonKind::Secondary,
                    self.replaying,
                    cx.listener(|this, _event, _window, cx| this.save_replay(cx)),
                ))
                .into_any_element(),
        );

        children.push(widgets::separator());
        children.push(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(widgets::muted("\u{25b6}"))
                .child(self.replay_timer.clone())
                .into_any_element(),
        );

        page_shell("Replay", Some(self.back_button("replay-back", cx)), children)
    }

    fn render_stream(&self, cx: &mut Context<Self>) -> AnyElement {
        let service = self.value_of(SelectKey::StreamService);

        let mut children: Vec<AnyElement> = Vec::new();

        children.push(self.field_row(
            "Stream service",
            self.select(SelectKey::StreamService, cx),
        ));

        let reveal = |field: &Entity<TextInput>, id: &str, cx: &mut Context<Self>| -> AnyElement {
            let masked = field.read(cx).masked();
            widgets::button(
                SharedString::from(id.to_owned()),
                if masked { "Show" } else { "Hide" },
                ButtonKind::Secondary,
                true,
                {
                    let field = field.clone();
                    cx.listener(move |_this, _event, _window, cx| {
                        field.update(cx, |input, cx| {
                            let masked = input.masked();
                            input.set_masked(!masked);
                            cx.notify();
                        });
                    })
                },
            )
        };

        match service.as_str() {
            "youtube" => {
                children.push(self.field_row(
                    "Stream key",
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().min_w(px(0.)).child(self.youtube_key.clone()))
                        .child(reveal(&self.youtube_key, "reveal-youtube", cx))
                        .into_any_element(),
                ));
            }
            "twitch" => {
                children.push(self.field_row(
                    "Stream key",
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().min_w(px(0.)).child(self.twitch_key.clone()))
                        .child(reveal(&self.twitch_key, "reveal-twitch", cx))
                        .into_any_element(),
                ));
            }
            "custom" => {
                children.push(self.field_row(
                    "Url",
                    div().child(self.custom_url.clone()).into_any_element(),
                ));
                children.push(self.field_row(
                    "Container",
                    self.select(SelectKey::CustomContainer, cx),
                ));
            }
            _ => {}
        }

        children.push(widgets::separator());
        children.push(
            div()
                .flex()
                .flex_row()
                .child(widgets::button(
                    "start-streaming",
                    if self.streaming {
                        "Stop streaming"
                    } else {
                        "Start streaming"
                    },
                    if self.streaming {
                        ButtonKind::Danger
                    } else {
                        ButtonKind::Primary
                    },
                    true,
                    cx.listener(|this, _event, window, cx| {
                        this.toggle_streaming(Some(window), cx)
                    }),
                ))
                .into_any_element(),
        );

        children.push(widgets::separator());
        children.push(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(widgets::muted("\u{25b6}"))
                .child(self.stream_timer.clone())
                .into_any_element(),
        );

        page_shell("Streaming", Some(self.back_button("stream-back", cx)), children)
    }

    /// The "< Back" button shared by the recording, replay and streaming pages.
    fn back_button(&self, id: &'static str, cx: &mut Context<Self>) -> AnyElement {
        widgets::button(
            id,
            "< Back",
            ButtonKind::Secondary,
            true,
            cx.listener(|this, _event, _window, cx| this.set_page(Page::Common, cx)),
        )
    }

    fn render_dialog(&self, dialog: &Dialog, cx: &mut Context<Self>) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00_00_00_aa))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w(px(460.))
                    .p_4()
                    .bg(theme::PANEL)
                    .border_1()
                    .border_color(theme::BORDER)
                    .rounded_lg()
                    .child(widgets::heading(dialog.title.clone()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::TEXT)
                            .child(dialog.message.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .child(widgets::button(
                                "dialog-ok",
                                "OK",
                                ButtonKind::Primary,
                                true,
                                cx.listener(|this, _event, _window, cx| {
                                    this.dialog = None;
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .into_any_element()
    }
}

impl Render for GsrApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vendor = self.gsr_info.gpu_info.vendor.name();
        window.set_window_title(&format!("GPU Screen Recorder | Running on {vendor}"));

        let content = match self.page {
            Page::Common => self.render_common(cx),
            Page::Record => self.render_record(cx),
            Page::Replay => self.render_replay(cx),
            Page::Stream => self.render_stream(cx),
        };

        let dialog = self
            .dialog
            .as_ref()
            .map(|dialog| self.render_dialog(dialog, cx));

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::BG)
            .text_color(theme::TEXT)
            .p_3()
            .child(div().flex_1().min_h(px(0.)).child(content))
            .when_some(dialog, |element, dialog| element.child(dialog))
    }
}

// ------------------------------------------------------------------- helpers

/// A page body: an optional back button and a heading, followed by the page's
/// rows, scrolled if needed.
fn page_shell(title: &str, back: Option<AnyElement>, children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .size_full()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .when_some(back, |row, back| row.child(back))
                .child(widgets::heading(title.to_owned())),
        )
        .child(
            div()
                .id("page-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_3()
                .children(children),
        )
        .into_any_element()
}

fn pairs(table: &[(&str, &str)]) -> Vec<(String, String)> {
    table
        .iter()
        .map(|(label, value)| ((*label).to_owned(), (*value).to_owned()))
        .collect()
}

fn index_of(table: &[(&str, &str)], value: &str) -> usize {
    table
        .iter()
        .position(|(_, candidate)| *candidate == value)
        .unwrap_or(0)
}

fn index_of_owned(options: &[(String, String)], value: &str) -> usize {
    options
        .iter()
        .position(|(_, candidate)| candidate == value)
        .unwrap_or(0)
}

/// Containers default to the first entry when unset or unknown.
fn container_index(options: &[(String, String)], value: &str) -> usize {
    if value.is_empty() {
        return 0;
    }
    index_of_owned(options, value)
}

/// Returns `value` when it is one of the table's values, otherwise `fallback`.
fn normalize(value: &str, table: &[(&str, &str)], fallback: &str) -> String {
    if table.iter().any(|(_, candidate)| *candidate == value) {
        value.to_owned()
    } else {
        fallback.to_owned()
    }
}

fn new_number_field(cx: &mut Context<GsrApp>, value: i32) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(cx, value.to_string(), "", InputFilter::Digits, false))
}

fn new_text_field(
    cx: &mut Context<GsrApp>,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(cx, value, placeholder, InputFilter::Any, false))
}

fn new_masked_field(cx: &mut Context<GsrApp>, value: impl Into<SharedString>) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(cx, value, "Stream key", InputFilter::Any, true))
}

fn yes_no(value: bool) -> String {
    if value { "yes" } else { "no" }.to_owned()
}

fn is_webm_codec(codec: &str) -> bool {
    codec == "vp8" || codec == "vp9"
}

fn is_webm_container(container: &str) -> bool {
    container == "webm" || container == "matroska"
}

fn app_audio_prefix(inverted: bool) -> &'static str {
    if inverted {
        "app-inverse:"
    } else {
        "app:"
    }
}

/// True when the custom stream URL already carries a scheme the recorder takes.
fn has_stream_scheme(url: &str) -> bool {
    const SCHEMES: &[&str] = &[
        "rtmp://", "rtmps://", "rtsp://", "srt://", "http://", "https://", "tcp://", "udp://",
    ];
    SCHEMES.iter().any(|scheme| url.starts_with(scheme))
}

/// `HH:MM:SS`, formatted like the Qt frontend's timer labels.
fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}
