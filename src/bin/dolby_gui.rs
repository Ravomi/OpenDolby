#[path = "dolby_gui/assets.rs"]
mod assets;
#[path = "dolby_gui/instance.rs"]
mod instance;
#[path = "dolby_gui/palette.rs"]
mod palette;
#[path = "dolby_gui/view.rs"]
mod view;

use clap::Parser;
use dolby_rs::audio_devices::{self, choose_internal_speaker, DeviceInventory, OutputDevice};
use dolby_rs::default_xml_path;
use dolby_rs::dsp::DolbyProcessor;
use dolby_rs::pw_sink::{spawn_pipewire_sink, EngineHandle};
use dolby_rs::tuning::DeviceTuning;
use gpui_kit::component::{
    slider::{SliderEvent, SliderState},
    Root,
};
use gpui_kit::{
    prelude::*, px, size, Bounds, Context, Entity, SharedString, Subscription, Window,
    WindowBounds, WindowOptions,
};
use std::{path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(name = "dolby-gui", about = "OpenDolby native PipeWire control panel")]
struct GuiArgs {
    /// Local OEM tuning XML. Defaults to OPENDOLBY_XML or the user config directory.
    #[arg(long)]
    xml: Option<PathBuf>,
    /// Print the discovered audio outputs as JSON without opening a window.
    #[arg(long)]
    devices: bool,
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

const VIRTUAL_SINK: &str = "dolby_atmos_rust";

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
    outputs: Vec<OutputDevice>,
    target_sink: Option<String>,
    default_sink: Option<String>,
    restore_sink: Option<String>,
    device_error: Option<String>,
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
        let (outputs, default_sink, device_error) = match audio_devices::discover_outputs() {
            Ok(inventory) => (inventory.outputs, inventory.default_sink, None),
            Err(error) => (Vec::new(), None, Some(format!("{error:#}"))),
        };
        let target_sink = choose_internal_speaker(&outputs, default_sink.as_deref())
            .map(|device| device.name.clone());
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
            outputs,
            target_sink,
            default_sink,
            restore_sink: None,
            device_error,
            status,
        }
    }

    fn is_engine_running(&self) -> bool {
        self.engine.is_some()
    }

    fn status_json(&self, window_open: bool) -> String {
        serde_json::json!({
            "pid": std::process::id(),
            "running": self.engine.is_some(),
            "profile": self.active_profile,
            "window_open": window_open,
            "target_sink": self.target_sink,
            "bypass": self.bypass,
            "level_match": self.level_match,
            "surround_amount": self.surround_amount,
            "dialog_amount": self.dialog_amount,
        })
        .to_string()
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
            self.refresh_output_devices();
            if let Some(error) = &self.device_error {
                self.status = format!("无法读取输出设备: {error}").into();
                return;
            }
            let Some(target) = self.target_sink.clone().filter(|name| {
                self.outputs.iter().any(|device| {
                    device.name == *name
                        && device.is_internal_speaker
                        && device.available
                        && !device.is_virtual
                })
            }) else {
                self.status = "未找到可安全使用的内置扬声器，请在输出设备页检查。".into();
                return;
            };
            let Some(ref tuning) = self.tuning else {
                self.status = "无法启动: 未加载调音文件".into();
                return;
            };
            let Some(proc) = DolbyProcessor::from_tuning(tuning, &self.active_profile, 48000)
            else {
                self.status = format!("未找到配置 '{}'", self.active_profile).into();
                return;
            };
            let original_output = self.default_sink.clone().filter(|name| {
                self.outputs
                    .iter()
                    .any(|device| device.name == *name && !device.is_virtual && device.available)
            });
            match spawn_pipewire_sink(proc, Some(target.clone()), 48000) {
                Ok(handle) => {
                    self.apply_live_controls(&handle);
                    self.restore_sink = original_output.or(Some(target));
                    self.engine = Some(handle);
                    self.status = "引擎已启动，可在输出设备中选择 OpenDolby".into();
                }
                Err(e) => self.status = format!("PipeWire 启动失败: {e}").into(),
            }
        } else {
            let restore_error = self.restore_output_if_ours().err();
            if let Some(mut handle) = self.engine.take() {
                handle.stop();
            }
            self.refresh_output_devices();
            self.status = match restore_error {
                Some(error) => format!("引擎已停止，恢复输出时出错: {error:#}").into(),
                None => "引擎已停止，已移除虚拟输出".into(),
            };
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

    fn refresh_output_devices(&mut self) {
        self.apply_inventory(audio_devices::discover_outputs());
    }

    fn apply_inventory(&mut self, result: anyhow::Result<DeviceInventory>) {
        match result {
            Ok(inventory) => {
                self.outputs = inventory.outputs;
                self.default_sink = inventory.default_sink;
                self.device_error = None;
                // A disappearing target must never silently turn into a headset or HDMI.
                let safe = target_is_safe(&self.outputs, self.target_sink.as_deref());
                if self.engine.is_some() && !safe {
                    let _ = self.restore_output_if_ours();
                    if let Some(mut engine) = self.engine.take() {
                        engine.stop();
                    }
                    self.status =
                        "处理已暂停：扬声器已断开或切换了端口，请重新检查输出设备。".into();
                }
            }
            Err(error) => self.device_error = Some(format!("{error:#}")),
        }
    }

    fn select_output_target(&mut self, name: &str) {
        if self.engine.is_some() {
            self.status = "请先停止音效，再更换处理目标。".into();
            return;
        }
        self.refresh_output_devices();
        if !target_is_safe(&self.outputs, Some(name)) || self.device_error.is_some() {
            self.status = "此设备不是可用的内置扬声器，不应用本机扬声器调音。".into();
            return;
        }
        self.target_sink = Some(name.to_string());
        self.status = "处理目标已更新，请确认调音 XML 属于这台设备。".into();
    }

    fn set_output(&mut self, name: &str) {
        self.refresh_output_devices();
        if self.device_error.is_some() {
            self.status = "无法读取音频设备，暂不更改默认输出。".into();
            return;
        }
        let valid = if name == VIRTUAL_SINK {
            self.engine.is_some() && target_is_safe(&self.outputs, self.target_sink.as_deref())
        } else {
            self.outputs
                .iter()
                .any(|device| device.name == name && !device.is_virtual && device.available)
        };
        if !valid {
            self.status = "目标输出不可用，系统默认输出未更改。".into();
            return;
        }
        match audio_devices::set_default_output(name) {
            Ok(()) => {
                self.default_sink = Some(name.to_string());
                self.status = if name == VIRTUAL_SINK {
                    "系统音频已交给 OpenDolby；关闭面板不会停止处理。".into()
                } else {
                    "已切换为设备直连，不经过 OpenDolby。".into()
                };
            }
            Err(error) => self.status = format!("输出切换失败: {error:#}").into(),
        }
    }

    fn restore_output_if_ours(&mut self) -> anyhow::Result<()> {
        let inventory = audio_devices::discover_outputs()?;
        if !should_restore_default_output(inventory.default_sink.as_deref()) {
            return Ok(());
        }
        let name = restore_output_name(
            &inventory.outputs,
            self.restore_sink.as_deref(),
            self.target_sink.as_deref(),
        )
        .ok_or_else(|| anyhow::anyhow!("原输出和扬声器均不可用，请在系统设置中选择音频输出"))?;
        audio_devices::set_default_output(name)?;
        self.default_sink = Some(name.to_string());
        Ok(())
    }
}

