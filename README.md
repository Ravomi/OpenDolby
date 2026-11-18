# OpenDolby

Open-source Dolby DAX3 speaker tuning parser and PipeWire `filter-chain` generator for Linux.

> **Not a generic EQ preset.** OpenDolby directly restores the OEM's speaker-specific DAX3 tuning (crossovers, biquads, and multiband curves) from Windows driver XMLs and runs it natively on PipeWire with zero-latency minimum-phase FIR convolution.

![OpenDolby Synthesized Frequency Response](docs/frequency-response.png)

Built and verified on **ASUS Zenbook UX3404VC** (`Realtek ALC294`, `VEN_10EC&DEV_0294&SUBSYS_10431693`) using the official DAX3 XML tuning extracted from ASUS `DolbyAPOdriver` (`DEV_0294_SUBSYS_10431693_PCI_SUBSYS_16931043.xml`).

## Quickstart

```bash
# Run directly with uv (no manual venv or cargo needed)
git clone https://github.com/1jehuang/OpenDolby.git && cd OpenDolby
uv run python -m opendolby.cli install music

# Or install CLI globally via pipx / uv tool
uv tool install .
opendolby install music
```

To revert at any time and restore plain speaker output:

```bash
uv run python -m opendolby.cli uninstall
```

### Install / Switch / Uninstall on PipeWire

```bash
# Install the 'music' profile and restart PipeWire
python3 -m opendolby.cli install music

# Switch to 'movie' or 'dynamic'
python3 -m opendolby.cli install movie

# Remove all OpenDolby PipeWire configs and restore plain Speaker output
python3 -m opendolby.cli uninstall
```

## What It Does

1. **Parses DAX3 XML (`opendolby/parser.py`)**: Reads `tuning-cp` (IEQ, GEQ, Audio/Process Optimizer, gains) and `tuning-vlldp` (speaker 20-band optimizer, `speaker-peq-filters` biquads/Butterworth crossovers, multiband compressor, regulator thresholds, peak ceiling).
2. **Synthesizes Minimum-Phase FIR (`opendolby/dsp.py`)**: Combines active frequency-domain curves and biquad responses on a dense FFT grid, converts to a causal minimum-phase impulse response (`.irs` stereo float32 WAV) via real cepstrum, and trims headroom safely below `0 dBFS`.
3. **Generates PipeWire `filter-chain` (`opendolby/pipewire.py`)**: Emits a zero-latency `builtin` convolver + optional `lsp-plugins` LV2 brickwall limiter (`limiter_stereo`) targeting the hardware speaker sink.

See [docs/architecture.md](docs/architecture.md) for the DAX3-to-Linux DSP mapping details.
