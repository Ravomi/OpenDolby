#[path = "dolby_gui/assets.rs"]
mod assets;
#[path = "dolby_gui/instance.rs"]
mod instance;
#[path = "dolby_gui/view.rs"]
mod view;

use clap::Parser;
use dolby_rs::default_xml_path;
use dolby_rs::dsp::DolbyProcessor;
use dolby_rs::pw_sink::{spawn_pipewire_sink, EngineHandle};
use dolby_rs::tuning::DeviceTuning;
use gpui_kit::component::{
    slider::{SliderEvent, SliderState},
    Root, Theme, ThemeMode,
};
use gpui_kit::{
    prelude::*, px, rgb, size, Bounds, Context, Entity, SharedString, Subscription, Window,
    WindowBounds, WindowOptions,
};
use std::{path::PathBuf, process::Command};

#[derive(Parser)]
#[command(name = "dolby-gui", about = "OpenDolby native PipeWire control panel")]
struct GuiArgs {
    /// Local OEM tuning XML. Defaults to OPENDOLBY_XML or the user config directory.
    #[arg(long)]
    xml: Option<PathBuf>,
    /// Start the Dolby audio engine via the running instance (or start GUI + engine).
    #[arg(long)]
    start: bool,
    /// Stop the Dolby audio engine and restore the speaker output via the running instance.
    #[arg(long)]
    stop: bool,
    /// Print JSON status of the running instance and exit.
    #[arg(long)]
    status: bool,
    /// Stop the Dolby audio engine, restore the speaker output, and quit the background instance.
    #[arg(long)]
    quit: bool,
}

const OUR_SINK_PATTERN: &str = "Dolby Atmos Rust";
const SPEAKER_SINK_PATTERN: &str = "Raptor Lake-P/U/H cAVS Speaker";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Sound,
    Output,
    About,
}

struct DolbyGui {
    tuning: Option<DeviceTuning>,
    active_profile: String,
    page: Page,
    show_details: bool,
    engine: Option<EngineHandle>,
    bypass: bool,
    level_match: bool,
    surround_amount: f32,
    dialog_amount: f32,
    surround_slider: Entity<SliderState>,
    dialog_slider: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
    we_changed_default: bool,
    status: SharedString,
}

