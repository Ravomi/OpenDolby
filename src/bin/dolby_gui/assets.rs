use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString, Svg, svg};

const OPENDOLBY_SVG: &[u8] = include_bytes!("../../../assets/opendolby.svg");

const LUCIDE_ICONS: &[(&str, &[u8])] = &[
    ("arrow-right", include_bytes!("../../../assets/icons/arrow-right.svg")),
    ("audio-lines", include_bytes!("../../../assets/icons/audio-lines.svg")),
    ("check", include_bytes!("../../../assets/icons/check.svg")),
    ("clapperboard", include_bytes!("../../../assets/icons/clapperboard.svg")),
    ("gamepad-2", include_bytes!("../../../assets/icons/gamepad-2.svg")),
    ("headphones", include_bytes!("../../../assets/icons/headphones.svg")),
    ("house", include_bytes!("../../../assets/icons/house.svg")),
    ("info", include_bytes!("../../../assets/icons/info.svg")),
    ("mic", include_bytes!("../../../assets/icons/mic.svg")),
    ("power", include_bytes!("../../../assets/icons/power.svg")),
    ("rotate-ccw", include_bytes!("../../../assets/icons/rotate-ccw.svg")),
    ("settings-2", include_bytes!("../../../assets/icons/settings-2.svg")),
    ("sliders-horizontal", include_bytes!("../../../assets/icons/sliders-horizontal.svg")),
    ("sparkles", include_bytes!("../../../assets/icons/sparkles.svg")),
    ("speaker", include_bytes!("../../../assets/icons/speaker.svg")),
    ("volume-2", include_bytes!("../../../assets/icons/volume-2.svg")),
];

/// Embedded GPUI asset source combining OpenDolby's bundled Lucide icons and app mark
/// with `gpui_kit::assets::Assets` as a fallback.
#[derive(Debug, Default, Clone, Copy)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = lookup_embedded(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = gpui_kit::assets::Assets.list(path)?;
        if path.is_empty() || path == "icons" || path == "icons/" {
            for (name, _) in LUCIDE_ICONS {
                items.push(SharedString::from(format!("icons/{name}.svg")));
            }
            items.push(SharedString::from("opendolby.svg"));
        }
        Ok(items)
    }
}

fn normalize_icon_name(name: &str) -> &str {
    name.trim_start_matches("icons/")
        .trim_end_matches(".svg")
}

fn lookup_embedded(path: &str) -> Option<&'static [u8]> {
    let key = normalize_icon_name(path);
    if key == "opendolby" {
        return Some(OPENDOLBY_SVG);
    }
    LUCIDE_ICONS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, bytes)| *bytes)
}

/// Returns the [`AssetSource`] to register with `gpui_kit::application().with_assets(...)`.
pub fn application_assets() -> AppAssets {
    AppAssets
}

/// Returns a [`gpui_kit::Svg`] element for a bundled Lucide icon (e.g. `"house"`, `"speaker"`)
/// or `"opendolby"` for the app mark.
pub fn icon(name: &str) -> Svg {
    let key = normalize_icon_name(name);
    let path = if key == "opendolby" {
        "opendolby.svg".to_string()
    } else {
        format!("icons/{key}.svg")
    };
    svg().path(path)
}

/// Returns the monochrome OpenDolby app mark [`gpui_kit::Svg`].
pub fn app_mark() -> Svg {
    icon("opendolby")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_assets_resolve_independently_of_cwd() {
        let assets = application_assets();
        assert!(assets.load("icons/house.svg").unwrap().is_some());
        assert!(assets.load("opendolby.svg").unwrap().is_some());
        assert!(assets.load("icons/nonexistent-icon.svg").unwrap().is_none());
    }
}
