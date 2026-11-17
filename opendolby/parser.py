"""Parser for Dolby DAX3 Windows tuning XML files."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
import xml.etree.ElementTree as ET


DB_SCALE = 16.0


@dataclass
class PeqFilter:
    """Single parametric EQ or crossover filter from speaker-peq-filters."""

    speaker: int
    enabled: bool
    filter_type: int
    f0: float
    gain_db: float = 0.0
    q: float = 0.707
    order: int = 2


@dataclass
class MbCompressorBand:
    """Single band from tuning-vlldp mb-compressor-tuning."""

    band_index: int
    threshold_db: float
    ratio: float
    attack_ms: float
    release_ms: float
    makeup_db: float


@dataclass
class TuningCp:
    """Content processing (CP) stage parameters."""

    audio_optimizer_enable: bool = False
    audio_optimizer_bands: dict[int, list[float]] = field(default_factory=dict)
    process_optimizer_enable: bool = False
    process_optimizer_bands: dict[int, list[float]] = field(default_factory=dict)
    graphic_equalizer_enable: bool = False
    graphic_equalizer_bands: list[float] = field(default_factory=list)
    ieq_enable: bool = False
    ieq_amount: float = 0.0
    ieq_bands: list[float] = field(default_factory=list)
    pregain_db: float = 0.0
    postgain_db: float = 0.0
    system_gain_db: float = 0.0
    calibration_boost_db: float = 0.0
    dialog_enhancer_enable: bool = False
    dialog_enhancer_amount: float = 0.0
    surround_decoder_enable: bool = False
    volume_leveler_enable: bool = False


@dataclass
class TuningVlldp:
    """Speaker protection and acoustic correction (VLLDP) stage parameters."""

    audio_optimizer_enable: bool = False
    audio_optimizer_bands: dict[int, list[float]] = field(default_factory=dict)
    speaker_peq_enable: bool = False
    speaker_peq_filters: list[PeqFilter] = field(default_factory=list)
    mb_compressor_enable: bool = False
    mb_compressor_bands: list[MbCompressorBand] = field(default_factory=list)
    regulator_speaker_dist_enable: bool = False
    regulator_threshold_high: list[float] = field(default_factory=list)
    regulator_threshold_low: list[float] = field(default_factory=list)
    peak_level_db: float = 0.0
    postgain_db: float = 0.0
    system_gain_db: float = 0.0


@dataclass
class Dax3Profile:
    """A single tuning profile (e.g., dynamic, movie, music, game)."""

    name: str
    cp: TuningCp
    vlldp: TuningVlldp


@dataclass
class Dax3DeviceData:
    """Parsed DAX3 device tuning XML."""

    xml_path: Path
    xml_version: str
    tuning_date: str
    sample_rate: int
    endpoint_type: str
    channels: int
    band_freqs: list[float]
    constants: dict[str, list[float]]
    profiles: dict[str, Dax3Profile]


def _parse_csv_floats(raw: str, scale: float = 1.0) -> list[float]:
    if not raw.strip():
        return []
    return [float(x.strip()) / scale for x in raw.split(",") if x.strip()]


def _get_attr_float(parent: ET.Element, tag: str, default: float = 0.0, scale: float = 1.0) -> float:
    el = parent.find(tag)
    if el is None:
        return default
    val = el.attrib.get("value")
    if val is None:
        return default
    return float(val) / scale


def _get_attr_bool(parent: ET.Element, tag: str, default: bool = False) -> bool:
    el = parent.find(tag)
    if el is None:
        return default
    return el.attrib.get("value", "0") == "1"


def _resolve_band_array(
    el: ET.Element | None,
    constants: dict[str, list[float]],
    scale: float = DB_SCALE,
) -> list[float]:
    if el is None:
        return []
    if "preset" in el.attrib:
        return list(constants.get(el.attrib["preset"], []))
    if "value" in el.attrib:
        return _parse_csv_floats(el.attrib["value"], scale=scale)
    return []


def _parse_channel_bands(
    parent: ET.Element,
    tag: str,
    constants: dict[str, list[float]],
) -> dict[int, list[float]]:
    container = parent.find(tag)
    if container is None:
        return {}
    result: dict[int, list[float]] = {}
    for child in container:
        if child.tag.startswith("ch_"):
            try:
                ch_idx = int(child.tag.split("_", 1)[1])
            except ValueError:
                continue
            vals = _resolve_band_array(child, constants, scale=DB_SCALE)
            if vals:
                result[ch_idx] = vals
    return result


def _parse_cp(stage: ET.Element | None, constants: dict[str, list[float]]) -> TuningCp:
    if stage is None:
        return TuningCp()
    return TuningCp(
        audio_optimizer_enable=_get_attr_bool(stage, "audio-optimizer-enable"),
        audio_optimizer_bands=_parse_channel_bands(stage, "audio-optimizer-bands", constants),
        process_optimizer_enable=_get_attr_bool(stage, "process-optimizer-enable"),
        process_optimizer_bands=_parse_channel_bands(stage, "process-optimizer-bands", constants),
        graphic_equalizer_enable=_get_attr_bool(stage, "graphic-equalizer-enable"),
        graphic_equalizer_bands=_resolve_band_array(stage.find("graphic-equalizer-bands"), constants),
        ieq_enable=_get_attr_bool(stage, "ieq-enable"),
        ieq_amount=_get_attr_float(stage, "ieq-amount", default=0.0, scale=DB_SCALE),
        ieq_bands=_resolve_band_array(stage.find("ieq-bands-set"), constants),
        pregain_db=_get_attr_float(stage, "pregain", scale=DB_SCALE),
        postgain_db=_get_attr_float(stage, "postgain", scale=DB_SCALE),
        system_gain_db=_get_attr_float(stage, "system-gain", scale=DB_SCALE),
        calibration_boost_db=_get_attr_float(stage, "calibration-boost", scale=DB_SCALE),
        dialog_enhancer_enable=_get_attr_bool(stage, "dialog-enhancer-enable"),
        dialog_enhancer_amount=_get_attr_float(stage, "dialog-enhancer-amount", scale=DB_SCALE),
        surround_decoder_enable=_get_attr_bool(stage, "surround-decoder-enable"),
        volume_leveler_enable=_get_attr_bool(stage, "volume-leveler-enable"),
    )


def _parse_vlldp(stage: ET.Element | None, constants: dict[str, list[float]]) -> TuningVlldp:
    if stage is None:
        return TuningVlldp()

    peq_filters: list[PeqFilter] = []
    peq_container = stage.find("speaker-peq-filters")
    if peq_container is not None:
        for f in peq_container.findall("filter"):
            peq_filters.append(
                PeqFilter(
                    speaker=int(f.attrib.get("speaker", "0")),
                    enabled=f.attrib.get("enabled", "0") == "1",
                    filter_type=int(f.attrib.get("type", "1")),
                    f0=float(f.attrib.get("f0", "1000")),
                    gain_db=float(f.attrib.get("gain", "0")),
                    q=float(f.attrib.get("q", "0.707")),
                    order=int(f.attrib.get("order", "2")),
                )
            )

    mb_bands: list[MbCompressorBand] = []
    mb_container = stage.find("mb-compressor-tuning")
    if mb_container is not None:
        for child in sorted(mb_container, key=lambda c: c.tag):
            if child.tag.startswith("band_group_"):
                vals = _parse_csv_floats(child.attrib.get("value", ""))
                if len(vals) >= 6:
                    mb_bands.append(
                        MbCompressorBand(
                            band_index=int(vals[0]),
                            threshold_db=vals[1] / DB_SCALE,
                            ratio=vals[2] / 1024.0,
                            attack_ms=vals[3],
                            release_ms=vals[4],
                            makeup_db=vals[5] / DB_SCALE,
                        )
                    )

    return TuningVlldp(
        audio_optimizer_enable=_get_attr_bool(stage, "audio-optimizer-enable"),
        audio_optimizer_bands=_parse_channel_bands(stage, "audio-optimizer-bands", constants),
        speaker_peq_enable=_get_attr_bool(stage, "speaker-peq-enable"),
        speaker_peq_filters=peq_filters,
        mb_compressor_enable=_get_attr_bool(stage, "mb-compressor-enable"),
        mb_compressor_bands=mb_bands,
        regulator_speaker_dist_enable=_get_attr_bool(stage, "regulator-speaker-dist-enable"),
        regulator_threshold_high=_resolve_band_array(
            stage.find("regulator-tuning/threshold_high"), constants
        ),
        regulator_threshold_low=_resolve_band_array(
            stage.find("regulator-tuning/threshold_low"), constants
        ),
        peak_level_db=_get_attr_float(stage, "peak-level", scale=DB_SCALE),
        postgain_db=_get_attr_float(stage, "postgain", scale=DB_SCALE),
        system_gain_db=_get_attr_float(stage, "system-gain", scale=DB_SCALE),
    )


def parse_dax3_xml(xml_path: str | Path) -> Dax3DeviceData:
    """Parse a Dolby DAX3 tuning XML file into structured device and profile data."""
    path = Path(xml_path)
    root = ET.parse(path).getroot()

    xml_ver_el = root.find("xml_version")
    tuning_date_el = root.find("tuning_date")
    xml_version = xml_ver_el.attrib.get("value", "unknown") if xml_ver_el is not None else "unknown"
    tuning_date = tuning_date_el.attrib.get("value", "unknown") if tuning_date_el is not None else "unknown"

    endpoint = root.find("endpoint")
    if endpoint is None:
        raise ValueError(f"No <endpoint> element found in {path}")

    sample_rate = int(endpoint.attrib.get("fs", "48000"))
    endpoint_type = endpoint.attrib.get("type", "internal_speaker")
    channels = int(endpoint.attrib.get("total_count", "2"))

    constants: dict[str, list[float]] = {}
    band_freqs: list[float] = []
    const_el = root.find("constant")
    if const_el is not None:
        for child in const_el:
            if child.tag == "band_20_freq":
                key = f"fs_{sample_rate}"
                raw = child.attrib.get(key) or next(iter(child.attrib.values()), "")
                band_freqs = _parse_csv_floats(raw, scale=1.0)
            elif "target" in child.attrib:
                constants[child.tag] = _parse_csv_floats(child.attrib["target"], scale=DB_SCALE)

    profiles: dict[str, Dax3Profile] = {}
    for prof_el in endpoint.findall("profile"):
        name = prof_el.attrib.get("type")
        if not name:
            continue
        cp = _parse_cp(prof_el.find("tuning-cp"), constants)
        vlldp = _parse_vlldp(prof_el.find("tuning-vlldp"), constants)
        profiles[name] = Dax3Profile(name=name, cp=cp, vlldp=vlldp)

    return Dax3DeviceData(
        xml_path=path,
        xml_version=xml_version,
        tuning_date=tuning_date,
        sample_rate=sample_rate,
        endpoint_type=endpoint_type,
        channels=channels,
        band_freqs=band_freqs,
        constants=constants,
        profiles=profiles,
    )