impl DolbyGui {
    fn new(xml: &std::path::Path, cx: &mut Context<Self>) -> Self {
        let (tuning, status) = match DeviceTuning::from_xml_file(xml) {
            Ok(t) => (Some(t), "就绪，启动引擎即可使用".into()),
            Err(error) => {
                eprintln!("Failed to load tuning XML {}: {error:#}", xml.display());
                (
                    None,
                    "未加载调音文件，请通过 --xml 或 OPENDOLBY_XML 指定本机 XML".into(),
                )
            }
        };
        let surround_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(2.0)
                .step(0.05)
                .default_value(1.0)
        });
        let dialog_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(2.0)
                .step(0.05)
                .default_value(1.0)
        });
        let sub_surround = cx.subscribe(&surround_slider, |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(val) = event {
                this.set_surround_amount(val.start());
                cx.notify();
            }
        });
        let sub_dialog = cx.subscribe(&dialog_slider, |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(val) = event {
                this.set_dialog_amount(val.start());
                cx.notify();
            }
        });
        Self {
            tuning,
            active_profile: "dynamic".into(),
            page: Page::Sound,
            show_details: false,
            engine: None,
            bypass: false,
            level_match: true,
            surround_amount: 1.0,
            dialog_amount: 1.0,
            surround_slider,
            dialog_slider,
            _subscriptions: vec![sub_surround, sub_dialog],
            we_changed_default: false,
            status,
        }
    }

    fn is_engine_running(&self) -> bool {
        self.engine.is_some()
    }

    fn status_json(&self, window_open: bool) -> String {
        format!(
            r#"{{"pid":{},"running":{},"profile":"{}","window_open":{}}}"#,
            std::process::id(),
            self.engine.is_some(),
            self.active_profile,
            window_open
        )
    }

    fn apply_live_controls(&self, handle: &EngineHandle) {
        handle.set_bypass(self.bypass);
        handle.set_level_match(self.level_match);
        handle.set_surround_amount(self.surround_amount);
        handle.set_dialog_amount(self.dialog_amount);
    }

    fn toggle_engine(&mut self, enable: bool) {
        if enable {
            if self.engine.is_some() {
                return;
            }
            let Some(ref tuning) = self.tuning else {
                self.status = "无法启动: 未加载调音文件".into();
                return;
            };
            let Some(proc) = DolbyProcessor::from_tuning(tuning, &self.active_profile, 48000)
            else {
                self.status = format!("未找到配置 '{}'", self.active_profile).into();
                return;
            };
            match spawn_pipewire_sink(proc, None, 48000) {
                Ok(handle) => {
                    self.apply_live_controls(&handle);
                    self.engine = Some(handle);
                    self.status = "引擎已启动，可在输出设备中选择 OpenDolby".into();
                }
                Err(e) => self.status = format!("PipeWire 启动失败: {e}").into(),
            }
        } else {
            self.restore_speaker_if_our_sink_default();
            if let Some(mut handle) = self.engine.take() {
                handle.stop();
            }
            self.status = "引擎已停止，已移除虚拟输出".into();
        }
    }

    fn select_profile(&mut self, profile: &str) {
        self.active_profile = profile.into();
        self.page = Page::Sound;
        if let (Some(tuning), Some(handle)) = (&self.tuning, &self.engine) {
            if let Some(proc) = DolbyProcessor::from_tuning(tuning, profile, 48000) {
                handle.set_processor(proc);
                self.apply_live_controls(handle);
                self.status = format!("配置已实时切换 · {}", view::profile(profile).title).into();
            }
        } else {
            self.status = "配置已选择，启动引擎后生效".into();
        }
    }

    fn toggle_bypass(&mut self, bypass: bool) {
        self.bypass = bypass;
        if let Some(handle) = &self.engine {
            handle.set_bypass(bypass);
        }
        self.status = if bypass {
            "旁路试听 · 保留匹配开关和安全限幅".into()
        } else {
            "音效处理已启用".into()
        };
    }

    fn toggle_level_match(&mut self, enabled: bool) {
        self.level_match = enabled;
        if let Some(handle) = &self.engine {
            handle.set_level_match(enabled);
        }
        self.status = if enabled {
            "RMS 匹配已开启，请等待约 1–2 秒稳定".into()
        } else {
            "RMS 匹配已关闭，注意音量变化".into()
        };
    }

    fn set_surround_amount(&mut self, amount: f32) {
        self.surround_amount = amount.clamp(0.0, 2.0);
        if let Some(handle) = &self.engine {
            handle.set_surround_amount(self.surround_amount);
        }
    }
    fn set_dialog_amount(&mut self, amount: f32) {
        self.dialog_amount = amount.clamp(0.0, 2.0);
        if let Some(handle) = &self.engine {
            handle.set_dialog_amount(self.dialog_amount);
        }
    }

    fn reset_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_surround_amount(1.0);
        self.set_dialog_amount(1.0);
        self.toggle_bypass(false);
        self.toggle_level_match(true);
        self.surround_slider
            .update(cx, |slider, cx| slider.set_value(1.0, window, cx));
        self.dialog_slider
            .update(cx, |slider, cx| slider.set_value(1.0, window, cx));
        self.status = "已恢复配置基准 · 环绕 1.00x / 人声 1.00x".into();
        cx.notify();
    }

    fn set_default_sink(&mut self, pattern: &str, label: &str, ours: bool) {
        match find_wpctl_sink_id(pattern) {
            Some(id) => match Command::new("wpctl").args(["set-default", &id]).status() {
                Ok(s) if s.success() => {
                    self.we_changed_default = ours;
                    self.status = format!("默认输出已切换为 {label}").into();
                }
                Ok(s) => self.status = format!("输出切换失败: {s}").into(),
                Err(e) => self.status = format!("无法运行 wpctl: {e}").into(),
            },
            None => self.status = format!("未找到输出设备: {label}").into(),
        }
    }

    fn restore_speaker_if_our_sink_default(&mut self) {
        if is_sink_default(OUR_SINK_PATTERN) {
            if let Some(id) = find_wpctl_sink_id(SPEAKER_SINK_PATTERN) {
                if Command::new("wpctl")
                    .args(["set-default", &id])
                    .status()
                    .is_ok_and(|s| s.success())
                {
                    self.we_changed_default = false;
                }
            }
        }
    }
}

