use dolby_rs::dsp::DolbyProcessor;
use dolby_rs::pw_sink::{spawn_pipewire_sink, EngineHandle};
use dolby_rs::tuning::DeviceTuning;
use dolby_rs::DEFAULT_XML;
use gpui_kit::base::StyledExt;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex,
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    v_flex, ActiveTheme, Root, Sizable, Theme, ThemeMode,
};
use gpui_kit::{
    div, prelude::*, px, size, Bounds, Context, Entity, SharedString, Subscription, Window,
    WindowBounds, WindowOptions,
};
use std::process::Command;

const OUR_SINK_PATTERN: &str = "Dolby Atmos Rust";
const SPEAKER_SINK_PATTERN: &str = "Raptor Lake-P/U/H cAVS Speaker";

const PROFILE_ORDER: &[(&str, &str)] = &[
    ("dynamic", "Dynamic 动态"),
    ("music", "Music 音乐"),
    ("movie", "Movie 电影"),
    ("game", "Game 游戏"),
    ("voice", "Voice 人声"),
    ("voice_onlinecourse", "Course 网课"),
    ("personalize_user1", "Custom 1"),
    ("personalize_user2", "Custom 2"),
    ("personalize_user3", "Custom 3"),
    ("off", "Off 关闭"),
];

struct DolbyGui {
    tuning: Option<DeviceTuning>,
    active_profile: String,
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
    fn new(cx: &mut Context<Self>) -> Self {
        let (tuning, status) = match DeviceTuning::from_xml_file(DEFAULT_XML) {
            Ok(t) => (Some(t), SharedString::from("就绪 · 引擎未启动")),
            Err(e) => (None, SharedString::from(format!("XML 加载失败: {e}"))),
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

        let sub_surround = cx.subscribe(
            &surround_slider,
            |this, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(val) = event;
                this.set_surround_amount(val.start());
                cx.notify();
            },
        );
        let sub_dialog = cx.subscribe(
            &dialog_slider,
            |this, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(val) = event;
                this.set_dialog_amount(val.start());
                cx.notify();
            },
        );

        Self {
            tuning,
            active_profile: "dynamic".to_string(),
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
                self.status = "无法启动: XML 调音未加载".into();
                return;
            };
            let Some(proc) = DolbyProcessor::from_tuning(tuning, &self.active_profile, 48000)
            else {
                self.status = format!("未找到配置文件 '{}'", self.active_profile).into();
                return;
            };
            match spawn_pipewire_sink(proc, None, 48000) {
                Ok(handle) => {
                    self.apply_live_controls(&handle);
                    self.engine = Some(handle);
                    self.status = format!("运行中 · 配置: {}", self.active_profile).into();
                }
                Err(e) => {
                    self.status = format!("PipeWire 启动失败: {e}").into();
                }
            }
        } else {
            self.restore_speaker_if_our_sink_default();
            if let Some(mut handle) = self.engine.take() {
                handle.stop();
            }
            self.status = "已停止 · PipeWire 节点已移除".into();
        }
    }

    fn select_profile(&mut self, profile: &str) {
        self.active_profile = profile.to_string();
        if let (Some(ref tuning), Some(ref handle)) = (&self.tuning, &self.engine) {
            if let Some(proc) = DolbyProcessor::from_tuning(tuning, profile, 48000) {
                handle.set_processor(proc);
                self.apply_live_controls(handle);
                self.status = format!("已切换配置 '{}' (实时生效)", profile).into();
            }
        } else {
            self.status = format!("已选择 '{}' (启动引擎后生效)", profile).into();
        }
    }

    fn toggle_bypass(&mut self, bypass: bool) {
        self.bypass = bypass;
        if let Some(ref handle) = self.engine {
            handle.set_bypass(bypass);
            self.status = if bypass {
                "直通模式 (Bypass): 未经 DSP 处理".into()
            } else {
                format!("处理中 (Processed) · 配置: {}", self.active_profile).into()
            };
        }
    }

    fn toggle_level_match(&mut self, level_match: bool) {
        self.level_match = level_match;
        if let Some(ref handle) = self.engine {
            handle.set_level_match(level_match);
        }
        self.status = if level_match {
            "等响度 A/B 已开启 (近似 RMS 匹配)".into()
        } else {
            "等响度 A/B 已关闭".into()
        };
    }

    fn set_surround_amount(&mut self, amount: f32) {
        let clamped = amount.clamp(0.0, 2.0);
        self.surround_amount = clamped;
        if let Some(ref handle) = self.engine {
            handle.set_surround_amount(clamped);
        }
    }

    fn set_dialog_amount(&mut self, amount: f32) {
        let clamped = amount.clamp(0.0, 2.0);
        self.dialog_amount = clamped;
        if let Some(ref handle) = self.engine {
            handle.set_dialog_amount(clamped);
        }
    }

    fn set_default_sink(&mut self, pattern: &str, label: &str, is_our_sink: bool) {
        match find_wpctl_sink_id(pattern) {
            Some(id) => {
                let res = Command::new("wpctl").args(["set-default", &id]).status();
                match res {
                    Ok(s) if s.success() => {
                        self.we_changed_default = is_our_sink;
                        self.status = format!("默认输出 -> {label} (id {id})").into();
                    }
                    Ok(s) => {
                        self.status = format!("wpctl 退出码: {s}").into();
                    }
                    Err(e) => {
                        self.status = format!("运行 wpctl 失败: {e}").into();
                    }
                }
            }
            None => {
                self.status = format!("wpctl status 中未找到 '{label}'").into();
            }
        }
    }

    fn restore_speaker_if_our_sink_default(&mut self) {
        if is_sink_default(OUR_SINK_PATTERN) {
            if let Some(id) = find_wpctl_sink_id(SPEAKER_SINK_PATTERN) {
                if let Ok(s) = Command::new("wpctl").args(["set-default", &id]).status() {
                    if s.success() {
                        self.we_changed_default = false;
                    }
                }
            }
        }
    }
}

