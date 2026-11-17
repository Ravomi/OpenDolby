use dolby_rs::audio_devices::OutputDevice;
use std::fs;
use std::path::Path;

const DMI_VENDOR_PATH: &str = "/sys/class/dmi/id/sys_vendor";
const DMI_PRODUCT_PATH: &str = "/sys/class/dmi/id/product_name";

fn read_dmi_field(path: &str) -> Option<String> {
    fs::read_to_string(Path::new(path))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub(crate) fn machine_label() -> String {
    let vendor = read_dmi_field(DMI_VENDOR_PATH);
    let product = read_dmi_field(DMI_PRODUCT_PATH);

    match (vendor, product) {
        (Some(v), Some(p)) => format!("{v} {p}"),
        (Some(v), None) => v,
        (None, Some(p)) => p,
        (None, None) => "未知设备".to_string(),
    }
}

fn is_safe_internal_speaker(dev: &OutputDevice) -> bool {
    dev.available && dev.is_internal_speaker && !dev.is_virtual
}

pub(crate) fn resolve_target(
    outputs: &[OutputDevice],
    current: Option<&str>,
    default: Option<&str>,
    automatic: bool,
) -> Option<String> {
    if !automatic {
        return current.map(ToString::to_string);
    }

    if let Some(curr) = current {
        if outputs
            .iter()
            .any(|d| d.name == curr && is_safe_internal_speaker(d))
        {
            return Some(curr.to_string());
        }
    }

    if let Some(def) = default {
        if outputs
            .iter()
            .any(|d| d.name == def && is_safe_internal_speaker(d))
        {
            return Some(def.to_string());
        }
    }

    outputs
        .iter()
        .find(|d| is_safe_internal_speaker(d))
        .map(|d| d.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dolby_rs::audio_devices::DeviceKind;

    fn make_dev(
        name: &str,
        kind: DeviceKind,
        is_internal_speaker: bool,
        is_virtual: bool,
        available: bool,
    ) -> OutputDevice {
        OutputDevice {
            id: 1,
            name: name.to_string(),
            description: name.to_string(),
            is_virtual,
            is_internal_speaker,
            available,
            kind,
        }
    }

    #[test]
    fn auto_starts_empty_then_picks_internal_speaker_when_connected() {
        let initial: Vec<OutputDevice> = vec![];
        assert_eq!(resolve_target(&initial, None, None, true), None);

        let connected = vec![make_dev(
            "alsa_output.pci.speaker",
            DeviceKind::InternalSpeaker,
            true,
            false,
            true,
        )];
        assert_eq!(
            resolve_target(&connected, None, None, true),
            Some("alsa_output.pci.speaker".to_string())
        );
    }

    #[test]
    fn auto_switches_to_safe_speaker_when_current_target_is_lost() {
        let outputs = vec![
            make_dev(
                "alsa_output.pci.speaker_old",
                DeviceKind::InternalSpeaker,
                true,
                false,
                false,
            ),
            make_dev(
                "alsa_output.pci.speaker_new",
                DeviceKind::InternalSpeaker,
                true,
                false,
                true,
            ),
        ];

        assert_eq!(
            resolve_target(&outputs, Some("alsa_output.pci.speaker_old"), None, true),
            Some("alsa_output.pci.speaker_new".to_string())
        );
    }

    #[test]
    fn auto_never_selects_external_virtual_or_unavailable_devices() {
        let outputs = vec![
            make_dev("headphones", DeviceKind::Headphones, false, false, true),
            make_dev("usb_dac", DeviceKind::Usb, false, false, true),
            make_dev("bt_headset", DeviceKind::Bluetooth, false, false, true),
            make_dev("hdmi_out", DeviceKind::Hdmi, false, false, true),
            make_dev("opendolby_virtual", DeviceKind::Virtual, false, true, true),
            make_dev(
                "unavailable_speaker",
                DeviceKind::InternalSpeaker,
                true,
                false,
                false,
            ),
        ];

        assert_eq!(resolve_target(&outputs, None, Some("headphones"), true), None);
        assert_eq!(resolve_target(&outputs, Some("usb_dac"), None, true), None);
    }

    #[test]
    fn manual_mode_preserves_current_target_without_auto_switching() {
        let outputs = vec![
            make_dev(
                "alsa_output.pci.speaker_a",
                DeviceKind::InternalSpeaker,
                true,
                false,
                true,
            ),
            make_dev(
                "alsa_output.pci.speaker_b",
                DeviceKind::InternalSpeaker,
                true,
                false,
                false,
            ),
        ];

        assert_eq!(
            resolve_target(&outputs, Some("alsa_output.pci.speaker_b"), None, false),
            Some("alsa_output.pci.speaker_b".to_string())
        );
        assert_eq!(
            resolve_target(&outputs, None, Some("alsa_output.pci.speaker_a"), false),
            None
        );
    }
}
