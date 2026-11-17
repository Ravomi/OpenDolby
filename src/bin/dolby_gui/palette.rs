use gpui_kit::component::theme::{Theme, ThemeMode, ThemeToken};
use gpui_kit::{px, rgb, App, Hsla, WindowAppearance};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Palette {
    pub(super) bg: u32,
    pub(super) nav: u32,
    pub(super) panel: u32,
    pub(super) line: u32,
    pub(super) text: u32,
    pub(super) muted: u32,
    pub(super) accent: u32,
    pub(super) cyan: u32,
    pub(super) violet: u32,
    pub(super) soft_blue: u32,
    pub(super) soft_cyan: u32,
    pub(super) soft_violet: u32,
    pub(super) primary_foreground: u32,
    pub(super) thumb: u32,
}

#[derive(Clone, Copy)]
struct ControlTokens {
    primary_hover: u32,
    primary_active: u32,
    secondary_hover: u32,
    secondary_active: u32,
    switch_off: u32,
}

impl Palette {
    pub(super) const fn light() -> Self {
        Self {
            bg: 0xf5f4ef,
            nav: 0xefeee8,
            panel: 0xffffff,
            line: 0xdadbd5,
            text: 0x232622,
            muted: 0x646c70,
            accent: 0x2559ca,
            cyan: 0x127a8d,
            violet: 0x77648b,
            soft_blue: 0xe5eaf0,
            soft_cyan: 0xe2f7fa,
            soft_violet: 0xf0ebff,
            primary_foreground: 0xffffff,
            thumb: 0xffffff,
        }
    }

    pub(super) const fn dark() -> Self {
        Self {
            bg: 0x111317,
            nav: 0x171a20,
            panel: 0x1d2128,
            line: 0x2f3540,
            text: 0xedeff2,
            muted: 0x9aa3b2,
            accent: 0x2563eb,
            cyan: 0x168ea3,
            violet: 0x9d86b8,
            soft_blue: 0x1e293b,
            soft_cyan: 0x152c36,
            soft_violet: 0x272038,
            primary_foreground: 0xffffff,
            thumb: 0xf1f5f9,
        }
    }

    const fn control_tokens(is_dark: bool) -> ControlTokens {
        if is_dark {
            ControlTokens {
                primary_hover: 0x3b82f6,
                primary_active: 0x1d4ed8,
                secondary_hover: 0x283548,
                secondary_active: 0x334155,
                switch_off: 0x475569,
            }
        } else {
            ControlTokens {
                primary_hover: 0x1d4ed8,
                primary_active: 0x1e40af,
                secondary_hover: 0xdbeafe,
                secondary_active: 0xc7d2fe,
                switch_off: 0x94a3b8,
            }
        }
    }
}

#[inline]
pub(super) fn hsla(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub(super) fn colors(cx: &App) -> Palette {
    if Theme::global(cx).is_dark() {
        Palette::dark()
    } else {
        Palette::light()
    }
}

/// Pick a pure monochrome mark from the actual surface, not the OS preference.
pub(super) fn brand_mark_color(background: u32) -> u32 {
    let linear = |channel: u32| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear((background >> 16) & 0xff)
        + 0.7152 * linear((background >> 8) & 0xff)
        + 0.0722 * linear(background & 0xff);
    if luminance > 0.179 {
        0x000000
    } else {
        0xffffff
    }
}

fn resolve_mode(preference: ThemePreference, appearance: WindowAppearance) -> ThemeMode {
    match preference {
        ThemePreference::System => match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeMode::Light,
        },
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
    }
}

pub(super) fn apply(preference: ThemePreference, cx: &mut App) {
    apply_with_appearance(preference, cx.window_appearance(), cx);
}