impl Drop for DolbyGui {
    fn drop(&mut self) {
        if self.we_changed_default && is_sink_default(OUR_SINK_PATTERN) {
            if let Some(id) = find_wpctl_sink_id(SPEAKER_SINK_PATTERN) {
                let _ = Command::new("wpctl").args(["set-default", &id]).status();
            }
            self.we_changed_default = false;
        }
        if let Some(mut handle) = self.engine.take() {
            handle.stop();
        }
    }
}

fn find_wpctl_sink_id(pattern: &str) -> Option<String> {
    let out = Command::new("wpctl").arg("status").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_wpctl_sink_id(&text, pattern)
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
                    return Some(num.to_string());
                }
            }
        }
    }
    None
}

fn is_sink_default(pattern: &str) -> bool {
    let Ok(out) = Command::new("wpctl").arg("status").output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    parse_is_sink_default(&text, pattern)
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
        let theme = cx.theme();
        let bg = theme.background;
        let card = theme.secondary;
        let border = theme.border;
        let muted = theme.muted_foreground;
        let fg = theme.foreground;

        let running = self.engine.is_some();

        let profile_details = self
            .tuning
            .as_ref()
            .and_then(|t| t.profiles.get(&self.active_profile));

        let surround_enabled_in_profile = profile_details
            .map(|p| p.cp.surround_virtualizer_enable)
            .unwrap_or(false);
        let dialog_enabled_in_profile = profile_details
            .map(|p| p.cp.dialog_enhancer_enable)
            .unwrap_or(false);

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .p_5()
            .gap_4()
            .overflow_y_scroll()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_lg().font_bold().child("OpenDolby"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child("Experimental OEM tuning / spatial approximation · Realtek ALC294 · 48 kHz"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .items_center()
                            .child(
                                Switch::new("bypass-switch")
                                    .checked(!self.bypass)
                                    .label(if self.bypass {
                                        "直通 Bypass"
                                    } else {
                                        "处理中 Processed"
                                    })
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.toggle_bypass(!*checked);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Switch::new("level-match-switch")
                                    .checked(self.level_match)
                                    .label(if self.level_match {
                                        "等响度 A/B 开"
                                    } else {
                                        "等响度 A/B 关"
                                    })
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.toggle_level_match(*checked);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Switch::new("engine-switch")
                                    .checked(running)
                                    .label(if running { "运行中" } else { "已停止" })
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.toggle_engine(*checked);
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(card)
                    .border_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(muted)
                            .child("配置文件 (PROFILES)"),
                    )
                    .child(div().flex().flex_wrap().gap_2().children(
                        PROFILE_ORDER.iter().map(|&(id, label)| {
                            let is_active = self.active_profile == id;
                            let btn = Button::new(SharedString::from(format!("prof-{id}")))
                                .label(label)
                                .small();
                            let btn = if is_active { btn.primary() } else { btn.outline() };
                            btn.on_click(cx.listener(move |this, _, _, cx| {
                                this.select_profile(id);
                                cx.notify();
                            }))
                        }),
                    )),
            )
            .child(
                v_flex()
                    .gap_3()
                    .p_3()
                    .rounded_lg()
                    .bg(card)
                    .border_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(muted)
                            .child("实时调节 (LIVE CONTROLS · 1.00x = 配置文件基准, 0 = 关闭)"),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(div().text_sm().child("环绕强度 (Surround)"))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(if surround_enabled_in_profile {
                                                fg
                                            } else {
                                                muted
                                            })
                                            .child(if surround_enabled_in_profile {
                                                format!("{:.2}x", self.surround_amount)
                                            } else {
                                                format!(
                                                    "{:.2}x (当前配置已禁用)",
                                                    self.surround_amount
                                                )
                                            }),
                                    ),
                            )
                            .child(Slider::new(&self.surround_slider)),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(div().text_sm().child("人声增强 (Dialog)"))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(if dialog_enabled_in_profile {
                                                fg
                                            } else {
                                                muted
                                            })
                                            .child(if dialog_enabled_in_profile {
                                                format!("{:.2}x", self.dialog_amount)
                                            } else {
                                                format!(
                                                    "{:.2}x (当前配置已禁用)",
                                                    self.dialog_amount
                                                )
                                            }),
                                    ),
                            )
                            .child(Slider::new(&self.dialog_slider)),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(card)
                    .border_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(muted)
                            .child("DSP 管线状态 (ACTIVE DSP PIPELINE)"),
                    )
                    .child(match profile_details {
                        Some(p) => {
                            let stages = [
                                ("Volume Leveler", p.cp.volume_leveler_enable),
                                ("Dialog Enhancer", p.cp.dialog_enhancer_enable),
                                ("Intelligent EQ", p.cp.ieq_enable),
                                ("Surround Virtualizer", p.cp.surround_virtualizer_enable),
                                ("Audio Optimizer", p.vlldp.audio_optimizer_enable),
                                ("Speaker PEQ", p.vlldp.speaker_peq_enable),
                                ("4-Band MBC", p.vlldp.mb_compressor_enable),
                            ];
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .children(stages.into_iter().map(|(name, on)| {
                                    div()
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(border)
                                        .text_xs()
                                        .text_color(if on { fg } else { muted })
                                        .child(format!(
                                            "{}: {}",
                                            name,
                                            if on { "ON" } else { "OFF" }
                                        ))
                                }))
                                .into_any_element()
                        }
                        None => div()
                            .text_xs()
                            .text_color(muted)
                            .child("未加载配置文件数据")
                            .into_any_element(),
                    }),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(card)
                    .border_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(muted)
                            .child("PIPEWIRE 路由"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("route-dolby")
                                    .label("使用 OpenDolby 输出")
                                    .small()
                                    .primary()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_default_sink(
                                            OUR_SINK_PATTERN,
                                            "OpenDolby (ALC294)",
                                            true,
                                        );
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("route-speaker")
                                    .label("恢复扬声器 (Speaker)")
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_default_sink(
                                            SPEAKER_SINK_PATTERN,
                                            "Speaker",
                                            false,
                                        );
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .mt_auto()
                    .text_xs()
                    .text_color(muted)
                    .child(self.status.clone()),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);

        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(640.0), px(720.0)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| DolbyGui::new(cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("failed to open OpenDolby GUI window");
    });
}

