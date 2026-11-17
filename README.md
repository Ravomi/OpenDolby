# OpenDolby / dolby-rs

Experimental Linux speaker DSP and GPUI-Kit control panel for this ASUS Zenbook's DAX3 tuning XML. **This is not a Dolby Atmos decoder, certified implementation, or a bit-exact port of the Windows APO.**

## Run

```sh
cargo build --release --bins
./target/release/dolby-gui
```

The GUI starts with the engine stopped. Start it, then choose the virtual output. Its output is pinned to the physical Speaker to avoid a self-loop. Stopping restores Speaker if the app's virtual output is currently selected.

- **Processed / Bypass**: A/B comparison through the same virtual sink. Both paths keep running. Switching crossfades over 20 ms.
- **RMS matching** (on by default): matches the two paths to the quieter rolling stereo RMS by attenuation only. Let it settle for about 1-2 seconds after a major content/profile change. This is approximate electrical RMS matching, **not perceptual equal loudness or LUFS normalization**. Matched bypass can therefore be quieter than unity-gain raw input. Turn matching off for unaligned output, but start at a low system volume.
- **Surround / Dialog**: live smoothed multipliers from 0 to 2, with 1 using the profile's approximation. Sliders never force an effect on if the XML disables it. Use Dynamic to audition these effects. The spatial stage is M/S widening and delayed crossfeed, not object-based Atmos rendering.
- **Off profile**: exact flat passthrough, separate from matched A/B bypass. It does not retain speaker-protection DSP. Avoid loud sub-bass tests on laptop speakers.

The default XML path is currently the local ASUS driver extraction in `src/lib.rs`. It is not bundled or licensed by this project. The CLI accepts `--xml PATH` before its subcommand.

## DSP changes and limits

The old implementation cascaded each 20-band target directly as an overlapping peaking filter. The replacement fits the **combined** cascade response at the XML's actual nonuniform band centers. Prototype Q=4.3 is an engineering choice, not a recovered Dolby constant. The fit uses analytic RBJ biquad responses and control-thread-only iterations. Between-center behavior and phase are still approximations. IEQ remains a static target approximation, not the proprietary content-adaptive algorithm.

OEM PEQ/high-pass and the existing leveler/compressor remain. Compressor parameter semantics, crossover summation, and unused regulator fields have not been validated against a Windows APO capture. A generic output limiter is not proof of original OEM speaker protection. Do not call this a complete or acoustically verified OEM restoration.

Streaming callbacks are currently serialized on the PipeWire mainloop (no `RT_PROCESS`), with bounded stereo buffering and atomic live controls. This deliberately avoids the old cross-thread `Arc<RefCell<_>>` sharing but is **not a hard-real-time-certified engine**. Profile construction happens on the control thread and replacement on the mainloop timer. Changing profiles can reset DSP state; use the A/B toggle, not repeated profile changes, for quick comparisons.

## Reproducible checks

```sh
cargo test --all-targets
python3 tools/measure_dsp.py --label after --check-matching
# Stop the engine first. This uses only a temporary silent null sink:
python3 tools/smoke_pipewire.py
```

The Python measurement tool requires NumPy and SciPy and writes deterministic offline comparison WAVs/JSON under `target/dsp-check/`. It does not play audio. To export matched wet/dry versions of the same content:

```sh
./target/release/dolby-rs process-wav input.wav wet.wav --profile dynamic
./target/release/dolby-rs process-wav input.wav dry.wav --profile dynamic --bypass
# Disable alignment explicitly for an unaligned reference:
./target/release/dolby-rs process-wav input.wav unaligned.wav --profile dynamic --no-level-match
```

### Local validation (2026-11-18, ALC294 / 0x10431693)

- `cargo test --all-targets`: 14 tests passed, including variable block sizes, live invalid-value handling, exact Off passthrough and matched A/B.
- Deterministic 8-second, 48 kHz stereo band-limited noise at approximately -29.12 dBFS: the old Dynamic output was +17.05 dB RMS above input. With matching enabled, settled Dynamic/Music/Movie wet-versus-bypass differences were all below 0.001 dB. This is a signal-specific electrical measurement, not proof of perceptual equality on arbitrary music.
- Temporary silent PipeWire null-sink test: 240000 captured stereo frames at 48 kHz, RMS about 0.00360 per channel, finite output and no silent 100 ms blocks in the measured 1-4 second interval. Default hardware Speaker remained unchanged. No speaker test tone was played.
- The installed PipeWire 1.6.11 `pw-record` returns 1 when its sample-count limit ends recording without setting `data.drained`. The smoke test accepts that case only with an empty error log and exactly the requested frame count, then checks the actual samples and routing.

Sources used for the implementation: W3C Audio EQ Cookbook (Robert Bristow-Johnson), the installed GPUI-Kit Slider/SliderState API, and the installed PipeWire stream/buffer API. These specify filter/control mechanisms, not Dolby's proprietary algorithms.