/// Applies the resolved light or dark palette for an explicit window appearance
/// and synchronizes both legacy `colors` and `tokens` used by gpui-kit 0.9.1 components.
pub(super) fn apply_with_appearance(
    preference: ThemePreference,
    appearance: WindowAppearance,
    cx: &mut App,
) {
    let mode = resolve_mode(preference, appearance);
    Theme::change(mode, None, cx);

    let is_dark = matches!(mode, ThemeMode::Dark);
    let p = if is_dark {
        Palette::dark()
    } else {
        Palette::light()
    };
    let ctrl = Palette::control_tokens(is_dark);

    Theme::update(cx, |theme| {
        theme.radius = px(4.0);
        let c = &mut theme.colors;
        c.background = hsla(p.bg);
        c.foreground = hsla(p.text);
        c.popover = hsla(p.panel);
        c.popover_foreground = hsla(p.text);
        c.sidebar = hsla(p.nav);
        c.sidebar_foreground = hsla(p.text);
        c.sidebar_border = hsla(p.line);
        c.sidebar_accent = hsla(p.soft_blue);
        c.sidebar_accent_foreground = hsla(p.text);
        c.border = hsla(p.line);
        c.input = hsla(p.line);
        c.ring = hsla(p.accent);

        c.muted = hsla(p.soft_blue);
        c.muted_foreground = hsla(p.muted);
        c.accent = hsla(p.soft_cyan);
        c.accent_foreground = hsla(p.text);

        c.primary = hsla(p.accent);
        c.primary_hover = hsla(ctrl.primary_hover);
        c.primary_active = hsla(ctrl.primary_active);
        c.primary_foreground = hsla(p.primary_foreground);

        c.secondary = hsla(p.soft_blue);
        c.secondary_hover = hsla(ctrl.secondary_hover);
        c.secondary_active = hsla(ctrl.secondary_active);
        c.secondary_foreground = hsla(p.text);

        c.info = hsla(p.cyan);
        c.info_foreground = hsla(p.primary_foreground);

        c.list = hsla(p.panel);
        c.list_active = hsla(p.soft_blue);
        c.list_active_border = hsla(p.accent);
        c.list_hover = hsla(p.soft_cyan);
        c.selection = hsla(p.soft_violet);

        c.slider_bar = hsla(p.cyan);
        c.slider_thumb = hsla(p.thumb);
        c.switch = hsla(ctrl.switch_off);
        c.switch_thumb = hsla(p.thumb);

        // Keep native controls flat, without decorative button gradients.
        let primary_token = ThemeToken::from(hsla(p.accent));

        theme.tokens.primary = primary_token;
        theme.tokens.button = ThemeToken::from(hsla(p.panel));
        theme.tokens.button_hover = ThemeToken::from(hsla(p.soft_blue));
        theme.tokens.button_active = ThemeToken::from(hsla(ctrl.secondary_hover));
        theme.tokens.button_primary = primary_token;
        theme.tokens.button_primary_hover = ThemeToken::from(hsla(ctrl.primary_hover));
        theme.tokens.button_primary_active = ThemeToken::from(hsla(ctrl.primary_active));
        theme.tokens.button_secondary = ThemeToken::from(hsla(p.soft_blue));
        theme.tokens.button_secondary_hover = ThemeToken::from(hsla(ctrl.secondary_hover));
        theme.tokens.button_secondary_active = ThemeToken::from(hsla(ctrl.secondary_active));
        theme.tokens.slider_bar = ThemeToken::from(hsla(p.cyan));
        theme.tokens.slider_thumb = ThemeToken::from(hsla(p.thumb));
        theme.tokens.switch = ThemeToken::from(hsla(ctrl.switch_off));
        theme.tokens.switch_thumb = ThemeToken::from(hsla(p.thumb));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative_luminance(hex: u32) -> f64 {
        let to_linear = |channel: u8| {
            let srgb = f64::from(channel) / 255.0;
            if srgb <= 0.03928 {
                srgb / 12.92
            } else {
                ((srgb + 0.055) / 1.055).powf(2.4)
            }
        };
        let r = to_linear(((hex >> 16) & 0xff) as u8);
        let g = to_linear(((hex >> 8) & 0xff) as u8);
        let b = to_linear((hex & 0xff) as u8);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    fn contrast_ratio(a: u32, b: u32) -> f64 {
        let la = relative_luminance(a);
        let lb = relative_luminance(b);
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn theme_preference_roundtrip() {
        for pref in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            assert_eq!(ThemePreference::parse(pref.as_str()), Some(pref));
            assert!(!pref.label().is_empty());
        }
        assert_eq!(ThemePreference::default(), ThemePreference::System);
        assert_eq!(ThemePreference::parse("DARK"), Some(ThemePreference::Dark));
        assert_eq!(ThemePreference::parse("unknown"), None);
    }

    #[test]
    fn brand_mark_is_black_on_light_and_white_on_dark_surfaces() {
        let light = Palette::light();
        for background in [light.nav, light.bg, light.panel, 0xffffff] {
            assert_eq!(brand_mark_color(background), 0x000000);
            assert!(contrast_ratio(brand_mark_color(background), background) >= 4.5);
        }
        let dark = Palette::dark();
        for background in [dark.nav, dark.bg, dark.panel, 0x000000, 0x101114] {
            assert_eq!(brand_mark_color(background), 0xffffff);
            assert!(contrast_ratio(brand_mark_color(background), background) >= 4.5);
        }
    }

    #[test]
    fn light_and_dark_palettes_meet_readable_contrast() {
        for p in [Palette::light(), Palette::dark()] {
            assert!(contrast_ratio(p.text, p.bg) >= 10.0);
            assert!(contrast_ratio(p.text, p.panel) >= 10.0);
            assert!(contrast_ratio(p.muted, p.bg) >= 4.5);
            assert!(contrast_ratio(p.muted, p.panel) >= 4.5);
            assert!(contrast_ratio(p.primary_foreground, p.accent) >= 4.5);
            assert!(contrast_ratio(p.primary_foreground, p.cyan) >= 3.5);
            assert!(contrast_ratio(p.thumb, p.bg) >= 3.0);
        }
    }
}
