"""Minimum-phase FIR impulse response synthesis from DAX3 tuning parameters."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import numpy as np
from scipy import signal
from scipy.io import wavfile

from opendolby.parser import Dax3DeviceData, Dax3Profile, PeqFilter


@dataclass
class DspSummary:
    """Summary of synthesized impulse response."""

    profile: str
    sample_rate: int
    num_taps: int
    channels: int
    peak_gain_db: float
    headroom_trim_db: float
    irs_path: Path


def _interpolate_bands_db(
    band_freqs: list[float],
    band_db: list[float],
    freqs: np.ndarray,
) -> np.ndarray:
    """Log-frequency linear interpolation of 20-band dB gains onto FFT bins."""
    if not band_freqs or not band_db:
        return np.zeros_like(freqs, dtype=np.float64)
    n = min(len(band_freqs), len(band_db))
    log_bf = np.log10(np.maximum(np.asarray(band_freqs[:n], dtype=np.float64), 1.0))
    log_f = np.log10(np.maximum(freqs, 1.0))
    return np.interp(log_f, log_bf, np.asarray(band_db[:n], dtype=np.float64))


def _biquad_mag_db(peq: PeqFilter, freqs: np.ndarray, fs: int) -> np.ndarray:
    """Evaluate magnitude response (in dB) of a DAX3 speaker-peq filter."""
    if not peq.enabled or peq.f0 <= 0 or peq.f0 >= fs * 0.499:
        return np.zeros_like(freqs, dtype=np.float64)

    w = 2.0 * np.pi * freqs / fs
    if peq.filter_type in (8, 9):
        btype = "low" if peq.filter_type == 8 else "high"
        sos = signal.butter(max(1, peq.order), peq.f0, btype=btype, fs=fs, output="sos")
        _, h = signal.sosfreqz(sos, worN=w)
        return 20.0 * np.log10(np.maximum(np.abs(h), 1e-9))

    w0 = 2.0 * np.pi * peq.f0 / fs
    cos_w0 = np.cos(w0)
    sin_w0 = np.sin(w0)
    q = max(peq.q, 0.05)
    alpha = sin_w0 / (2.0 * q)
    a = 10.0 ** (peq.gain_db / 40.0)

    if peq.filter_type == 1:
        b = np.array([1.0 + alpha * a, -2.0 * cos_w0, 1.0 - alpha * a])
        a_coef = np.array([1.0 + alpha / a, -2.0 * cos_w0, 1.0 - alpha / a])
    elif peq.filter_type == 2:
        sq = 2.0 * np.sqrt(a) * alpha
        b = a * np.array([
            (a + 1.0) - (a - 1.0) * cos_w0 + sq,
            2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
            (a + 1.0) - (a - 1.0) * cos_w0 - sq,
        ])
        a_coef = np.array([
            (a + 1.0) + (a - 1.0) * cos_w0 + sq,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
            (a + 1.0) + (a - 1.0) * cos_w0 - sq,
        ])
    elif peq.filter_type == 3:
        sq = 2.0 * np.sqrt(a) * alpha
        b = a * np.array([
            (a + 1.0) + (a - 1.0) * cos_w0 + sq,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
            (a + 1.0) + (a - 1.0) * cos_w0 - sq,
        ])
        a_coef = np.array([
            (a + 1.0) - (a - 1.0) * cos_w0 + sq,
            2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
            (a + 1.0) - (a - 1.0) * cos_w0 - sq,
        ])
    else:
        return np.zeros_like(freqs, dtype=np.float64)

    _, h = signal.freqz(b, a_coef, worN=w)
    return 20.0 * np.log10(np.maximum(np.abs(h), 1e-9))


def _channel_response_db(
    device: Dax3DeviceData,
    profile: Dax3Profile,
    ch_idx: int,
    freqs: np.ndarray,
) -> np.ndarray:
    """Combine CP and VLLDP frequency-domain curves for one channel."""
    total_db = np.zeros_like(freqs, dtype=np.float64)
    cp = profile.cp
    vlldp = profile.vlldp

    if cp.audio_optimizer_enable and ch_idx in cp.audio_optimizer_bands:
        total_db += _interpolate_bands_db(device.band_freqs, cp.audio_optimizer_bands[ch_idx], freqs)
    if cp.process_optimizer_enable and ch_idx in cp.process_optimizer_bands:
        total_db += _interpolate_bands_db(device.band_freqs, cp.process_optimizer_bands[ch_idx], freqs)
    if cp.graphic_equalizer_enable and cp.graphic_equalizer_bands:
        total_db += _interpolate_bands_db(device.band_freqs, cp.graphic_equalizer_bands, freqs)
    if cp.ieq_enable and cp.ieq_bands and cp.ieq_amount > 0.0:
        amount = min(max(cp.ieq_amount, 0.0), 1.0)
        total_db += amount * _interpolate_bands_db(device.band_freqs, cp.ieq_bands, freqs)

    if vlldp.audio_optimizer_enable and ch_idx in vlldp.audio_optimizer_bands:
        total_db += _interpolate_bands_db(device.band_freqs, vlldp.audio_optimizer_bands[ch_idx], freqs)

    if vlldp.speaker_peq_enable:
        for peq in vlldp.speaker_peq_filters:
            if peq.speaker == ch_idx:
                total_db += _biquad_mag_db(peq, freqs, device.sample_rate)

    total_db += cp.system_gain_db + cp.pregain_db + cp.postgain_db
    total_db += vlldp.system_gain_db + vlldp.postgain_db
    return total_db


def _min_phase_ir(mag_linear: np.ndarray, num_taps: int) -> np.ndarray:
    """Convert a one-sided magnitude spectrum into a causal minimum-phase FIR."""
    n_fft = (len(mag_linear) - 1) * 2
    log_mag = np.log(np.maximum(mag_linear, 1e-7))
    cepstrum = np.fft.irfft(log_mag, n=n_fft)

    window = np.zeros(n_fft, dtype=np.float64)
    window[0] = 1.0
    window[1 : n_fft // 2] = 2.0
    if n_fft % 2 == 0:
        window[n_fft // 2] = 1.0

    min_phase_spec = np.exp(np.fft.rfft(cepstrum * window, n=n_fft))
    ir = np.fft.irfft(min_phase_spec, n=n_fft)[:num_taps].copy()

    fade_len = min(256, num_taps // 4)
    if fade_len > 0:
        fade = 0.5 * (1.0 + np.cos(np.linspace(0.0, np.pi, fade_len)))
        ir[-fade_len:] *= fade
    return ir


def synthesize_profile_irs(
    device: Dax3DeviceData,
    profile_name: str,
    out_wav: str | Path,
    num_taps: int = 4096,
    n_fft: int = 16384,
) -> DspSummary:
    """Synthesize a stereo minimum-phase .irs WAV file for a DAX3 profile."""
    if profile_name not in device.profiles:
        available = ", ".join(sorted(device.profiles))
        raise KeyError(f"Profile '{profile_name}' not found. Available: {available}")

    profile = device.profiles[profile_name]
    freqs = np.fft.rfftfreq(n_fft, d=1.0 / device.sample_rate)
    channels = max(2, min(device.channels, 2))

    ch_curves_db = [
        _channel_response_db(device, profile, ch, freqs) for ch in range(channels)
    ]
    peak_gain_db = float(max(np.max(c) for c in ch_curves_db))
    target_ceiling_db = min(-0.5, profile.vlldp.peak_level_db - 0.5)
    headroom_trim_db = min(0.0, target_ceiling_db - peak_gain_db)

    irs = np.zeros((num_taps, channels), dtype=np.float32)
    for ch in range(channels):
        mag = 10.0 ** ((ch_curves_db[ch] + headroom_trim_db) / 20.0)
        irs[:, ch] = _min_phase_ir(mag, num_taps).astype(np.float32)

    out_path = Path(out_wav)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    wavfile.write(out_path, device.sample_rate, irs)

    return DspSummary(
        profile=profile_name,
        sample_rate=device.sample_rate,
        num_taps=num_taps,
        channels=channels,
        peak_gain_db=peak_gain_db,
        headroom_trim_db=headroom_trim_db,
        irs_path=out_path,
    )
