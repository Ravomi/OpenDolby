use super::{assets, palette::*, DolbyGui, VIRTUAL_SINK};
use dolby_rs::audio_devices::DeviceKind;
use gpui_kit::base::StyledExt;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, Disableable, Sizable,
};
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, Div, SharedString};

fn note(text: impl Into<SharedString>) -> Div {
    div().text_xs().text_color(rgb(MUTED)).child(text.into())
}

fn rule() -> Div {
    div().w_full().h(px(1.0)).flex_shrink_0().bg(rgb(LINE))
}

impl DolbyGui {
    pub(super) fn render_output(&self, cx: &mut Context<Self>) -> AnyElement {
        let running = self.engine.is_some();
        let routed_to_virtual = self.default_sink.as_deref() == Some(VIRTUAL_SINK);
        let target_label: SharedString = self
            .target_sink
            .as_deref()
            .and_then(|name| {
                self.outputs
                    .iter()
                    .find(|d| d.name == name)
                    .map(|d| d.description.clone())
            })
            .unwrap_or_else(|| {
                self.target_sink
                    .clone()
                    .unwrap_or_else(|| "未选择内置扬声器".to_string())
            })
            .into();

        let target_available = self
            .target_sink
            .as_deref()
            .map(|name| {
                self.outputs.iter().any(|d| {
                    d.name == name && d.available && d.is_internal_speaker && !d.is_virtual
                })
            })
            .unwrap_or(false);

        let mut sorted_outputs = self.outputs.clone();
        sorted_outputs.sort_by(|a, b| {
            b.available
                .cmp(&a.available)
                .then_with(|| b.is_internal_speaker.cmp(&a.is_internal_speaker))
                .then_with(|| a.description.cmp(&b.description))
        });

        let mut page = v_flex()
            .w_full()
            .max_w(px(1000.0))
            .px_8()
            .pt_8()
            .pb_6()
            .gap_6()
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(38.0))
                            .line_height(px(44.0))
                            .font_semibold()
                            .child("输出设备"),
                    )
                    .child(note("管理内置扬声器处理目标与系统默认音频路由。")),
            )
            .child(rule())
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .flex_1()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .child(div().font_semibold().child("当前处理目标"))
                                    .child(
                                        div().text_color(rgb(TEXT)).truncate().child(target_label),
                                    ),
                            )
                            .child(note(if running {
                                if routed_to_virtual {
                                    "虚拟输出已启动，系统默认音频正经由 OpenDolby 处理。"
                                } else {
                                    "音效引擎已启动，但系统默认输出尚未切至虚拟设备。"
                                }
                            } else {
                                "音效引擎未启动；启动后可将系统音频路由至虚拟输出。"
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_shrink_0()
                            .child(
                                Button::new("route-virtual-sink")
                                    .label(if routed_to_virtual {
                                        "已路由至 OpenDolby"
                                    } else {
                                        "路由到 OpenDolby"
                                    })
                                    .small()
                                    .primary()
                                    .disabled(!running || !target_available || routed_to_virtual)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_output(VIRTUAL_SINK);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("refresh-outputs")
                                    .label("刷新")
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.refresh_output_devices();
                                        cx.notify();
                                    })),
                            ),
                    ),
            );

        if let Some(err) = &self.device_error {
            page = page.child(
                v_flex()
                    .w_full()
                    .py_2()
                    .border_l_2()
                    .border_color(rgb(ACCENT))
                    .pl_3()
                    .gap_1()
                    .child(div().font_semibold().child("设备读取失败"))
                    .child(note(err.clone())),
            );
        } else if self.target_sink.is_some() && !target_available {
            page = page.child(
                v_flex()
                    .w_full()
                    .py_2()
                    .border_l_2()
                    .border_color(rgb(ACCENT))
                    .pl_3()
                    .gap_1()
                    .child(div().font_semibold().child("适配警告"))
                    .child(note(
                        "当前处理目标不可用或非内置扬声器，已阻止错误应用本机扬声器调音。",
                    )),
            );
        }

        page = page.child(rule()).child(
            v_flex()
                .w_full()
                .gap_3()
                .child(div().font_semibold().child("设备列表"))
                .children(if sorted_outputs.is_empty() {
                    vec![note("未发现音频输出设备。").into_any_element()]
                } else {
                    sorted_outputs
                        .into_iter()
                        .enumerate()
                        .map(|(idx, dev)| {
                            let is_target = self.target_sink.as_deref() == Some(dev.name.as_str());
                            let is_default =
                                self.default_sink.as_deref() == Some(dev.name.as_str());
                            let kind_label = match dev.kind {
                                DeviceKind::InternalSpeaker => "内置扬声器",
                                DeviceKind::Headphones => "耳机",
                                DeviceKind::Hdmi => "HDMI / DisplayPort",
                                DeviceKind::Bluetooth => "蓝牙",
                                DeviceKind::Usb => "USB 音频",
                                DeviceKind::Virtual => "虚拟输出",
                                DeviceKind::Unknown => "其他输出",
                            };
                            let avail_label = if dev.available { "可用" } else { "不可用" };

                            let can_set_target = !running
                                && dev.available
                                && dev.is_internal_speaker
                                && !dev.is_virtual
                                && !is_target;
                            let can_direct = dev.available && !dev.is_virtual && !is_default;

                            let name_for_target = dev.name.clone();
                            let name_for_direct = dev.name.clone();

                            v_flex()
                                .w_full()
                                .child(
                                    h_flex()
                                        .w_full()
                                        .py_3()
                                        .gap_4()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .flex_1()
                                                .min_w_0()
                                                .child(
                                                    h_flex()
                                                        .gap_2()
                                                        .items_center()
                                                        .child(
                                                            div()
                                                                .font_medium()
                                                                .truncate()
                                                                .child(dev.description),
                                                        )
                                                        .when(is_target, |s| {
                                                            s.child(note("· 处理目标"))
                                                        })
                                                        .when(is_default, |s| {
                                                            s.child(note("· 当前默认"))
                                                        }),
                                                )
                                                .child(note(format!(
                                                    "{kind_label} · {avail_label}"
                                                ))),
                                        )
                                        .child(h_flex().gap_2().flex_shrink_0().when(
                                            !dev.is_virtual,
                                            |row| {
                                                row.when(dev.is_internal_speaker, |row| {
                                                    row.child(
                                                        Button::new(SharedString::from(format!(
                                                            "target-{idx}"
                                                        )))
                                                        .label(if is_target {
                                                            "当前目标"
                                                        } else {
                                                            "设为处理目标"
                                                        })
                                                        .small()
                                                        .outline()
                                                        .disabled(!can_set_target)
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.select_output_target(
                                                                    &name_for_target,
                                                                );
                                                                cx.notify();
                                                            },
                                                        )),
                                                    )
                                                })
                                                .child(
                                                    Button::new(SharedString::from(format!(
                                                        "direct-{idx}"
                                                    )))
                                                    .label(if is_default {
                                                        "当前直连"
                                                    } else {
                                                        "直连物理设备"
                                                    })
                                                    .small()
                                                    .outline()
                                                    .disabled(!can_direct)
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.set_output(&name_for_direct);
                                                        cx.notify();
                                                    })),
                                                )
                                            },
                                        )),
                                )
                                .child(rule())
                                .into_any_element()
                        })
                        .collect()
                }),
        );

        page.child(note(
            "仅对内置扬声器使用同机 OEM 调音文件。识别到设备不代表该机型已完成适配验证。",
        ))
        .into_any_element()
    }

    pub(super) fn render_about(&self) -> AnyElement {
        let version = env!("CARGO_PKG_VERSION");
        v_flex()
            .w_full()
            .max_w(px(1000.0))
            .px_8()
            .pt_8()
            .pb_6()
            .gap_6()
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(
                                assets::app_mark()
                                    .w(px(36.0))
                                    .h(px(28.0))
                                    .flex_shrink_0()
                                    .text_color(rgb(brand_mark_color(BG))),
                            )
                            .child(
                                div()
                                    .text_size(px(38.0))
                                    .line_height(px(44.0))
                                    .font_semibold()
                                    .child("OpenDolby"),
                            ),
                    )
                    .child(note(format!(
                        "版本 {version} · 原生 Linux 笔记本扬声器调音与 PipeWire 实时处理工具"
                    ))),
            )
            .child(rule())
            .child(
                v_flex()
                    .gap_3()
                    .child(div().font_semibold().child("技术与安全说明"))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(note("• 直接解析本机 OEM 调音 XML，以 48 kHz / 2 ch 实时运行 DSP 处理。"))
                            .child(note("• 仅对已确认的内置扬声器应用本机调音，避免误将扬声器校准施加到耳机或外接设备。"))
                            .child(note("• 退出或目标设备断开时自动恢复系统默认输出，不修改原始硬件调音文件。")),
                    ),
            )
            .into_any_element()
    }
}