impl Drop for DolbyGui {
    fn drop(&mut self) {
        if self.engine.is_some() {
            let _ = self.restore_output_if_ours();
        }
        if let Some(mut handle) = self.engine.take() {
            handle.stop();
        }
    }
}

fn should_restore_default_output(default_sink: Option<&str>) -> bool {
    default_sink == Some(VIRTUAL_SINK)
}

fn target_is_safe(outputs: &[OutputDevice], target: Option<&str>) -> bool {
    target.is_some_and(|name| {
        outputs.iter().any(|device| {
            device.name == name
                && device.is_internal_speaker
                && device.available
                && !device.is_virtual
        })
    })
}

fn restore_output_name<'a>(
    outputs: &'a [OutputDevice],
    original: Option<&str>,
    target: Option<&str>,
) -> Option<&'a str> {
    [original, target].into_iter().flatten().find_map(|name| {
        outputs
            .iter()
            .find(|device| device.name == name && !device.is_virtual && device.available)
            .map(|device| device.name.as_str())
    })
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
            instance::wait_for_quit_flush(&req.flushed);
            cx.quit();
            return;
        }
    };
    let _ = req.reply.send(response);
}

fn main() {
    let args = GuiArgs::parse();
    if args.devices {
        match audio_devices::discover_outputs() {
            Ok(inventory) => println!(
                "{}",
                serde_json::json!({
                    "default_sink": inventory.default_sink,
                    "outputs": inventory.outputs.iter().map(|device| serde_json::json!({
                        "name": device.name,
                        "description": device.description,
                        "available": device.available,
                        "is_virtual": device.is_virtual,
                        "is_internal_speaker": device.is_internal_speaker,
                    })).collect::<Vec<_>>(),
                })
            ),
            Err(error) => {
                eprintln!("Failed to discover audio outputs: {error:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    let sock_path = instance::default_socket_path();

    if args.status {
        match instance::send_command(&sock_path, instance::IpcCommand::Status) {
            Ok(resp) => println!("{resp}"),
            Err(err) if instance::is_not_running_error(&err) => {
                println!(r#"{{"pid":null,"running":false,"profile":null,"window_open":false}}"#);
            }
            Err(err) => {
                eprintln!("Failed to query dolby-gui status: {err}");
                std::process::exit(1);
            }
        }
        return;
    }

    if args.stop {
        match instance::send_command(&sock_path, instance::IpcCommand::Stop) {
            Ok(resp) => println!("{resp}"),
            Err(err) if instance::is_not_running_error(&err) => {
                println!(r#"{{"ok":true,"running":false}}"#);
            }
            Err(err) => {
                eprintln!("Failed to stop dolby-gui engine: {err}");
                std::process::exit(1);
            }
        }
        return;
    }

    if args.quit {
        match instance::send_command(&sock_path, instance::IpcCommand::Quit) {
            Ok(resp) => println!("{resp}"),
            Err(err) if instance::is_not_running_error(&err) => {
                println!(r#"{{"ok":true,"running":false,"quit":true}}"#);
            }
            Err(err) => {
                eprintln!("Failed to quit dolby-gui instance: {err}");
                std::process::exit(1);
            }
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
        .with_quit_mode(gpui_kit::QuitMode::Explicit)
        .with_assets(assets::application_assets())
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_app_identity("dolby-gui", "OpenDolby");
            palette::apply(cx);

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

            cx.spawn(async move |cx| loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let inventory = cx
                    .background_executor()
                    .spawn(async { audio_devices::discover_outputs() })
                    .await;
                cx.update(|cx| {
                    let gui = cx.global::<AppState>().gui.clone();
                    gui.update(cx, |this, cx| {
                        this.apply_inventory(inventory);
                        cx.notify();
                    });
                });
            })
            .detach();

            open_or_activate_window(cx);
        });
}

#[cfg(test)]
mod tests {
    use super::{restore_output_name, should_restore_default_output, target_is_safe, VIRTUAL_SINK};
    use dolby_rs::audio_devices::{DeviceKind, OutputDevice};

    fn device(
        id: u32,
        name: &str,
        is_internal_speaker: bool,
        available: bool,
        is_virtual: bool,
        kind: DeviceKind,
    ) -> OutputDevice {
        OutputDevice {
            id,
            name: name.to_string(),
            description: name.to_string(),
            is_virtual,
            is_internal_speaker,
            available,
            kind,
        }
    }

    #[test]
    fn target_is_safe_rejects_unavailable_headphones_and_virtual_outputs() {
        let outputs = vec![
            device(1, "speaker.ok", true, true, false, DeviceKind::InternalSpeaker),
            device(2, "speaker.unavail", true, false, false, DeviceKind::InternalSpeaker),
            device(3, "headphones.ok", false, true, false, DeviceKind::Headphones),
            device(4, VIRTUAL_SINK, true, true, true, DeviceKind::Virtual),
        ];

        assert!(target_is_safe(&outputs, Some("speaker.ok")));
        assert!(!target_is_safe(&outputs, Some("speaker.unavail")));
        assert!(!target_is_safe(&outputs, Some("headphones.ok")));
        assert!(!target_is_safe(&outputs, Some(VIRTUAL_SINK)));
        assert!(!target_is_safe(&outputs, Some("missing.sink")));
        assert!(!target_is_safe(&outputs, None));
    }

    #[test]
    fn restore_output_name_prefers_original_then_falls_back_to_target() {
        let outputs = vec![
            device(1, "headphones.ok", false, true, false, DeviceKind::Headphones),
            device(2, "speaker.ok", true, true, false, DeviceKind::InternalSpeaker),
            device(3, "headphones.unavail", false, false, false, DeviceKind::Headphones),
            device(4, VIRTUAL_SINK, false, true, true, DeviceKind::Virtual),
        ];

        assert_eq!(
            restore_output_name(&outputs, Some("headphones.ok"), Some("speaker.ok")),
            Some("headphones.ok")
        );
        assert_eq!(
            restore_output_name(&outputs, Some("headphones.unavail"), Some("speaker.ok")),
            Some("speaker.ok")
        );
        assert_eq!(
            restore_output_name(&outputs, Some(VIRTUAL_SINK), Some("speaker.ok")),
            Some("speaker.ok")
        );
        assert_eq!(
            restore_output_name(&outputs, Some("missing"), Some("headphones.unavail")),
            None
        );
    }

    #[test]
    fn should_restore_default_output_does_not_override_user_selected_sink() {
        assert!(should_restore_default_output(Some(VIRTUAL_SINK)));
        assert!(!should_restore_default_output(Some("headphones.ok")));
        assert!(!should_restore_default_output(Some("speaker.ok")));
        assert!(!should_restore_default_output(None));
    }
}