impl Drop for DolbyGui {
    fn drop(&mut self) {
        if self.we_changed_default {
            self.restore_speaker_if_our_sink_default();
        }
        if let Some(mut handle) = self.engine.take() {
            handle.stop();
        }
    }
}

fn find_wpctl_sink_id(pattern: &str) -> Option<String> {
    let out = Command::new("wpctl").arg("status").output().ok()?;
    parse_wpctl_sink_id(&String::from_utf8_lossy(&out.stdout), pattern)
}
fn parse_wpctl_sink_id(text: &str, pattern: &str) -> Option<String> {
    let mut in_sinks = false;
    for line in text.lines() {
        if line.contains("Sinks:") {
            in_sinks = true;
            continue;
        }
        if in_sinks && (line.contains("Sources:") || line.contains("Filters:")) {
            break;
        }
        if in_sinks && line.contains(pattern) {
            let trimmed = line.trim_start_matches(|c: char| !c.is_ascii_digit());
            if let Some((num, _)) = trimmed.split_once('.') {
                if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
                    return Some(num.into());
                }
            }
        }
    }
    None
}
fn is_sink_default(pattern: &str) -> bool {
    Command::new("wpctl")
        .arg("status")
        .output()
        .ok()
        .is_some_and(|out| parse_is_sink_default(&String::from_utf8_lossy(&out.stdout), pattern))
}
fn parse_is_sink_default(text: &str, pattern: &str) -> bool {
    let mut in_sinks = false;
    for line in text.lines() {
        if line.contains("Sinks:") {
            in_sinks = true;
            continue;
        }
        if in_sinks && (line.contains("Sources:") || line.contains("Filters:")) {
            break;
        }
        if in_sinks && line.contains(pattern) && line.contains('*') {
            return true;
        }
    }
    false
}

impl Render for DolbyGui {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_shell(cx)
    }
}

struct AppState {
    gui: Entity<DolbyGui>,
    _socket_guard: Option<instance::SocketGuard>,
}

impl gpui_kit::Global for AppState {}

fn open_or_activate_window(cx: &mut gpui_kit::App) {
    if let Some(window) = cx.windows().into_iter().next() {
        let _ = window.update(cx, |_, window, _| {
            window.activate_window();
        });
        return;
    }

    let bounds = Bounds::centered(None, size(px(1040.0), px(760.0)), cx);
    gpui_kit::open_window(
        WindowOptions {
            window_min_size: Some(size(px(780.0), px(600.0))),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        },
        cx,
        |window, cx| {
            window.set_window_title("OpenDolby");
            window.set_app_id("dolby-gui");
            let view = cx.global::<AppState>().gui.clone();
            cx.new(|cx| Root::new(view, window, cx))
        },
    )
    .expect("failed to open OpenDolby window");
}

fn handle_ipc_request(req: instance::IpcRequest, cx: &mut gpui_kit::App) {
    let gui = cx.global::<AppState>().gui.clone();
    let response = match req.command {
        instance::IpcCommand::Activate => {
            open_or_activate_window(cx);
            let window_open = !cx.windows().is_empty();
            gui.read(cx).status_json(window_open)
        }
        instance::IpcCommand::Start => {
            gui.update(cx, |this, cx| {
                this.toggle_engine(true);
                cx.notify();
            });
            let window_open = !cx.windows().is_empty();
            gui.read(cx).status_json(window_open)
        }
        instance::IpcCommand::Stop => {
            gui.update(cx, |this, cx| {
                this.toggle_engine(false);
                cx.notify();
            });
            let window_open = !cx.windows().is_empty();
            gui.read(cx).status_json(window_open)
        }
        instance::IpcCommand::Status => {
            let window_open = !cx.windows().is_empty();
            gui.read(cx).status_json(window_open)
        }
        instance::IpcCommand::Quit => {
            gui.update(cx, |this, cx| {
                this.toggle_engine(false);
                cx.notify();
            });
            let resp = format!(
                r#"{{"ok":true,"pid":{},"running":false,"quit":true}}"#,
                std::process::id()
            );
            let _ = req.reply.send(resp);
            cx.quit();
            return;
        }
    };
    let _ = req.reply.send(response);
}

