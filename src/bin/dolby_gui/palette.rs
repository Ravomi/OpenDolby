use gpui_kit::component::theme::{Theme, ThemeMode, ThemeToken};
use gpui_kit::{px, rgb, Hsla};

pub(super) const BG: u32 = 0xf5f4ef;
pub(super) const NAV: u32 = 0xefeee8;
pub(super) const PANEL: u32 = 0xffffff;
pub(super) const LINE: u32 = 0xdadbd5;
pub(super) const TEXT: u32 = 0x232622;
pub(super) const MUTED: u32 = 0x646c70;
pub(super) const ACCENT: u32 = 0x2559ca;
pub(super) const CYAN: u32 = 0x127a8d;
pub(super) const VIOLET: u32 = 0x77648b;
pub(super) const SOFT_BLUE: u32 = 0xe5eaf0;
pub(super) const SOFT_CYAN: u32 = 0xe2f7fa;
pub(super) const SOFT_VIOLET: u32 = 0xf0ebff;

const PRIMARY_HOVER: u32 = 0x1d4ed8;
const PRIMARY_ACTIVE: u32 = 0x1e40af;
const SECONDARY_HOVER: u32 = 0xdbeafe;
const SECONDARY_ACTIVE: u32 = 0xc7d2fe;
const SWITCH_OFF: u32 = 0x94a3b8;
const WHITE: u32 = 0xffffff;

#[inline]
pub(super) fn hsla(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Pick a pure monochrome mark from the actual surface, not the OS preference.
/// The custom light header can remain light even when the desktop is dark.
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
        WHITE
    }
}

/// Applies the warm neutral surfaces and restrained audio-control accents.
/// and synchronizes both legacy `colors` and `tokens` used by 0.9.1 components.
pub(super) fn apply(cx: &mut gpui_kit::App) {
    Theme::change(ThemeMode::Light, None, cx);

    Theme::update(cx, |theme| {
        theme.radius = px(4.0);
        let c = &mut theme.colors;
        c.background = hsla(BG);
        c.foreground = hsla(TEXT);
        c.popover = hsla(PANEL);
        c.popover_foreground = hsla(TEXT);
        c.sidebar = hsla(NAV);
        c.sidebar_foreground = hsla(TEXT);
        c.sidebar_border = hsla(LINE);
        c.sidebar_accent = hsla(SOFT_BLUE);
        c.sidebar_accent_foreground = hsla(TEXT);
        c.border = hsla(LINE);
        c.input = hsla(LINE);
        c.ring = hsla(ACCENT);

        c.muted = hsla(SOFT_BLUE);
        c.muted_foreground = hsla(MUTED);
        c.accent = hsla(SOFT_CYAN);
        c.accent_foreground = hsla(TEXT);

        c.primary = hsla(ACCENT);
        c.primary_hover = hsla(PRIMARY_HOVER);
        c.primary_active = hsla(PRIMARY_ACTIVE);
        c.primary_foreground = hsla(WHITE);

        c.secondary = hsla(SOFT_BLUE);
        c.secondary_hover = hsla(SECONDARY_HOVER);
        c.secondary_active = hsla(SECONDARY_ACTIVE);
        c.secondary_foreground = hsla(TEXT);

        c.info = hsla(CYAN);
        c.info_foreground = hsla(WHITE);

        c.list = hsla(PANEL);
        c.list_active = hsla(SOFT_BLUE);
        c.list_active_border = hsla(ACCENT);
        c.list_hover = hsla(SOFT_CYAN);
        c.selection = hsla(SOFT_VIOLET);

        c.slider_bar = hsla(CYAN);
        c.slider_thumb = hsla(WHITE);
        c.switch = hsla(SWITCH_OFF);
        c.switch_thumb = hsla(WHITE);

        // Keep native controls flat, without decorative button gradients.
        let primary_token = ThemeToken::from(hsla(ACCENT));

        theme.tokens.primary = primary_token;
        theme.tokens.button = ThemeToken::from(hsla(PANEL));
        theme.tokens.button_hover = ThemeToken::from(hsla(SOFT_BLUE));
        theme.tokens.button_active = ThemeToken::from(hsla(SECONDARY_HOVER));
        theme.tokens.button_primary = primary_token;
        theme.tokens.button_primary_hover = ThemeToken::from(hsla(PRIMARY_HOVER));
        theme.tokens.button_primary_active = ThemeToken::from(hsla(PRIMARY_ACTIVE));
        theme.tokens.button_secondary = ThemeToken::from(hsla(SOFT_BLUE));
        theme.tokens.button_secondary_hover = ThemeToken::from(hsla(SECONDARY_HOVER));
        theme.tokens.button_secondary_active = ThemeToken::from(hsla(SECONDARY_ACTIVE));
        theme.tokens.slider_bar = ThemeToken::from(hsla(CYAN));
        theme.tokens.slider_thumb = ThemeToken::from(hsla(WHITE));
        theme.tokens.switch = ThemeToken::from(hsla(SWITCH_OFF));
        theme.tokens.switch_thumb = ThemeToken::from(hsla(WHITE));
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
    fn brand_mark_is_black_on_light_and_white_on_dark_surfaces() {
        for background in [NAV, BG, PANEL, 0xffffff] {
            assert_eq!(brand_mark_color(background), 0x000000);
            assert!(contrast_ratio(brand_mark_color(background), background) >= 4.5);
        }
        for background in [0x000000, 0x101114, 0x182336] {
            assert_eq!(brand_mark_color(background), 0xffffff);
            assert!(contrast_ratio(brand_mark_color(background), background) >= 4.5);
        }
    }

    #[test]
    fn text_and_controls_meet_readable_contrast() {
        assert!(contrast_ratio(TEXT, BG) >= 10.0);
        assert!(contrast_ratio(TEXT, PANEL) >= 12.0);
        assert!(contrast_ratio(MUTED, BG) >= 4.5);
        assert!(contrast_ratio(MUTED, PANEL) >= 4.5);
        assert!(contrast_ratio(WHITE, ACCENT) >= 4.5);
        assert!(contrast_ratio(WHITE, CYAN) >= 3.5);
    }
}
