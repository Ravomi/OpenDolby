//! User-owned UI preferences. Never stores or copies OEM tuning contents.
use super::palette::ThemePreference;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Preferences {
    pub theme: ThemePreference,
    pub auto_target: bool,
    pub manual_target: Option<String>,
    pub xml_path: Option<PathBuf>,
    pub profile: String,
    pub level_match: bool,
    pub surround_amount: f32,
    pub dialog_amount: f32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            auto_target: true,
            manual_target: None,
            xml_path: None,
            profile: "dynamic".to_string(),
            level_match: true,
            surround_amount: 1.0,
            dialog_amount: 1.0,
        }
    }
}

pub(super) fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("OPENDOLBY_UI_CONFIG").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".config")
        });
    root.join("opendolby/ui.json")
}

impl Preferences {
    pub fn load() -> Result<Self> {
        Self::load_from(&config_path())
    }
    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    fn from_json(text: &str) -> Result<Self> {
        let data: Value = serde_json::from_str(text).context("设置文件不是有效 JSON")?;
        if !data.is_object() {
            bail!("设置文件应为 JSON 对象");
        }
        match data.get("version") {
            None => {}
            Some(value) if value.as_u64() == Some(1) => {}
            _ => bail!("不支持的设置文件版本"),
        }
        let theme = match data.get("theme") {
            Some(Value::String(value)) => ThemePreference::parse(value)
                .ok_or_else(|| anyhow::anyhow!("未知外观选项: {value}"))?,
            None => ThemePreference::System,
            _ => bail!("外观选项应为字符串"),
        };
        let auto_target = match data.get("auto_target") {
            Some(Value::Bool(value)) => *value,
            None => true,
            _ => bail!("自动选择设备选项应为布尔值"),
        };
        let optional_string = |key: &str| -> Result<Option<String>> {
            match data.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(value)) if value.is_empty() => Ok(None),
                Some(Value::String(value)) if !value.chars().any(char::is_control) => {
                    Ok(Some(value.clone()))
                }
                _ => bail!("{key} 应为不含控制字符的路径或名称"),
            }
        };
        let profile = optional_string("profile")?.unwrap_or_else(|| "dynamic".to_string());
        let level_match = match data.get("level_match") {
            Some(Value::Bool(value)) => *value,
            None => true,
            _ => bail!("电平匹配选项应为布尔值"),
        };
        let parse_slider = |key: &str| -> Result<f32> {
            match data.get(key) {
                None => Ok(1.0),
                Some(Value::Number(num)) => {
                    let v = num
                        .as_f64()
                        .ok_or_else(|| anyhow::anyhow!("{key} 数值超出范围"))?
                        as f32;
                    if !v.is_finite() || !(0.0..=2.0).contains(&v) {
                        bail!("{key} 应在 0.0 到 2.0 之间");
                    }
                    Ok((v * 100.0).round() / 100.0)
                }
                _ => bail!("{key} 应为 0.0 到 2.0 之间的数字"),
            }
        };
        Ok(Self {
            theme,
            auto_target,
            manual_target: optional_string("manual_target")?,
            xml_path: optional_string("xml_path")?.map(PathBuf::from),
            profile,
            level_match,
            surround_amount: parse_slider("surround_amount")?,
            dialog_amount: parse_slider("dialog_amount")?,
        })
    }

    fn load_from(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                Self::from_json(&text).with_context(|| format!("无法读取设置 {}", path.display()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("无法读取设置 {}", path.display())),
        }
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|value| !value.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).context("无法创建设置目录")?;
        let data = json!({
            "version": 1,
            "theme": self.theme.as_str(),
            "auto_target": self.auto_target,
            "manual_target": self.manual_target,
            "xml_path": self.xml_path,
            "profile": self.profile,
            "level_match": self.level_match,
            "surround_amount": (self.surround_amount * 100.0).round() / 100.0,
            "dialog_amount": (self.dialog_amount * 100.0).round() / 100.0,
        });
        let temp = parent.join(format!(
            ".opendolby-ui-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let result = (|| -> Result<()> {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(serde_json::to_string_pretty(&data)?.as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.with_context(|| format!("无法保存设置 {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_defaults_are_system_theme_and_safe_automatic_target() {
        assert_eq!(
            Preferences::from_json("{}").unwrap(),
            Preferences::default()
        );
        assert!(Preferences::from_json("[]").is_err());
        assert!(Preferences::from_json(r#"{"theme":"unknown"}"#).is_err());
        assert!(Preferences::from_json(r#"{"version":9}"#).is_err());
        assert!(Preferences::from_json(r#"{"version":"1"}"#).is_err());
        assert!(Preferences::from_json(r#"{"auto_target":"true"}"#).is_err());
        assert!(Preferences::from_json(r#"{"manual_target":"bad\nname"}"#).is_err());
        assert!(Preferences::from_json(r#"{"level_match":"yes"}"#).is_err());
        assert!(Preferences::from_json(r#"{"surround_amount":2.5}"#).is_err());
        assert!(Preferences::from_json(r#"{"dialog_amount":-0.1}"#).is_err());
    }
    #[test]
    fn preferences_round_trip_without_storing_tuning_contents() {
        let root = std::env::temp_dir().join(format!(
            "opendolby-prefs-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("nested/ui.json");
        let prefs = Preferences {
            theme: ThemePreference::Dark,
            auto_target: false,
            manual_target: Some("alsa_output.speaker".into()),
            xml_path: Some(PathBuf::from("/local/OEM tuning.xml")),
            profile: "movie".into(),
            level_match: false,
            surround_amount: 1.35,
            dialog_amount: 0.85,
        };
        assert_eq!(
            Preferences::load_from(&path).unwrap(),
            Preferences::default()
        );
        prefs.save_to(&path).unwrap();
        assert_eq!(Preferences::load_from(&path).unwrap(), prefs);
        let contents = fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("<"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }
}
