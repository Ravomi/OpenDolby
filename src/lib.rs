use std::path::PathBuf;

pub mod dsp;
pub mod pw_sink;
pub mod tuning;

/// Returns the default tuning XML path:
/// 1. `OPENDOLBY_XML` environment variable (if set and non-empty)
/// 2. `$XDG_CONFIG_HOME/opendolby/tuning.xml` (if `XDG_CONFIG_HOME` is set and non-empty)
/// 3. `$HOME/.config/opendolby/tuning.xml`
pub fn default_xml_path() -> PathBuf {
    if let Ok(path) = std::env::var("OPENDOLBY_XML") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("opendolby/tuning.xml");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".config/opendolby/tuning.xml");
        }
    }
    PathBuf::from(".config/opendolby/tuning.xml")
}
