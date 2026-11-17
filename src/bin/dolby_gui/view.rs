//! Application-owned native presentation. No webview or DSP implementation here.
use super::{assets, palette::*, DolbyGui, Page};
use gpui_kit::base::StyledExt;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex,
    slider::Slider,
    switch::Switch,
    v_flex, Disableable, Sizable,
};
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, Div, SharedString};

pub(super) struct Profile {
    pub id: &'static str,
    pub title: &'static str,
    description: &'static str,
}

#[cfg(test)]
mod tests {
    use super::{profile, PROFILES};

    #[test]
    fn profiles_have_unique_real_engine_ids_and_clear_fallback() {
        let mut ids = std::collections::HashSet::new();
        for profile in PROFILES {
            assert!(ids.insert(profile.id));
            assert!(!profile.title.is_empty());
            assert!(!profile.description.is_empty());
        }
        assert_eq!(PROFILES.len(), 9);
        assert!(!PROFILES.iter().any(|p| p.id == "voice_onlinecourse"));
        assert_eq!(profile("music").title, "音乐");
        assert_eq!(profile("missing").id, "dynamic");
    }
}
const PROFILES: &[Profile] = &[
    Profile {
        id: "dynamic",
        title: "动态",
        description: "适合日常聆听的本机调音。",
    },
    Profile {
        id: "game",
        title: "游戏",
        description: "用于游戏内容的声场与对白配置。",
    },
    Profile {
        id: "movie",
        title: "电影",
        description: "用于电影对白与声效的本机调音。",
    },
    Profile {
        id: "music",
        title: "音乐",
        description: "用于音乐聆听的本机调音。",
    },
    Profile {
        id: "voice",
        title: "人声",
        description: "用于播客、会议和语音内容。",
    },
    Profile {
        id: "personalize_user1",
        title: "自定义 1",
        description: "从 OEM 基准配置出发，调整聆听偏好。",
    },
    Profile {
        id: "personalize_user2",
        title: "自定义 2",
        description: "独立的 OEM 基准配置，不改写原始 XML。",
    },
    Profile {
        id: "personalize_user3",
        title: "自定义 3",
        description: "独立的 OEM 基准配置，不改写原始 XML。",
    },
    Profile {
        id: "off",
        title: "关闭音效",
        description: "完全直通，不含扬声器校准与保护。请注意音量。",
    },
];
pub(super) fn profile(id: &str) -> &'static Profile {
    PROFILES.iter().find(|p| p.id == id).unwrap_or(&PROFILES[0])
}
fn note(text: impl Into<SharedString>) -> Div {
    div().text_xs().text_color(rgb(MUTED)).child(text.into())
}
fn rule() -> Div {
    div().w_full().h(px(1.0)).flex_shrink_0().bg(rgb(LINE))
}

