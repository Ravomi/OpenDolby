//! Application-owned native presentation. No webview and no DSP implementation here.
use super::{assets, DolbyGui, Page, OUR_SINK_PATTERN, SPEAKER_SINK_PATTERN};
use gpui_kit::base::StyledExt;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex,
    slider::Slider,
    switch::Switch,
    v_flex, Disableable, Sizable,
};
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, Div, SharedString};

const BG: u32 = 0x0c111a;
const NAV: u32 = 0x090e16;
const PANEL: u32 = 0x121b27;
const LINE: u32 = 0x243140;
const TEXT: u32 = 0xe9f0f8;
const MUTED: u32 = 0x8d9bad;
const ACCENT: u32 = 0x63d9ef;

pub(super) struct Profile {
    pub id: &'static str,
    pub title: &'static str,
    english: &'static str,
    icon: &'static str,
    description: &'static str,
}
const PROFILES: &[Profile] = &[
    Profile {
        id: "dynamic",
        title: "动态",
        english: "DYNAMIC",
        icon: "sparkles",
        description: "从本机调音配置出发，让日常聆听拥有更完整的声音层次。",
    },
    Profile {
        id: "game",
        title: "游戏",
        english: "GAME",
        icon: "gamepad-2",
        description: "为游戏选择专属调音，按需调节空间展宽和人声清晰度。",
    },
    Profile {
        id: "movie",
        title: "电影",
        english: "MOVIE",
        icon: "clapperboard",
        description: "为对白与电影声效选择专属配置，找到舒适的观影听感。",
    },
    Profile {
        id: "music",
        title: "音乐",
        english: "MUSIC",
        icon: "music",
        description: "专注于旋律与细节，聆听这台扬声器的专属音乐调音。",
    },
    Profile {
        id: "voice",
        title: "人声",
        english: "VOICE",
        icon: "mic",
        description: "让语音内容成为主角，适合播客、会议与日常对白。",
    },
    Profile {
        id: "voice_onlinecourse",
        title: "网课",
        english: "COURSE",
        icon: "headphones",
        description: "面向长时间语音内容的配置，减少调节，让注意力留在内容上。",
    },
    Profile {
        id: "personalize_user1",
        title: "自定义 1",
        english: "CUSTOM 01",
        icon: "sliders-horizontal",
        description: "基于 OEM 自定义配置，微调环绕与人声的实时强度。",
    },
    Profile {
        id: "personalize_user2",
        title: "自定义 2",
        english: "CUSTOM 02",
        icon: "sliders-horizontal",
        description: "独立的自定义配置入口，实时调节不改写原始调音文件。",
    },
    Profile {
        id: "personalize_user3",
        title: "自定义 3",
        english: "CUSTOM 03",
        icon: "sliders-horizontal",
        description: "保留原始硬件调音，用实时控制微调自己的聆听偏好。",
    },
    Profile {
        id: "off",
        title: "关闭音效",
        english: "OFF",
        icon: "power",
        description: "完全平直直通，不保留扬声器校准或高通保护。请注意系统音量。",
    },
];
pub(super) fn profile(id: &str) -> &'static Profile {
    PROFILES.iter().find(|p| p.id == id).unwrap_or(&PROFILES[0])
}
fn caption(text: impl Into<SharedString>) -> Div {
    div().text_xs().text_color(rgb(MUTED)).child(text.into())
}
fn divider() -> Div {
    div().w_full().h(px(1.0)).bg(rgb(LINE)).flex_shrink_0()
}
fn badge(text: impl Into<SharedString>, active: bool) -> Div {
    div()
        .px_2()
        .py_1()
        .rounded_md()
        .text_xs()
        .text_color(rgb(if active { ACCENT } else { MUTED }))
        .bg(rgb(if active { 0x14303b } else { PANEL }))
        .child(text.into())
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
                    .h(px(72.0))
                    .flex_shrink_0()
                    .px_6()
                    .gap_5()
                    .items_center()
                    .bg(rgb(NAV))
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        h_flex()
                            .gap_3()
                            .flex_shrink_0()
                            .items_center()
                            .child(
                                assets::app_mark()
                                    .w(px(34.0))
                                    .h(px(24.0))
                                    .text_color(rgb(TEXT)),
                            )
                            .child(div().text_lg().font_semibold().child("OpenDolby")),
                    )
                    .child(
                        h_flex().flex_1().justify_center().gap_1().children(
                            [
                                (Page::Sound, "sound", "音效设置"),
                                (Page::Output, "output", "输出设备"),
                                (Page::About, "about", "关于"),
                            ]
                            .into_iter()
                            .map(|(page, id, label)| {
                                let selected = self.page == page;
                                div()
                                    .id(SharedString::from(format!("nav-{id}")))
                                    .px_3()
                                    .h(px(44.0))
                                    .flex()
                                    .items_center()
                                    .border_b_2()
                                    .border_color(rgb(if selected { ACCENT } else { NAV }))
                                    .text_color(rgb(if selected { TEXT } else { MUTED }))
                                    .cursor_pointer()
                                    .hover(|style| style.text_color(rgb(TEXT)).bg(rgb(PANEL)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.page = page;
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
                            .label(if running { "已启用" } else { "启用音效" })
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
                    .h(px(42.0))
                    .flex_shrink_0()
                    .px_5()
                    .gap_3()
                    .bg(rgb(NAV))
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .items_center()
                    .child(
                        div()
                            .size(px(6.0))
                            .rounded_full()
                            .flex_shrink_0()
                            .bg(rgb(if running { ACCENT } else { MUTED })),
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
                    .child(caption("ALC294 · 48 kHz · Stereo")),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("profiles-sidebar")
            .w(px(174.0))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(NAV))
            .border_r_1()
            .border_color(rgb(LINE))
            .pt_6()
            .pb_4()
            .gap_1()
            .overflow_y_scroll()
            .child(div().px_5().pb_3().child(caption("聆听模式")))
            .children(PROFILES.iter().enumerate().map(|(index, p)| {
                let selected = p.id == self.active_profile;
                div()
                    .id(SharedString::from(format!("profile-{}", p.id)))
                    .when(index == 6 || index == 9, |row| row.mt_3())
                    .h(px(43.0))
                    .flex_shrink_0()
                    .pl_4()
                    .pr_3()
                    .gap_3()
                    .flex()
                    .items_center()
                    .border_l_2()
                    .border_color(rgb(if selected { ACCENT } else { NAV }))
                    .bg(rgb(if selected { 0x142a36 } else { NAV }))
                    .text_color(rgb(if selected { ACCENT } else { MUTED }))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(PANEL)).text_color(rgb(TEXT)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_profile(p.id);
                        cx.notify();
                    }))
                    .child(
                        assets::icon(p.icon)
                            .size(px(18.0))
                            .flex_shrink_0()
                            .text_color(rgb(if selected { ACCENT } else { MUTED })),
                    )
                    .child(div().flex_1().child(p.title))
                    .when(selected, |row| {
                        row.child(
                            assets::icon("check")
                                .size(px(14.0))
                                .flex_shrink_0()
                                .text_color(rgb(ACCENT)),
                        )
                    })
            }))
            .child(
                div()
                    .mt_auto()
                    .pt_8()
                    .px_5()
                    .child(caption("本机 OEM 调音\n实验性音频处理")),
            )
    }

    fn render_sound(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = profile(&self.active_profile);
        let tuning = self
            .tuning
            .as_ref()
            .and_then(|t| t.profiles.get(&self.active_profile));
        let surround = tuning
            .is_some_and(|p| p.cp.surround_virtualizer_enable || p.cp.surround_decoder_enable)
            && self.active_profile != "off";
        let dialog =
            tuning.is_some_and(|p| p.cp.dialog_enhancer_enable) && self.active_profile != "off";
        let off = self.active_profile == "off";
        v_flex()
            .p_7()
            .gap_6()
            .w_full()
            .max_w(px(880.0))
            .child(
                h_flex()
                    .gap_4()
                    .items_center()
                    .child(
                        div()
                            .size(px(58.0))
                            .rounded_lg()
                            .bg(rgb(PANEL))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                assets::icon(selected.icon)
                                    .size(px(28.0))
                                    .text_color(rgb(ACCENT)),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(caption(selected.english))
                            .child(div().text_3xl().font_semibold().child(selected.title)),
                    ),
            )
            .child(
                div()
                    .text_color(rgb(MUTED))
                    .line_height(px(23.0))
                    .child(selected.description),
            )
            .child(divider())
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(div().font_semibold().child("声音个性化"))
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
                                    .gap_3()
                                    .items_center()
                                    .child(
                                        assets::icon("audio-lines")
                                            .size(px(21.0))
                                            .text_color(rgb(if surround { ACCENT } else { MUTED })),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .flex_1()
                                            .child(div().child("环绕声场"))
                                            .child(caption(if surround {
                                                "立体声展宽与早期反射"
                                            } else {
                                                "此配置未启用环绕处理"
                                            })),
                                    )
                                    .child(badge(
                                        format!("{:.2}x", self.surround_amount),
                                        surround,
                                    )),
                            )
                            .child(Slider::new(&self.surround_slider).disabled(!surround)),
                    )
                    .child(
                        v_flex()
                            .gap_3()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .child(
                                        assets::icon("mic")
                                            .size(px(21.0))
                                            .text_color(rgb(if dialog { ACCENT } else { MUTED })),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .flex_1()
                                            .child(div().child("清晰人声"))
                                            .child(caption(if dialog {
                                                "调节中置人声的存在感"
                                            } else {
                                                "此配置未启用人声增强"
                                            })),
                                    )
                                    .child(badge(format!("{:.2}x", self.dialog_amount), dialog)),
                            )
                            .child(Slider::new(&self.dialog_slider).disabled(!dialog)),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .child(caption("0 = 关闭"))
                            .child(caption("1.00x = 配置基准"))
                            .child(caption("2.00x = 增强")),
                    ),
            )
            .child(divider())
            .child(
                v_flex()
                    .gap_4()
                    .child(div().font_semibold().child("A/B 试听"))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_4()
                            .items_center()
                            .justify_between()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .p_1()
                                    .rounded_lg()
                                    .bg(rgb(PANEL))
                                    .child(
                                        Button::new("audition-processed")
                                            .small()
                                            .when(!self.bypass, |b| b.primary())
                                            .when(self.bypass, |b| b.ghost())
                                            .label("音效处理")
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
                                            .label("旁路对比")
                                            .disabled(off)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.toggle_bypass(true);
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                Switch::new("rms-match")
                                    .checked(self.level_match)
                                    .disabled(off)
                                    .label("RMS 匹配")
                                    .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                                        this.toggle_level_match(*enabled);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(caption(if off {
                        "关闭配置为完全直通，不包含匹配、校准或安全限幅。"
                    } else {
                        "保持近似 RMS 匹配，避免把音量差误认为音质差。切换后等待约 1–2 秒。"
                    })),
            )
            .child(divider())
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        assets::icon("settings-2")
                                            .size(px(17.0))
                                            .text_color(rgb(MUTED)),
                                    )
                                    .child(caption("本机扬声器校准")),
                            )
                            .child(
                                Button::new("pipeline-details")
                                    .ghost()
                                    .small()
                                    .label(if self.show_details {
                                        "收起详情"
                                    } else {
                                        "查看详情"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_details = !this.show_details;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(self.show_details, |column| {
                        column.child(self.render_pipeline())
                    })
                    .child(caption(
                        "实时调节不改写原始 XML。当前空间处理是立体声近似，并非 Atmos 解码。",
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
            .gap_2()
            .children(stages.into_iter().map(|(label, on)| {
                badge(
                    format!("{label} · {}", if on && !off { "开" } else { "关" }),
                    on && !off,
                )
            }))
            .into_any_element()
    }

    fn render_output(&self, cx: &mut Context<Self>) -> AnyElement {
        let running = self.engine.is_some();
        v_flex()
            .p_7()
            .gap_6()
            .w_full()
            .max_w(px(880.0))
            .child(caption("OUTPUT DEVICES"))
            .child(div().text_3xl().font_semibold().child("输出设备"))
            .child(
                div()
                    .text_color(rgb(MUTED))
                    .child("选择系统音频的路径，处理输出始终连接至内置 Speaker。"),
            )
            .child(divider())
            .child(
                v_flex()
                    .gap_5()
                    .p_5()
                    .rounded_lg()
                    .bg(rgb(PANEL))
                    .border_1()
                    .border_color(rgb(LINE))
                    .child(
                        h_flex()
                            .gap_4()
                            .items_center()
                            .child(assets::app_mark().size(px(42.0)).text_color(rgb(ACCENT)))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(div().text_lg().font_semibold().child("OpenDolby"))
                                    .child(caption("虚拟音频输出 · OEM 调音 / 实时 DSP")),
                            )
                            .child(badge(
                                if running {
                                    "引擎运行"
                                } else {
                                    "尚未启动"
                                },
                                running,
                            )),
                    )
                    .child(caption("应用音频 → OpenDolby → 内置扬声器"))
                    .child(
                        Button::new("route-dolby")
                            .primary()
                            .label("使用此输出")
                            .disabled(!running)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_default_sink(OUR_SINK_PATTERN, "OpenDolby", true);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_5()
                    .p_5()
                    .rounded_lg()
                    .bg(rgb(PANEL))
                    .border_1()
                    .border_color(rgb(LINE))
                    .child(
                        h_flex()
                            .gap_4()
                            .items_center()
                            .child(
                                assets::icon("speaker")
                                    .size(px(32.0))
                                    .text_color(rgb(MUTED)),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_lg().font_semibold().child("内置扬声器"))
                                    .child(caption("Realtek ALC294 · Raptor Lake-P/U/H cAVS")),
                            ),
                    )
                    .child(caption("直接使用硬件输出，不经过 OpenDolby 处理。"))
                    .child(
                        Button::new("route-speaker")
                            .outline()
                            .label("恢复 Speaker")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_default_sink(SPEAKER_SINK_PATTERN, "Speaker", false);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(assets::icon("info").size(px(16.0)).text_color(rgb(MUTED)))
                    .child(caption(
                        "停止引擎时，如当前使用 OpenDolby，会先恢复 Speaker。",
                    )),
            )
            .into_any_element()
    }

    fn render_about(&self) -> AnyElement {
        v_flex().p_7().gap_6().w_full().max_w(px(820.0))
            .child(caption("ABOUT THE PROJECT"))
            .child(h_flex().items_center().gap_5()
                .child(div().size(px(80.0)).rounded_xl().bg(rgb(NAV)).flex().items_center().justify_center()
                    .child(assets::app_mark().size(px(60.0)).text_color(rgb(TEXT))))
                .child(v_flex().gap_2().child(div().text_3xl().font_semibold().child("OpenDolby"))
                    .child(caption("原生 Linux 扬声器 DSP · v0.1.0"))))
            .child(divider())
            .child(div().text_lg().font_semibold().child("让本机调音，回到 Linux。"))
            .child(div().text_color(rgb(MUTED)).line_height(px(25.0))
                .child("基于本机 ASUS 调音 XML 的实验性音频处理工具，使用 Rust、PipeWire 和 GPUI-Kit。界面借鉴 Dolby Access 的设置层级，项目标识为独立原创设计。"))
            .child(v_flex().gap_3().p_5().rounded_lg().bg(rgb(PANEL))
                .child(div().font_semibold().child("实验性实现，不冒充官方产品"))
                .child(caption("不是 Dolby 官方软件、认证实现或 Atmos 对象音频解码器。"))
                .child(caption("RMS 匹配不是感知等响度，校准效果未与 Windows APO 做声学等价验证。")))
            .child(v_flex().gap_3()
                .child(div().font_semibold().child("开源图标与致谢"))
                .child(caption("界面图标：Lucide · ISC / MIT，许可随项目 assets/icons/LICENSE 分发。"))
                .child(caption("项目图标：OpenDolby 独立图形，不使用 Dolby 双 D 标识。")))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_have_unique_real_engine_ids_and_clear_fallback() {
        let mut ids = std::collections::HashSet::new();
        for p in PROFILES {
            assert!(ids.insert(p.id));
            assert!(!p.title.is_empty());
            assert!(!p.icon.is_empty());
            use gpui_kit::AssetSource;
            assert!(assets::application_assets()
                .load(&format!("icons/{}.svg", p.icon))
                .unwrap_or_else(|_| panic!("missing profile icon: {}", p.icon))
                .is_some());
        }
        assert_eq!(PROFILES.len(), 10);
        assert_eq!(profile("music").title, "音乐");
        assert_eq!(profile("missing").id, "dynamic");
    }
}
