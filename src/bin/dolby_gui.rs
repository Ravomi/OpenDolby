use dolby_rs::dsp::DolbyProcessor;
use dolby_rs::pw_sink::{spawn_pipewire_sink, EngineHandle};
use dolby_rs::tuning::DeviceTuning;
use dolby_rs::DEFAULT_XML;
use gpui_kit::base::StyledExt;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex,
    switch::Switch,
    v_flex, ActiveTheme, Root, Sizable, Theme, ThemeMode,
};
use gpui_kit::{
    div, prelude::*, px, size, AppContext, Bounds, Context, SharedString, Window, WindowBounds,
    WindowOptions,
};
use std::process::Command;

const PROFILE_ORDER: &[(&str, &str)] = &[
    ("dynamic", "Dynamic"),
    ("music", "Music"),
    ("movie", "Movie"),
    ("game", "Game"),
    ("voice", "Voice"),
    ("voice_onlinecourse", "Online Course"),
    ("personalize_user1", "Custom 1"),
    ("personalize_user2", "Custom 2"),
    ("personalize_user3", "Custom 3"),
    ("off", "Off (Flat)"),
];

struct DolbyGui {
    tuning: Option<DeviceTuning>,
    active_profile: String,
    engine: Option<EngineHandle>,
    bypass: bool,
    status: SharedString,
}

impl DolbyGui {
    fn new() -> Self {
        let (tuning, status) = match DeviceTuning::from_xml_file(DEFAULT_XML) {
            Ok(t) => (Some(t), SharedString::from("Ready · Engine stopped")),
            Err(e) => (None, SharedString::from(format!("XML load error: {e}"))),
        };
        Self {
            tuning,
            active_profile: "dynamic".to_string(),
            engine: None,
            bypass: false,
            status,
        }
    }

    fn toggle_engine(&mut self, enable: bool) {
        if enable {
            if self.engine.is_some() {
                return;
            }
            let Some(ref tuning) = self.tuning else {
                self.status = "Cannot start: XML tuning not loaded".into();
                return;
            };
            let Some(proc) = DolbyProcessor::from_tuning(tuning, &self.active_profile, 48000) else {
                self.status = format!("Profile '{}' not found", self.active_profile).into();
                return;
            };
            match spawn_pipewire_sink(proc, None, 48000) {
                Ok(handle) => {
                    handle.set_bypass(self.bypass);
                    self.engine = Some(handle);
                    self.status = format!("Running · Profile: {}", self.active_profile).into();
                }
                Err(e) => {
                    self.status = format!("PipeWire start failed: {e}").into();
                }
            }
        } else {
            if let Some(mut handle) = self.engine.take() {
                handle.stop();
            }
            self.status = "Stopped · PipeWire node removed".into();
        }
    }

    fn select_profile(&mut self, profile: &str) {
        self.active_profile = profile.to_string();
        if let (Some(ref tuning), Some(ref handle)) = (&self.tuning, &self.engine) {
            if let Some(proc) = DolbyProcessor::from_tuning(tuning, profile, 48000) {
                handle.set_processor(proc);
                self.status = format!("Switched to '{}' (live)", profile).into();
            }
        } else {
            self.status = format!("Selected '{}' (start engine to apply)", profile).into();
        }
    }

    fn toggle_bypass(&mut self, bypass: bool) {
        self.bypass = bypass;
        if let Some(ref handle) = self.engine {
            handle.set_bypass(bypass);
            self.status = if bypass {
                "DSP bypassed (pass-through)".into()
            } else {
                format!("DSP active · Profile: {}", self.active_profile).into()
            };
        }
    }

    fn set_default_sink(&mut self, pattern: &str, label: &str) {
        match find_wpctl_sink_id(pattern) {
            Some(id) => {
                let res = Command::new("wpctl").args(["set-default", &id]).status();
                match res {
                    Ok(s) if s.success() => {
                        self.status = format!("Default output -> {label} (id {id})").into();
                    }
                    Ok(s) => {
                        self.status = format!("wpctl exited with {s}").into();
                    }
                    Err(e) => {
                        self.status = format!("Failed to run wpctl: {e}").into();
                    }
                }
            }
            None => {
                self.status = format!("Sink '{label}' not found in wpctl status").into();
            }
        }
    }
}

fn find_wpctl_sink_id(pattern: &str) -> Option<String> {
    let out = Command::new("wpctl").arg("status").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut in_sinks = false;
    for line in text.lines() {
        if line.contains("Sinks:") {
            in_sinks = true;
            continue;
        }
        if in_sinks && (line.contains("Sources:") || line.contains("Filters:") || line.trim().is_empty()) {
            if line.contains("Sources:") || line.contains("Filters:") {
                break;
            }
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

impl Render for DolbyGui {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let bg = theme.background;
        let card = theme.secondary;
        let border = theme.border;
        let muted = theme.muted_foreground;
        let fg = theme.foreground;

        let running = self.engine.is_some();

        let profile_details = self.tuning.as_ref().and_then(|t| t.profiles.get(&self.active_profile));

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .p_5()
            .gap_4()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_lg().font_bold().child("Dolby Atmos Linux"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child("Realtek ALC294 · 0x10431693 · 48 kHz"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .items_center()
                            .child(
                                Switch::new("bypass-switch")
                                    .checked(!self.bypass)
                                    .label(if self.bypass { "Bypass" } else { "DSP On" })
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.toggle_bypass(!*checked);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Switch::new("engine-switch")
                                    .checked(running)
                                    .label(if running { "Running" } else { "Stopped" })
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
                            .child("PROFILES"),
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
                            .child("ACTIVE DSP PIPELINE"),
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
                                        .child(format!("{}: {}", name, if on { "ON" } else { "OFF" }))
                                }))
                                .into_any_element()
                        }
                        None => div()
                            .text_xs()
                            .text_color(muted)
                            .child("No profile data loaded")
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
                            .child("PIPEWIRE ROUTING"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("route-dolby")
                                    .label("Use Dolby Atmos Rust")
                                    .small()
                                    .primary()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_default_sink(
                                            "Dolby Atmos Rust",
                                            "Dolby Atmos Rust (ALC294)",
                                        );
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("route-speaker")
                                    .label("Restore Speaker")
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_default_sink(
                                            "Raptor Lake-P/U/H cAVS Speaker",
                                            "Speaker",
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

        let bounds = Bounds::centered(None, size(px(560.0), px(520.0)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|_| DolbyGui::new());
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("failed to open Dolby GUI window");
    });
}