#[cfg(test)]
mod tests {
    use super::{parse_is_sink_default, parse_wpctl_sink_id};

    const SAMPLE_WPCTL_STATUS: &str = "\
Audio
 ├─ Devices:
 │      42. Raptor Lake-P/U/H cAVS [alsa]
 │
 ├─ Sinks:
 │  *   55. Dolby Atmos Rust                    [vol: 1.00]
 │      56. Raptor Lake-P/U/H cAVS Speaker      [vol: 0.75]
 │
 ├─ Sources:
 │  *   57. Raptor Lake-P/U/H cAVS Digital Microphone [vol: 1.00]
";

    #[test]
    fn parses_sink_id_and_default_status() {
        assert_eq!(
            parse_wpctl_sink_id(SAMPLE_WPCTL_STATUS, "Dolby Atmos Rust"),
            Some("55".to_string())
        );
        assert_eq!(
            parse_wpctl_sink_id(SAMPLE_WPCTL_STATUS, "Raptor Lake-P/U/H cAVS Speaker"),
            Some("56".to_string())
        );
        assert!(parse_is_sink_default(SAMPLE_WPCTL_STATUS, "Dolby Atmos Rust"));
        assert!(!parse_is_sink_default(
            SAMPLE_WPCTL_STATUS,
            "Raptor Lake-P/U/H cAVS Speaker"
        ));
    }
}
