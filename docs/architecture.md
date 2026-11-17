# OpenDolby Architecture & DAX3 Mapping

## Hardware Match (Zenbook UX3404VC)

- Codec: `Realtek ALC294` (`0x10ec0294`)
- Subsystem ID: `0x10431693`
- Matched XML: `data/DEV_0294_SUBSYS_10431693_PCI_SUBSYS_16931043.xml` (`48000 Hz`, 2-channel `internal_speaker`)

## DAX3 Processing Pipeline vs. Linux PipeWire

```mermaid
flowchart LR
    XML["DAX3 Tuning XML (CP + VLLDP)"] --> Parser["opendolby.parser"]
    Parser --> DSP["opendolby.dsp (Min-Phase FIR .irs)"]
    DSP --> PW["PipeWire filter-chain (convolver + lsp limiter)"]
    PW --> SPK["ALSA Speaker Sink"]
```

### Stage Mapping

| DAX3 XML Stage / Parameter | Scale / Format | OpenDolby Linux Handling |
| :--- | :--- | :--- |
| `tuning-vlldp/audio-optimizer-bands` | 20-band `dB * 16` | Log-freq interpolated into FIR |
| `tuning-vlldp/speaker-peq-filters` | Biquad `type` (1=Peak, 2=LowShelf, 3=HighShelf, 8=LPF, 9=HPF) | Evaluated analytically into FIR |
| `tuning-cp/audio-optimizer-bands`, `process-optimizer-bands` | 20-band `dB * 16` | Combined into FIR when enabled |
| `tuning-cp/ieq-bands-set` + `ieq-amount` | 20-band `dB * 16`, amount `0..16` | Weighted into FIR when enabled |
| `tuning-vlldp/peak-level`, gains | `dB * 16` | Headroom trim + LV2 `limiter_stereo` |
| `tuning-vlldp/mb-compressor-tuning` | 4-band threshold/ratio/attack/release | Parsed for inspection (future LV2 MBC) |
| Proprietary Atmos virtualizer | Closed-source APO (`DolbyDax3Apo.dll`) | Not portable directly; speaker acoustics handled by FIR |
