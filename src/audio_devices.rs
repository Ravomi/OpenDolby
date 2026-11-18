use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::process::Command;

pub const VIRTUAL_SINK_NAME: &str = "opendolby_speaker";
pub const VIRTUAL_OUT_NAME: &str = "opendolby_speaker_out";
const LEGACY_VIRTUAL_SINK_NAME: &str = "dolby_atmos_rust";
const LEGACY_VIRTUAL_OUT_NAME: &str = "dolby_atmos_rust_out";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    InternalSpeaker,
    Headphones,
    Hdmi,
    Bluetooth,
    Usb,
    Virtual,
    Unknown,
}

impl DeviceKind {
    pub fn label(&self) -> &'static str {
        match self {
            DeviceKind::InternalSpeaker => "Internal Speaker",
            DeviceKind::Headphones => "Headphones",
            DeviceKind::Hdmi => "HDMI / DisplayPort",
            DeviceKind::Bluetooth => "Bluetooth",
            DeviceKind::Usb => "USB Audio",
            DeviceKind::Virtual => "Virtual",
            DeviceKind::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDevice {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub sample_rate: u32,
    pub is_virtual: bool,
    pub is_internal_speaker: bool,
    pub available: bool,
    pub kind: DeviceKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInventory {
    pub outputs: Vec<OutputDevice>,
    pub default_sink: Option<String>,
}

/// Discover audio output sinks via `pactl --format=json list sinks` and `pactl get-default-sink`.
pub fn discover_outputs() -> Result<DeviceInventory> {
    let sinks_out = Command::new("pactl")
        .args(["--format=json", "list", "sinks"])
        .output()
        .context("Failed to execute `pactl --format=json list sinks`")?;

    if !sinks_out.status.success() {
        let stderr = String::from_utf8_lossy(&sinks_out.stderr);
        return Err(anyhow!(
            "`pactl --format=json list sinks` failed ({}): {}",
            sinks_out.status,
            stderr.trim()
        ));
    }

    let json_text = String::from_utf8_lossy(&sinks_out.stdout);
    let outputs = parse_sinks_json(&json_text)?;

    let default_sink = Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    Ok(DeviceInventory {
        outputs,
        default_sink,
    })
}

/// Set the default PipeWire/PulseAudio output sink using `pactl set-default-sink`.
pub fn set_default_output(name: &str) -> Result<()> {
    validate_sink_name(name)?;

    let out = Command::new("pactl")
        .args(["set-default-sink", name])
        .output()
        .with_context(|| format!("Failed to execute `pactl set-default-sink {}`", name))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!(
            "`pactl set-default-sink {}` failed ({}): {}",
            name,
            out.status,
            stderr.trim()
        ));
    }

    Ok(())
}

/// Choose an eligible internal speaker from `outputs`, preferring `preferred` if it is an
/// available, non-virtual internal speaker. Never auto-selects USB, Bluetooth, HDMI, headphones,
/// virtual, or unavailable sinks.
pub fn choose_internal_speaker<'a>(
    outputs: &'a [OutputDevice],
    preferred: Option<&str>,
) -> Option<&'a OutputDevice> {
    let is_eligible = |d: &&OutputDevice| {
        !d.is_virtual
            && d.is_internal_speaker
            && d.available
            && d.kind == DeviceKind::InternalSpeaker
    };

    if let Some(pref) = preferred {
        if let Some(found) = outputs.iter().filter(is_eligible).find(|d| d.name == pref) {
            return Some(found);
        }
    }

    outputs.iter().find(is_eligible)
}

fn validate_sink_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(anyhow!("Sink name cannot be empty"));
    }
    if name.starts_with('-') {
        return Err(anyhow!(
            "Invalid sink name '{}': cannot start with '-'",
            name
        ));
    }
    if name.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(anyhow!(
            "Invalid sink name '{}': contains control characters",
            name
        ));
    }
    Ok(())
}

/// Parse `pactl --format=json list sinks` output into `Vec<OutputDevice>`.
pub fn parse_sinks_json(json_str: &str) -> Result<Vec<OutputDevice>> {
    let root: Value =
        serde_json::from_str(json_str).context("Failed to parse `pactl` sinks JSON")?;
    let arr = root
        .as_array()
        .ok_or_else(|| anyhow!("Expected JSON array from `pactl --format=json list sinks`"))?;

    let mut devices = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(dev) = parse_single_sink(item) {
            devices.push(dev);
        }
    }
    Ok(devices)
}