fn main() {
    let args = GuiArgs::parse();
    let sock_path = instance::default_socket_path();

    if args.status {
        match instance::send_command(&sock_path, instance::IpcCommand::Status) {
            Ok(resp) => println!("{resp}"),
            Err(_) => println!(r#"{{"pid":null,"running":false,"profile":null,"window_open":false}}"#),
        }
        return;
    }

    if args.stop {
        match instance::send_command(&sock_path, instance::IpcCommand::Stop) {
            Ok(resp) => println!("{resp}"),
            Err(_) => println!(r#"{{"ok":true,"running":false}}"#),
        }
        return;
    }

    if args.quit {
        match instance::send_command(&sock_path, instance::IpcCommand::Quit) {
            Ok(resp) => println!("{resp}"),
            Err(_) => println!(r#"{{"ok":true,"running":false,"quit":true}}"#),
        }
        return;
    }

    let xml_explicit = args.xml.is_some();
    let xml = args.xml.unwrap_or_else(default_xml_path);

    let (ipc_rx, socket_guard) = match instance::bind_or_connect(&sock_path) {
        Ok(instance::BindOutcome::Bound(listener)) => instance::spawn_listener(listener, sock_path),
        Ok(instance::BindOutcome::AlreadyRunning) => {
            if xml_explicit {
                eprintln!(
                    "dolby-gui is already running; ignoring --xml {}",
                    xml.display()
                );
            }
            let cmd = if args.start {
                instance::IpcCommand::Start
            } else {
                instance::IpcCommand::Activate
            };
            match instance::send_command(&sock_path, cmd) {
                Ok(resp) => {
                    if args.start {
                        println!("{resp}");
                    }
                }
                Err(err) => {
                    eprintln!("Failed to communicate with running dolby-gui instance: {err}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Err(err) => {
            eprintln!("Failed to initialize dolby-gui IPC socket: {err}");
            std::process::exit(1);
        }
    };

    let auto_start = args.start;
    gpui_kit::application()
        .with_assets(assets::application_assets())
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_app_identity("dolby-gui", "OpenDolby");
            Theme::change(ThemeMode::Dark, None, cx);
            let theme = Theme::global_mut(cx);
            theme.colors.background = rgb(0x0c111a).into();
            theme.colors.foreground = rgb(0xecf1f7).into();
            theme.colors.primary = rgb(0x63d9ef).into();
            theme.colors.primary_foreground = rgb(0x07212a).into();
            theme.colors.primary_hover = rgb(0x89e6f5).into();
            theme.colors.primary_active = rgb(0x45bfd8).into();
            theme.colors.border = rgb(0x25303f).into();
            theme.colors.secondary = rgb(0x161f2b).into();

            let gui = cx.new(|cx| DolbyGui::new(&xml, cx));
            if auto_start {
                gui.update(cx, |this, cx| {
                    this.toggle_engine(true);
                    cx.notify();
                });
            }
            cx.set_global(AppState {
                gui,
                _socket_guard: Some(socket_guard),
            });

            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty()
                    && !cx.global::<AppState>().gui.read(cx).is_engine_running()
                {
                    cx.quit();
                }
            })
            .detach();

            cx.spawn(async move |cx| {
                while let Ok(req) = ipc_rx.recv().await {
                    cx.update(|cx| handle_ipc_request(req, cx));
                }
            })
            .detach();

            open_or_activate_window(cx);
        });
}

#[cfg(test)]
mod tests {
    use super::{parse_is_sink_default, parse_wpctl_sink_id};
    const STATUS: &str = "Audio\n ├─ Sinks:\n │ * 55. Dolby Atmos Rust [vol: 1.00]\n │ 56. Raptor Lake-P/U/H cAVS Speaker [vol: 0.75]\n ├─ Sources:\n │ * 57. Microphone\n";
    #[test]
    fn parses_sink_id_and_default_status() {
        assert_eq!(
            parse_wpctl_sink_id(STATUS, "Dolby Atmos Rust"),
            Some("55".into())
        );
        assert_eq!(
            parse_wpctl_sink_id(STATUS, "Raptor Lake-P/U/H cAVS Speaker"),
            Some("56".into())
        );
        assert!(parse_is_sink_default(STATUS, "Dolby Atmos Rust"));
        assert!(!parse_is_sink_default(
            STATUS,
            "Raptor Lake-P/U/H cAVS Speaker"
        ));
    }
}