impl DolbyGui {
    pub(super) fn render_shell(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.engine.is_some();
        v_flex()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_sm()
            .child(
                h_flex()
                    .h(px(68.0))
                    .flex_shrink_0()
                    .px_6()
                    .gap_6()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        h_flex()
                            .w(px(152.0))
                            .gap_2()
                            .flex_shrink_0()
                            .items_center()
                            .child(
                                assets::app_mark()
                                    .w(px(30.0))
                                    .h(px(24.0))
                                    .flex_shrink_0()
                                    .text_color(rgb(brand_mark_color(BG))),
                            )
                            .child(div().text_lg().font_semibold().child("OpenDolby")),
                    )
                    .child(
                        h_flex().flex_1().h_full().gap_6().items_stretch().children(
                            [
                                (Page::Sound, "sound", "音效"),
                                (Page::Output, "output", "输出设备"),
                                (Page::About, "about", "关于"),
                            ]
                            .into_iter()
                            .map(|(page, id, label)| {
                                let selected = self.page == page;
                                div()
                                    .id(SharedString::from(format!("nav-{id}")))
                                    .h_full()
                                    .pt(px(2.0))
                                    .border_b_2()
                                    .border_color(rgb(if selected { TEXT } else { BG }))
                                    .flex()
                                    .items_center()
                                    .text_color(rgb(if selected { TEXT } else { MUTED }))
                                    .when(selected, |s| s.font_semibold())
                                    .cursor_pointer()
                                    .hover(|s| s.text_color(rgb(TEXT)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.page = page;
                                        if page == Page::Output {
                                            this.refresh_output_devices();
                                        }
                                        cx.notify();
                                    }))
                                    .child(label)
                            }),
                        ),
                    )
                    .child(
                        Switch::new("engine-switch")
                            .checked(running)
                            .disabled(self.tuning.is_none())
                            .small()
                            .label(if running {
                                "音效已启用"
                            } else {
                                "启用音效"
                            })
                            .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                                this.toggle_engine(*enabled);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.render_sidebar(cx))
                    .child(
                        div()
                            .id("settings-scroll")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .child(match self.page {
                                Page::Sound => self.render_sound(cx),
                                Page::Output => self.render_output(cx),
                                Page::About => self.render_about(),
                            }),
                    ),
            )
            .child(
                h_flex()
                    .h(px(32.0))
                    .flex_shrink_0()
                    .px_6()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .items_center()
                    .child(
                        div()
                            .size(px(5.0))
                            .rounded_full()
                            .flex_shrink_0()
                            .bg(rgb(if running { CYAN } else { MUTED })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(self.status.clone()),
                    )
                    .child(note("48 kHz / 2 ch")),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("profiles-sidebar")
            .w(px(176.0))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(NAV))
            .border_r_1()
            .border_color(rgb(LINE))
            .pt_6()
            .pb_5()
            .overflow_y_scroll()
            .child(div().px_6().pb_4().child(note("聆听模式")))
            .children(PROFILES.iter().enumerate().map(|(index, p)| {
                let selected = p.id == self.active_profile;
                v_flex()
                    .flex_shrink_0()
                    .when(index == 5, |s| {
                        s.child(div().px_6().pt_7().pb_3().child(note("个人配置")))
                    })
                    .when(index == 8, |s| {
                        s.child(div().mx_6().my_5().h(px(1.0)).bg(rgb(LINE)))
                    })
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("profile-{}", p.id)))
                            .h(px(40.0))
                            .px_6()
                            .gap_3()
                            .items_center()
                            .border_l_2()
                            .border_color(rgb(if selected { ACCENT } else { NAV }))
                            .bg(rgb(if selected { SOFT_BLUE } else { NAV }))
                            .text_color(rgb(if selected { TEXT } else { MUTED }))
                            .when(selected, |s| s.font_semibold())
                            .cursor_pointer()
                            .hover(|s| s.text_color(rgb(TEXT)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_profile(p.id);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .w(px(15.0))
                                    .text_xs()
                                    .font_normal()
                                    .text_color(rgb(MUTED))
                                    .child(if index == 8 {
                                        "—".into()
                                    } else {
                                        format!("{:02}", index + 1)
                                    }),
                            )
                            .child(p.title),
                    )
            }))
            .child(
                v_flex()
                    .mt_auto()
                    .pt_8()
                    .px_6()
                    .gap_1()
                    .child(note("本机 OEM 调音"))
                    .child(note("实验性 DSP").text_color(rgb(VIOLET))),
            )
    }

    fn render_sound(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = profile(&self.active_profile);
        let tuning = self
            .tuning
            .as_ref()
            .and_then(|t| t.profiles.get(&self.active_profile));
        let off = self.active_profile == "off";
        let surround = tuning
            .is_some_and(|p| p.cp.surround_virtualizer_enable || p.cp.surround_decoder_enable)
            && !off;
        let dialog = tuning.is_some_and(|p| p.cp.dialog_enhancer_enable) && !off;
        v_flex()
            .px_8()
            .pt_8()
            .pb_6()
            .gap_5()
            .w_full()
            .max_w(px(1000.0))
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .items_end()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(38.0))
                                    .line_height(px(48.0))
                                    .font_semibold()
                                    .child(selected.title),
                            )
                            .child(note(if off { "直通" } else { "当前预设" }).pb_2()),
                    )
                    .child(
                        div()
                            .text_color(rgb(MUTED))
                            .line_height(px(22.0))
                            .child(selected.description),
                    ),
            )
            .child(rule())
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().font_medium().child("声音调整"))
                    .child(
                        Button::new("reset-controls")
                            .ghost()
                            .small()
                            .label("恢复基准")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.reset_controls(window, cx)),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .gap_6()
                    .child(
                        v_flex()
                            .gap_3()
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(div().font_medium().child("环绕声场"))
                                            .child(note(if surround {
                                                "立体声展宽与早期反射"
                                            } else {
                                                "当前预设未启用"
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_lg()
                                            .text_color(rgb(if surround { CYAN } else { MUTED }))
                                            .child(format!("{:.2} ×", self.surround_amount)),
                                    ),
                            )
                            .child(
                                Slider::new(&self.surround_slider)
                                    .disabled(!surround)
                                    .accessibility_label("环绕声场强度")
                                    .bg(rgb(CYAN)),
                            )
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(note("0"))
                                    .child(note("2.0")),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_3()
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(div().font_medium().child("人声清晰度"))
                                            .child(note(if dialog {
                                                "对白增强幅度"
                                            } else {
                                                "当前预设未启用"
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_lg()
                                            .text_color(rgb(if dialog { VIOLET } else { MUTED }))
                                            .child(format!("{:.2} ×", self.dialog_amount)),
                                    ),
                            )
                            .child(
                                Slider::new(&self.dialog_slider)
                                    .disabled(!dialog)
                                    .accessibility_label("人声清晰度强度")
                                    .bg(rgb(VIOLET)),
                            )
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(note("0"))
                                    .child(note("2.0")),
                            ),
                    ),
            )
            .child(rule())
            .child(
                v_flex()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(div().mr_3().font_medium().child("试听对比"))
                                    .child(
                                        Button::new("audition-processed")
                                            .small()
                                            .when(!self.bypass, |b| b.primary())
                                            .when(self.bypass, |b| b.ghost())
                                            .label("音效")
                                            .disabled(off)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.toggle_bypass(false);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("audition-bypass")
                                            .small()
                                            .when(self.bypass, |b| b.primary())
                                            .when(!self.bypass, |b| b.ghost())
                                            .label("旁路")
                                            .disabled(off)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.toggle_bypass(true);
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                Switch::new("rms-match")
                                    .small()
                                    .checked(self.level_match)
                                    .disabled(off)
                                    .label("电平匹配")
                                    .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                                        this.toggle_level_match(*enabled);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(note(if off {
                        "完全直通模式不包含校准、安全限幅或电平匹配。"
                    } else {
                        "20 ms 淡变切换。RMS 电平匹配约需 1–2 秒稳定，不代表感知等响度。"
                    })),
            )
            .child(rule())
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(note("处理链"))
                            .child(
                                Button::new("pipeline-details")
                                    .ghost()
                                    .small()
                                    .label(if self.show_details {
                                        "收起 −"
                                    } else {
                                        "展开 +"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_details = !this.show_details;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(self.show_details, |s| s.child(self.render_pipeline()))
                    .child(note(
                        "调节不改写原始 XML。空间效果为立体声近似，不是 Atmos 解码。",
                    )),
            )
            .into_any_element()
    }

    fn render_pipeline(&self) -> AnyElement {
        let profile = self
            .tuning
            .as_ref()
            .and_then(|t| t.profiles.get(&self.active_profile));
        let off = self.active_profile == "off";
        let stages = profile
            .map(|p| {
                vec![
                    ("响度均衡", p.cp.volume_leveler_enable),
                    ("人声增强", p.cp.dialog_enhancer_enable),
                    ("静态 IEQ", p.cp.ieq_enable),
                    (
                        "声场展宽",
                        p.cp.surround_virtualizer_enable || p.cp.surround_decoder_enable,
                    ),
                    ("Audio Optimizer", p.vlldp.audio_optimizer_enable),
                    ("Speaker PEQ", p.vlldp.speaker_peq_enable),
                    ("多段压缩", p.vlldp.mb_compressor_enable),
                ]
            })
            .unwrap_or_default();
        div()
            .flex()
            .flex_wrap()
            .gap_4()
            .children(stages.into_iter().map(|(label, enabled)| {
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(4.0))
                            .rounded_full()
                            .bg(rgb(if enabled && !off { CYAN } else { LINE })),
                    )
                    .child(note(label))
            }))
            .into_any_element()
    }
}