fn parse_single_sink(item: &Value) -> Option<OutputDevice> {
    let raw_id = item.get("index")?.as_u64()?;
    let id = u32::try_from(raw_id).ok()?;
    let name = item.get("name")?.as_str()?.to_string();
    let description = item
        .get("description")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&name)
        .to_string();

    let props = item.get("properties").and_then(|v| v.as_object());
    let prop_str =
        |key: &str| -> Option<&str> { props.and_then(|m| m.get(key)).and_then(|v| v.as_str()) };

    let active_port_name = item.get("active_port").and_then(|v| v.as_str());
    let ports = item
        .get("ports")
        .and_then(|v| v.as_array())
        .map(|v| v.as_slice())
        .unwrap_or(&[]);

    let active_port_obj = active_port_name.and_then(|ap| {
        ports
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(ap))
    });

    let available = compute_availability(active_port_obj, ports);
    let sample_rate = parse_sample_rate(item);
    let is_virtual = detect_virtual(&name, prop_str);
    let kind = classify_device_kind(
        &name,
        is_virtual,
        prop_str,
        active_port_name,
        active_port_obj,
    );
    let is_internal_speaker = !is_virtual && kind == DeviceKind::InternalSpeaker;

    Some(OutputDevice {
        id,
        name,
        description,
        sample_rate,
        is_virtual,
        is_internal_speaker,
        available,
        kind,
    })
}

fn parse_sample_rate(item: &Value) -> u32 {
    item.get("sample_specification")
        .and_then(|v| v.as_str())
        .and_then(|spec| {
            spec.split_whitespace().find_map(|token| {
                token
                    .strip_suffix("Hz")
                    .or_else(|| token.strip_suffix("hz"))
                    .and_then(|num| num.parse::<u32>().ok())
            })
        })
        .filter(|&r| (8_000..=384_000).contains(&r))
        .unwrap_or(48_000)
}

fn compute_availability(active_port_obj: Option<&Value>, ports: &[Value]) -> bool {
    let is_port_not_available = |p: &Value| -> bool {
        p.get("availability")
            .and_then(|v| v.as_str())
            .map(|s| s.eq_ignore_ascii_case("not available"))
            .unwrap_or(false)
    };

    if let Some(ap) = active_port_obj {
        return !is_port_not_available(ap);
    }
    if !ports.is_empty() {
        return ports.iter().any(|p| !is_port_not_available(p));
    }
    true
}

fn detect_virtual<'a>(name: &str, prop_str: impl Fn(&str) -> Option<&'a str>) -> bool {
    if matches!(
        name,
        VIRTUAL_SINK_NAME | VIRTUAL_OUT_NAME | LEGACY_VIRTUAL_SINK_NAME | LEGACY_VIRTUAL_OUT_NAME
    ) {
        return true;
    }
    if prop_str("node.virtual") == Some("true") {
        return true;
    }
    if let Some(factory) = prop_str("factory.name") {
        let f = factory.to_ascii_lowercase();
        if f.contains("null-audio-sink") || f.contains("virtual") || f.contains("loopback") {
            return true;
        }
    }
    false
}

