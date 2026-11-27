# SigmaOS audio-tuner

> Forked from AaryanSinghChauhan09/SigmaOS-audio-tuner (MIT). See `SIGMAOS_FORK.md`.

Tools for authoring laptop speaker tunings for
[SigmaOS](https://github.com/AaryanSinghChauhan09/SigmaOS).

Laptop speakers ship voiced by the vendor's Windows DSP layer, which Linux does
not get. SigmaOS restores that per model as a PipeWire filter-chain — see
`docs/AUDIO-TUNING.md` in the SigmaOS repo for how a tuning is shipped and
applied. This repo is the other half: measuring a laptop and producing the
`filter-chain.conf` that goes into it.

Most people never need this. Install it only if you are building a tuning for a
laptop SigmaOS does not cover yet.

## Requirements

```bash
sigmaos pkg add python ffmpeg mpv lsp-plugins-lv2
```

`lsp-plugins-lv2` supplies the lookahead limiter every tuning ends in, so it is
needed to audition a generated chain, not just to author one.

## The short version

Generate the probe once, then measure, fit, and generate:

```bash
sigmaos-audio-tuner probe                          # once, before measuring
sigmaos-audio-tuner capture <raw-speaker-sink> raw.wav
sigmaos-audio-tuner analyse raw.wav > raw.txt
```

From a git checkout use `./sigmaos-audio-tuner` instead; it is the same script.
Generated files (the probe and its tone list) go to
`~/.cache/sigmaos-audio-tuner`, so nothing is written into the install tree.

Everything measures the same way: play the probe into a sink and capture the
result. `capture.sh` always records the **physical** speaker sink's monitor, no
matter which sink you play into, because that is where both the raw and the
processed signal end up. A sink's monitor carries what it *receives*, so tapping a
DSP sink's own monitor measures the signal before its processing and every path
comes back identical.

## When there is a reference to copy

The easy case, and how the Dell XPS 14 tuning was made: something already sounds
right — a hand-tuned EasyEffects preset, say — and you want it as a filter-chain.
Measure both paths, subtract, fit. No microphone involved.

```bash
sigmaos-audio-tuner capture <raw-sink> raw.wav && sigmaos-audio-tuner analyse raw.wav > raw.txt
sigmaos-audio-tuner capture <ref-sink> ref.wav && sigmaos-audio-tuner analyse ref.wav > ref.txt
sigmaos-audio-tuner delta raw.txt ref.txt > target.txt   # what the tuning must do
sigmaos-audio-tuner fit target.txt 10 > fit.txt          # biquads that do it
sigmaos-audio-tuner generate fit.txt > filter-chain.conf
```

Copy the result into SigmaOS as
`default/audio/tunings/<vendor>-<model>/filter-chain.conf`, add a `tuning.conf`
beside it, and `sigmaos audio tuning on`.

A reference containing a compressor has no single response — it measures
differently under different signals. The probe here is dense and pink-weighted for
that reason, so a compressor in the path behaves roughly as it does on music.
Check afterwards whether the dynamics mattered by comparing loudness range. On the
XPS they did not: the reference's 8-band compressor changed LRA by 0.1 LU, so a
linear chain replaced it completely.

## When there is not

Now you are designing, not copying, and two things change.

First, you need a **calibrated measurement microphone** — a UMIK-1 or similar,
with its calibration file. In the copy case above the mic never appears; if you do
use one there, its own response cancels in the subtraction. Deriving a target from
a single absolute measurement gives it nowhere to cancel, so an uncalibrated mic
bakes its own curve into the tuning as error. A shaped voice mic is disqualifying:
its presence lift becomes a presence *cut* in your correction.

Measure near-field, a few centimetres out, to keep the room out of it:

```bash
sigmaos-audio-tuner capture --from <mic-source> --seconds 4 <raw-sink> acoustic.wav
sigmaos-audio-tuner analyse acoustic.wav > acoustic.txt   # then subtract your cal file
```

Second, and more importantly: **there is no correct target curve.** Flat is the
wrong answer for laptop speakers — the XPS tuning is deliberately far from flat.
Useful practice is to correct the sharp deviations (cabinet and port resonances)
without fighting the broad roll-off you cannot win, then voice deliberately from
there. That last part is taste, and no measurement settles it.

What *is* objective is the physical envelope, and it is worth establishing first
because it decides where the high-pass goes and how much bass lift is safe:

```bash
MIC=<mic-source> sigmaos-audio-tuner mic-sweep <sink> tone100.wav 100 raw
```

That reports harmonic distortion against level at a fixed frequency. Because it
compares the same frequencies across levels, the mic's own response cancels, so an
uncalibrated mic is fine for it. Sweep a few frequencies in the region you intend
to lift, find where distortion takes off, and do not lift past it.

## Auditioning

Loudness decides blind comparisons if you let it, so both tools level-match.

```bash
sigmaos-audio-tuner compare <track>   # calibrated A/B on one track
sigmaos-audio-tuner switch            # move your own playing music between candidates
```

`switch` is the one to use with Spotify: start playing, then switch with the
number keys and the stream moves without restarting. It stops EasyEffects for the
session if it is running, because EasyEffects moves any stream that follows the
default sink to its own sink and would grab audio back from a filter-chain.

## What a tuning must report

A tuning is not reviewable on "sounds better to me". SigmaOS asks for these in
`tuning.conf`:

| Field | What it is | How |
|---|---|---|
| `magnitude_rms_db` | Deviation from your reference or target | `analyse-dense.py` |
| `bass_group_delay_swing_ms` | Max minus min group delay, 30–300 Hz | From the biquad coefficients |
| `limiter_headroom_db` | Worst-case peak vs the limiter threshold | `ffmpeg volumedetect` on a hot master |
| `dynamic_range_delta_lu` | LRA change vs the reference | `ffmpeg ebur128` |

Two hard requirements, whatever the numbers say:

- **End in a limiter.** Peaks must stay under 0 dBFS with headroom.
- **Do not boost what the drivers cannot deliver.** The XPS 14 tuning deliberately
  *cuts* 40 Hz by around 18 dB. Excursion down there buys nothing and costs
  distortion.

## Traps

These cost real time building the first tuning.

- **Do not measure with a third-octave bandpass.** `ffmpeg bandpass=width_type=o`
  has wide skirts, so wherever the response has a steep gradient the leakage does
  not cancel between two paths. It read +1.5 dB at 100 Hz on hardware where a sine
  said -3.1 dB, and produced a match figure wrong by 4.6 dB. That is why the probe
  is integer-Hz tones analysed over exactly one second at 48 kHz.
- **Do not optimise magnitude alone.** Matching a convolution notch with a high-Q
  IIR biquad buys magnitude accuracy and pays in group delay. The first XPS fit
  reached 0.91 dB RMS but swung 31 ms across 63–80 Hz, which smears bass
  transients. Capping bass Q cost 0.33 dB and halved the swing.
- **LSP `limiter_stereo` defaults `alr` and `boost` to enabled.** Both add
  programme-dependent gain, so a fixed tuning must set both to 0 or its tone drifts
  with programme level. `gen-filter-chain.py` does this.
- **Verify where a stream landed.** `mpv --audio-device=pulse/<sink>` silently
  falls back to the default sink when the name does not resolve, and
  `pactl move-sink-input` reports success while leaving a pinned stream where it
  was. The tools here check the landing sink rather than the exit code.

## License

MIT, same as SigmaOS.
## Documentation

Wiki documentation is available in the SigmaOS Wiki Staging repository:

- [SigmaOS Wiki Staging](https://github.com/AaryanSinghChauhan09/sigmaos-wiki-staging#SigmaOS-audio-tuner) - Full Wiki documentation for all SigmaOS components

## See Also

Related SigmaOS **sigmaos** repositories:

- [SigmaOS-aether](../SigmaOS-aether/README.md) — SigmaOS-aether
- [SigmaOS-aether.nvim](../SigmaOS-aether.nvim/README.md) — SigmaOS-aether.nvim
- [SigmaOS-amiga-screensaver](../SigmaOS-amiga-screensaver/README.md) — SigmaOS-amiga-screensaver
- [SigmaOS-app-env](../SigmaOS-app-env/README.md) — SigmaOS-app-env
- [SigmaOS-archs](../SigmaOS-archs/README.md) — SigmaOS-archs
