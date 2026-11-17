//! Native settings with working theme, routing preferences and local tuning selection.
use super::{
    assets,
    palette::{self, colors, Palette, ThemePreference},
    preferences, DolbyGui, Page,
};
use dolby_rs::tuning::DeviceTuning;
use gpui_kit::base::StyledExt;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex,
    switch::Switch,
    v_flex, Disableable, Sizable,
};
use gpui_kit::{
    div, prelude::*, px, rgb, AnyElement, Context, Div, PathPromptOptions, SharedString,
};
use std::path::PathBuf;

fn note(text: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .text_xs()
        .text_color(rgb(p.muted))
        .line_height(px(20.0))
        .child(text.into())
}
fn rule(p: Palette) -> Div {
    div().h(px(1.0)).w_full().flex_shrink_0().bg(rgb(p.line))
}

impl DolbyGui {
    pub(super) fn persist_settings(&mut self) {
        let preferences = preferences::Preferences {
            theme: self.theme_preference,
            auto_target: self.auto_target,
            manual_target: self.target_sink.clone(),
            xml_path: Some(self.xml_path.clone()),
        };
        self.settings_error = preferences.save().err().map(|error| format!("{error:#}"));
    }

    pub(super) fn set_theme(&mut self, preference: ThemePreference, cx: &mut Context<Self>) {
        self.theme_preference = preference;
        palette::apply(preference, cx);
        self.persist_settings();
        self.status = format!("外观 · {}", preference.label()).into();
        cx.notify();
    }

    pub(super) fn cycle_theme(&mut self, cx: &mut Context<Self>) {
        let next = match self.theme_preference {
            ThemePreference::System => ThemePreference::Light,
            ThemePreference::Light => ThemePreference::Dark,
            ThemePreference::Dark => ThemePreference::System,
        };
        self.set_theme(next, cx);
    }

    pub(super) fn set_auto_target(&mut self, automatic: bool) {
        self.auto_target = automatic;
        self.refresh_output_devices();
        self.persist_settings();
        self.status = if automatic {
            "自动识别已开启，仅选择可用的内置扬声器。"
        } else {
            "已改为手动选择，当前处理目标保持不变。"
        }
        .into();
    }

    fn load_tuning(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.engine.is_some() {
            self.status = "请先停止音效，再更换或重新加载调音文件。".into();
            cx.notify();
            return;
        }
        match DeviceTuning::from_xml_file(&path).and_then(|tuning| {
            let profile = if tuning.profiles.contains_key(&self.active_profile) {
                self.active_profile.clone()
            } else {
                [
                    "dynamic",
                    "music",
                    "movie",
                    "game",
                    "voice",
                    "personalize_user1",
                    "personalize_user2",
                    "personalize_user3",
                    "off",
                ]
                .into_iter()
                .find(|name| tuning.profiles.contains_key(*name))
                .map(str::to_owned)
                .ok_or_else(|| anyhow::anyhow!("文件不包含界面可使用的调音预设"))?
            };
            Ok((tuning, profile))
        }) {
            Ok((tuning, profile)) => {
                self.tuning = Some(tuning);
                self.active_profile = profile;
                self.xml_path = path;
                self.persist_settings();
                self.status = "本地调音文件已加载，启动音效后生效。".into();
            }
            Err(error) => {
                self.settings_error = Some(format!("调音文件加载失败，原配置未更改：{error:#}"));
                self.status = "无法加载调音文件，原配置已保留。".into();
            }
        }
        cx.notify();
    }

    fn choose_tuning(&mut self, cx: &mut Context<Self>) {
        if self.engine.is_some() {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择这台设备的 OEM 调音 XML".into()),
        });
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| this.load_tuning(path, cx));
                }
            }
            Ok(Ok(None)) => {}
            result => {
                let _ = this.update(cx, |this, cx| {
                    this.settings_error = Some(format!("无法打开本地文件选择器：{result:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = colors(cx);
        let running = self.engine.is_some();
        v_flex().px_8().pt_8().pb_6().gap_5().w_full().max_w(px(1000.0))
            .child(v_flex().gap_2()
                .child(h_flex().gap_3().items_center()
                    .child(assets::icon("settings-2").size(px(26.0)).text_color(rgb(p.text)))
                    .child(div().text_size(px(38.0)).line_height(px(48.0)).font_semibold().child("设置")))
                .child(note("外观、设备选择与本地调音文件。更改会自动保存。", p)))
            .child(rule(p))
            .child(v_flex().gap_4()
                .child(div().font_semibold().child("外观"))
                .child(div().flex().flex_wrap().gap_3().children([
                    (ThemePreference::Light, "sun"), (ThemePreference::Dark, "moon"),
                    (ThemePreference::System, "monitor"),
                ].into_iter().map(|(preference, icon)| {
                    Button::new(SharedString::from(format!("theme-{}", preference.as_str())))
                        .when(self.theme_preference == preference, |button| button.primary())
                        .when(self.theme_preference != preference, |button| button.outline())
                        .label(preference.label())
                        .icon(gpui_kit::component::Icon::default().path(format!("icons/{icon}.svg")))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_theme(preference, cx)))
                })))
                .child(note("跟随系统会响应桌面的明暗变化。切换外观不重启音效。", p)))
            .child(rule(p))
            .child(v_flex().gap_3()
                .child(div().font_semibold().child("设备识别"))
                .child(div().child(self.hardware_name.clone()))
                .child(Switch::new("settings-auto-target").checked(self.auto_target)
                    .label("自动选择内置扬声器")
                    .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                        this.set_auto_target(*enabled); cx.notify();
                    })))
                .child(h_flex().items_center().justify_between().gap_3()
                    .child(note("每 2 秒检测设备变化。耳机、蓝牙、USB 与 HDMI 不套用扬声器调音。", p))
                    .child(Button::new("settings-devices").ghost().small().label("查看设备 →")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.page = Page::Output; this.refresh_output_devices(); cx.notify();
                        })))))
            .child(rule(p))
            .child(v_flex().gap_3()
                .child(h_flex().justify_between().items_center().gap_3()
                    .child(div().font_semibold().child("调音文件"))
                    .child(note(if self.tuning.is_some() { "已加载" } else { "未加载" }, p)))
                .child(div().text_xs().text_color(rgb(p.muted)).id("tuning-path").overflow_x_scroll().child(self.xml_path.display().to_string()))
                .when_some(self.tuning.as_ref(), |view, tuning| view.child(note(
                    format!("XML {} · {} 个配置", tuning.xml_version, tuning.profiles.len()), p)))
                .child(h_flex().gap_3()
                    .child(Button::new("choose-tuning").outline().small().label("选择本机 XML…")
                        .disabled(running).on_click(cx.listener(|this, _, _, cx| this.choose_tuning(cx))))
                    .child(Button::new("reload-tuning").ghost().small().label("重新加载")
                        .disabled(running).on_click(cx.listener(|this, _, _, cx| {
                            this.load_tuning(this.xml_path.clone(), cx);
                        }))))
                .child(note(if running { "更换文件前请先停止音效。原始 XML 不会被改写。" }
                    else { "请只使用这台机器自己的 OEM 调音。文件仅在本地读取，不上传。" }, p)))
            .when_some(self.settings_error.as_ref(), |view, error| view.child(
                div().border_l_2().border_color(rgb(p.violet)).pl_3().child(note(error.clone(), p))))
            .child(rule(p))
            .child(note("关闭面板后音效继续运行。停止音效或退出时，仅在系统仍使用 OpenDolby 输出时恢复原设备。", p))
            .into_any_element()
    }
}