fn classify_device_kind<'a>(
    name: &str,
    is_virtual: bool,
    prop_str: impl Fn(&str) -> Option<&'a str>,
    active_port_name: Option<&str>,
    active_port_obj: Option<&Value>,
) -> DeviceKind {
    if is_virtual {
        return DeviceKind::Virtual;
    }

    let name_lower = name.to_ascii_lowercase();
    let device_api = prop_str("device.api").unwrap_or("");
    let device_bus = prop_str("device.bus").unwrap_or("");
    let icon_name = prop_str("device.icon_name").unwrap_or("");

    // 1. Bluetooth
    if device_api.eq_ignore_ascii_case("bluez5")
        || device_bus.eq_ignore_ascii_case("bluetooth")
        || name_lower.starts_with("bluez_output.")
    {
        return DeviceKind::Bluetooth;
    }

    // 2. USB
    if device_bus.eq_ignore_ascii_case("usb") || name_lower.contains(".usb-") {
        return DeviceKind::Usb;
    }

    // 3. Active port takes priority over sink name / port list.
    // If active_port is present, never fall back to icon_name or sink name.
    if let Some(ap_name) = active_port_name {
        let ap_lower = ap_name.to_ascii_lowercase();
        let ap_type = active_port_obj
            .and_then(|p| p.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if ap_type == "headphones"
            || ap_type == "headset"
            || ap_lower.contains("headphone")
            || ap_lower.contains("headset")
        {
            return DeviceKind::Headphones;
        }

        if ap_type == "hdmi"
            || ap_lower.contains("hdmi")
            || ap_lower.contains("displayport")
            || ap_lower.starts_with("[out] dp")
        {
            return DeviceKind::Hdmi;
        }

        let is_local_bus = device_bus.eq_ignore_ascii_case("pci")
            || device_bus.eq_ignore_ascii_case("platform")
            || name_lower.starts_with("alsa_output.pci-")
            || name_lower.starts_with("alsa_output.platform-");

        if is_local_bus && (ap_type == "speaker" || ap_lower.contains("speaker")) {
            return DeviceKind::InternalSpeaker;
        }

        return DeviceKind::Unknown;
    }

    // 4. Fallback when active_port is absent (never infer InternalSpeaker from icon_name alone)
    if name_lower.contains("hdmi") || icon_name.eq_ignore_ascii_case("video-display") {
        return DeviceKind::Hdmi;
    }
    if name_lower.contains("headphone") || name_lower.contains("headset") {
        return DeviceKind::Headphones;
    }
    let is_local_bus = device_bus.eq_ignore_ascii_case("pci")
        || device_bus.eq_ignore_ascii_case("platform")
        || name_lower.starts_with("alsa_output.pci-")
        || name_lower.starts_with("alsa_output.platform-");
    if is_local_bus && name_lower.contains("speaker__sink") {
        return DeviceKind::InternalSpeaker;
    }

    DeviceKind::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ucm_internal_speaker_and_hdmi_and_virtual() {
        let json = r#"[
            {
                "index": 70,
                "name": "alsa_output.pci-0000_00_1f.3-platform-skl_hda_dsp_generic.HiFi__HDMI3__sink",
                "description": "HDMI / DisplayPort 3 Output",
                "properties": { "device.bus": "pci", "device.icon_name": "video-display" },
                "ports": [{ "name": "[Out] HDMI3", "type": "HDMI", "availability": "not available" }],
                "active_port": "[Out] HDMI3"
            },
            {
                "index": 73,
                "name": "alsa_output.pci-0000_03_00.6.HiFi__Speaker__sink",
                "description": "Generic Laptop Speaker",
                "sample_specification": "s32le 2ch 44100Hz",
                "properties": { "device.bus": "pci", "device.icon_name": "audio-speakers" },
                "ports": [{ "name": "[Out] Speaker", "type": "Speaker", "availability": "availability unknown" }],
                "active_port": "[Out] Speaker"
            },
            {
                "index": 2884,
                "name": "opendolby_speaker",
                "description": "OpenDolby Speaker",
                "sample_specification": "float32le 2ch 48000Hz",
                "properties": { "node.virtual": "true" },
                "ports": [],
                "active_port": null
            }
        ]"#;

        let devices = parse_sinks_json(json).unwrap();
        assert_eq!(devices.len(), 3);

        assert_eq!(devices[0].kind, DeviceKind::Hdmi);
        assert!(!devices[0].is_internal_speaker);
        assert!(!devices[0].available);
        assert_eq!(devices[0].sample_rate, 48_000);

        assert_eq!(devices[1].kind, DeviceKind::InternalSpeaker);
        assert!(devices[1].is_internal_speaker);
        assert!(devices[1].available);
        assert!(!devices[1].is_virtual);
        assert_eq!(devices[1].sample_rate, 44_100);

        assert_eq!(devices[2].kind, DeviceKind::Virtual);
        assert!(devices[2].is_virtual);
        assert!(!devices[2].is_internal_speaker);
        assert_eq!(devices[2].sample_rate, 48_000);

        let chosen = choose_internal_speaker(&devices, None).unwrap();
        assert_eq!(chosen.id, 73);
    }

    #[test]
    fn parses_analog_stereo_speaker_and_active_headphones_override() {
        let json = r#"[
            {
                "index": 10,
                "name": "alsa_output.pci-0000_00_1f.3.analog-stereo",
                "description": "Built-in Audio Analog Stereo",
                "properties": { "device.bus": "pci" },
                "ports": [
                    { "name": "analog-output-speaker", "type": "Speaker", "availability": "available" },
                    { "name": "analog-output-headphones", "type": "Headphones", "availability": "not available" }
                ],
                "active_port": "analog-output-speaker"
            },
            {
                "index": 11,
                "name": "alsa_output.pci-0000_00_1f.3.HiFi__Speaker__sink",
                "description": "Built-in Audio",
                "properties": { "device.bus": "pci" },
                "ports": [
                    { "name": "[Out] Speaker", "type": "Speaker", "availability": "not available" },
                    { "name": "[Out] Headphones", "type": "Headphones", "availability": "available" }
                ],
                "active_port": "[Out] Headphones"
            }
        ]"#;

        let devices = parse_sinks_json(json).unwrap();
        assert_eq!(devices[0].kind, DeviceKind::InternalSpeaker);
        assert!(devices[0].is_internal_speaker);

        // Even though sink name contains Speaker__sink and ports has Speaker,
        // active_port is Headphones so it must NOT be treated as internal speaker.
        assert_eq!(devices[1].kind, DeviceKind::Headphones);
        assert!(!devices[1].is_internal_speaker);

        let chosen = choose_internal_speaker(&devices, Some(&devices[1].name)).unwrap();
        assert_eq!(chosen.id, 10);
    }

    #[test]
    fn never_auto_selects_usb_bluetooth_hdmi_or_unavailable() {
        let json = r#"[
            {
                "index": 1,
                "name": "alsa_output.usb-Generic_USB_Audio-00.analog-stereo",
                "description": "USB Speaker",
                "properties": { "device.bus": "usb" },
                "ports": [{ "name": "analog-output-speaker", "type": "Speaker", "availability": "available" }],
                "active_port": "analog-output-speaker"
            },
            {
                "index": 2,
                "name": "bluez_output.AA_BB_CC_DD_EE_FF.1",
                "description": "Bluetooth Headphones",
                "properties": { "device.api": "bluez5", "device.bus": "bluetooth" },
                "ports": [{ "name": "headphone-output", "type": "Headphones", "availability": "available" }],
                "active_port": "headphone-output"
            },
            {
                "index": 3,
                "name": "alsa_output.pci-0000_00_1f.3.hdmi-stereo",
                "description": "HDMI Output",
                "properties": { "device.bus": "pci" },
                "ports": [{ "name": "hdmi-output-0", "type": "HDMI", "availability": "available" }],
                "active_port": "hdmi-output-0"
            },
            {
                "index": 4,
                "name": "alsa_output.pci-0000_00_1f.3.HiFi__Speaker__sink",
                "description": "Internal Speaker (Unavailable)",
                "properties": { "device.bus": "pci" },
                "ports": [{ "name": "[Out] Speaker", "type": "Speaker", "availability": "not available" }],
                "active_port": "[Out] Speaker"
            }
        ]"#;

        let devices = parse_sinks_json(json).unwrap();
        assert_eq!(devices[0].kind, DeviceKind::Usb);
        assert_eq!(devices[1].kind, DeviceKind::Bluetooth);
        assert_eq!(devices[2].kind, DeviceKind::Hdmi);
        assert_eq!(devices[3].kind, DeviceKind::InternalSpeaker);
        assert!(!devices[3].available);

        assert!(choose_internal_speaker(&devices, None).is_none());
    }

    #[test]
    fn validate_sink_name_rejects_injection_attempts() {
        assert!(validate_sink_name("").is_err());
        assert!(validate_sink_name("   ").is_err());
        assert!(validate_sink_name("--help").is_err());
        assert!(validate_sink_name("sink\0name").is_err());
        assert!(validate_sink_name("alsa_output.pci-0000_00_1f.3.analog-stereo").is_ok());
    }

    #[test]
    fn lineout_with_speaker_icon_and_network_bus_are_not_internal_speaker() {
        let json = r#"[
            {
                "index": 20,
                "name": "alsa_output.pci-0000_00_1f.3.analog-stereo",
                "description": "Line Out",
                "properties": { "device.bus": "pci", "device.icon_name": "audio-speakers" },
                "ports": [{ "name": "analog-output-lineout", "type": "Line", "availability": "available" }],
                "active_port": "analog-output-lineout"
            },
            {
                "index": 21,
                "name": "raop_sink.AirPlay.local",
                "description": "Living Room Speaker",
                "properties": { "device.bus": "network", "device.icon_name": "audio-speakers" },
                "ports": [{ "name": "speaker", "type": "Speaker", "availability": "available" }],
                "active_port": "speaker"
            }
        ]"#;

        let devices = parse_sinks_json(json).unwrap();
        assert_eq!(devices[0].kind, DeviceKind::Unknown);
        assert!(!devices[0].is_internal_speaker);
        assert_eq!(devices[1].kind, DeviceKind::Unknown);
        assert!(!devices[1].is_internal_speaker);
        assert!(choose_internal_speaker(&devices, None).is_none());
    }

    #[test]
    fn handles_malformed_json_and_index_overflow() {
        assert!(parse_sinks_json("not json").is_err());
        assert!(parse_sinks_json("{}").is_err());
        let overflow = r#"[{"index": 99999999999999, "name": "bad_sink"}]"#;
        let devices = parse_sinks_json(overflow).unwrap();
        assert!(devices.is_empty());
    }
}
