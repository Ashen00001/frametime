# frametime

A Rust CLI that reads frame-time captures from any game and tells you, in plain language with numbers, what looks wrong and where. It can also compare two runs to answer "did that tweak actually help?" without fooling you with noise.

It is **read-only**: it analyzes capture files and never changes a capture, a game or a system setting.

```
cargo build --release
./target/release/frametime analyze capture.csv
./target/release/frametime compare before.csv after.csv
```

## What you get

- Core stats: average fps, 1% and 0.1% lows (**both** common definitions, each labelled), frame-time percentiles, std dev, frame-to-frame jitter.
- Stutter: spike frames (over a configurable multiple of the rolling median, default 2.0x and at least 2 ms over it), stutter events, and fixed hitch counts (over 33 / 50 / 100 / 200 ms).
- Frame budget against a target refresh rate (`--target-hz`, default 144).
- CPU- vs GPU-bound over time (needs PresentMon 2.x `CPUBusy`/`GPUBusy`; with a sensor file it falls back to GPU utilization, flagged as weak).
- Periodic stutter: a hitch that repeats every N seconds, with the period and how strongly it lines up.
- Startup / shader-compilation pattern: stutter concentrated in the first seconds that then settles.
- Present mode, tearing, dropped frames and display latency.
- Sensor correlation (HWiNFO / Afterburner CSV): spikes that coincide with throttle flags, clock drops, hot temps, near-full VRAM or RAM.
- A short ranked "what to look at" list, each item with evidence, a confidence (high / medium / low) and a caveat.
- Compare mode with side-by-side stats, percent changes, 95% intervals, a verdict that says "within noise" when it should, and per-phase numbers.
- Output: coloured terminal tables (respects `NO_COLOR`, off when piped), sparkline and histogram, `--json`, `--markdown`, and `--html report.html` (self-contained, inline SVG, no scripts).

## Supported inputs

The format is detected from the file header, not the extension.

| Format | Notes |
| --- | --- |
| PresentMon CSV 1.x | `MsBetweenPresents`, `MsUntilRenderComplete`, `MsUntilDisplayed`, `Dropped`, `AllowsTearing`, `PresentMode`, `TimeInSeconds` |
| PresentMon CSV 2.x | adds `FrameTime`, `CPUBusy`, `CPUWait`, `GPUBusy`, `GPUWait`, `DisplayLatency`, `FrameType`, `CPUStartTime` |
| CapFrameX CSV / JSON | metadata block above the header is skipped; `--run N` picks a run in multi-run JSON |
| RTSS / MSI Afterburner frame-time log | any delimited file with a `Frametime` column (ms); an FPS-only column is accepted but coarse |
| Plain list | one frame time in ms per line |

Multi-process PresentMon captures: the busiest process/swapchain is analyzed and the others are listed; use `--process NAME` to pick one. 2.x rows whose `FrameType` is not `Application` (generated frames) are skipped.

### Capturing

- **PresentMon**: `PresentMon.exe --process_name game.exe --output_file capture.csv --timed 120` (flag names differ between versions; check `--help`). Needs an elevated shell or membership in the *Performance Log Users* group.
- **CapFrameX**: capture with the hotkey, then export the run as CSV or JSON.
- **RTSS / Afterburner**: enable the benchmark/frame-time logging in RTSS, then point `frametime` at the log.
- **Sensors**: HWiNFO "Logging to CSV" or an Afterburner sensor export. Sensor time starts at the first row; captures carry no wall clock, so line the two up with `--sensor-offset SECONDS` (capture time + offset = sensor time) if you started them at different moments.

A 2-minute capture of normal play (not a menu, not a loading screen) is a good size. Use `--trim-start N` to drop loading seconds.

## Examples (synthetic data)

```
frametime synth periodic --seconds 120 --out demo.csv
frametime analyze demo.csv
frametime analyze demo.csv --json
frametime analyze demo.csv --target-hz 60 --html report.html

frametime synth clean --seed 1 --out a.csv
frametime synth random --seed 2 --out b.csv
frametime compare a.csv b.csv --markdown
```

`synth` kinds: `clean`, `periodic`, `startup`, `cpu-bound`, `gpu-bound`, `random`. Everything in the tests and docs is synthetic.

## How to read the findings (and not over-read them)

- **Lows**: the *percentile* method is fps at the 99th percentile frame time; the *average-of-worst-N* method is fps of the mean of the slowest 1%. They are different numbers for the same run. 1% of a 60-second capture is ~85 frames, so lows are noisy on short captures.
- **Spikes / hitches**: counts of what happened. A spike is relative to the local rolling median, so a steady 30 fps game is not "stuttering" for being slow.
- **Periodic**: events are mapped onto a circle at each candidate period (a Rayleigh periodogram); strength 1.00 is a perfect repeat. It then requires most gaps between events to be about one period. A rhythm is *consistent with* something periodic (a timer, autosave, streaming, a background app); the tool cannot say which. Needs 6+ events and a period of at most a quarter of the capture.
- **Startup**: heavy early stutter that settles is *consistent with* shader compilation or asset streaming, but a loading phase looks the same. Re-run with a warm shader cache to tell.
- **CPU/GPU-bound**: per frame, a unit is the limiter when busy for 85%+ of the frame. "Neither" usually means a frame cap or vsync. The sensor-utilization fallback cannot separate CPU-bound from capped.
- **Present mode**: "Composed" paths go through the desktop compositor and can add latency, but windowed play does that legitimately.
- **Sensors**: correlation only, and sensors sample slowly (0.5-2 s). A GPU clock drop at a hitch can be the GPU idling *during* the hitch rather than causing it. Heuristic thresholds: temps 83 C GPU / 90 C CPU, VRAM 92%, RAM 90%, clock under 85% of its own 95th percentile.
- **Compare**: each run is cut into ~5 s blocks and block-bootstrapped. A difference is called only if its 95% interval excludes zero **and** it is at least 2%; rare-event counts also need a Poisson rate test to agree. This measures variability *inside* each run. Two separate sessions of the same scene differ more (different traffic, different lap), so repeat the test before trusting small changes. With under ~30 s per run only changes of 15%+ are called and the rest is "inconclusive".

## Limitations

- Timestamps in captures are relative; sensor alignment is manual (`--sensor-offset`).
- Frame generation (DLSS FG, AFMF) makes "frame time" ambiguous; only `Application` frames are analyzed.
- No CapFrameX sensor blocks, no Afterburner `.hml` binary logs (export to CSV first).
- Heuristic thresholds are tuned for desktop PC games, not consoles or 30 fps locked titles.

## Roadmap

- `frametime library`: per-game history in SQLite with trends.
- `frametime capture`: wrap an already-installed PresentMon for N seconds (will not install it; needs admin or the Performance Log Users group).
- PNG chart output and an interactive `ratatui` view.
- Wall-clock sensor alignment when both files carry timestamps.

## Development

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same on Ubuntu and Windows. License: MIT.
