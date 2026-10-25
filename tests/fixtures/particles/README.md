# Owned A6 particle matrix

All packages are generated from authored JSON and synthetic TEX pixels, with no
commercial wallpaper content. `python3 generate.py` verifies inputs and hashes;
`--write` regenerates only packages, descriptors and the manifest. It never
generates or overwrites captures. The viewport is 128×128, node ID 17, no property
or pointer overrides. The fixed RNG policy is FNV-1a of UTF-8 node ID followed by
SplitMix64 birth/channel counters, pinned by an independent integer test vector.

| Inputs | Purpose and fixed times |
| --- | --- |
| static, overlap, additive | One/two burst particles; size 32, RGB 128/255/64, texture RGBA 255/128/64/128; times 0,1,100 |
| scale-one, scale-two | Explicit native scales 1 and 2; width 16 and 32, respectively |
| pattern, rotated, stretched | Asymmetric point-sampled quadrants; UV orientation, Z rotation and nonuniform scale |
| finite | Rate 10, duration 1, lifetime 3600, unit RGB additive contributions; ten contributions at times 2,3,100 |
| continuous | Rate 10, fixed life 2, velocity 8/0; births/deaths at 0,.1,.5,1,2,3 and 1e9 |
| random | Rate 10, life 2; fixed ranges for box, velocity, size and color; frozen/replayed times, budgets and node-seed separation |
| budget | 100,000 zero-size burst particles; default/zero/full capacity and actual 24,000,000-byte maximum vertex batch |
| unsupported | Turbulence operator must be rejected by runtime and package-aware compatibility |
| delayed, starttime-probe | Nonzero starttime is explicitly rejected; native starttime=60 remains visible and cannot be treated as a simple delay |

The ten PNGs in `captures/` come from the hash-matched original program under
Wine/Xvfb; all were independently recaptured with identical RGB bytes. Nine are
positive stable probes; starttime-probe is negative semantic evidence. File-byte
hashes and native input identities are retained in `manifest.json` and
`native-evidence.json`. No Better Wallpaper output generated a native reference.

Use [the isolated capture setup](../atlas-native/README.md) with copied app/bin
and prefix directories, then run `python3 capture.py /absolute/private/work`.
The harness requires a fresh recapture directory and captures the nine positive
probes. The negative starttime probe was captured using the same helper with the
explicit selection `['starttime-probe']`. Give concurrent runs separate prefixes.
This script imports the existing motion capture harness; its virtual window title
remains "A4 Motion Probe", without affecting input identity or the real desktop.

See [acceptance and limitations](../../../docs/validation/a6-particles.md).
These tests establish bounded native static sprite/finite-emission behavior and
application-clock repeatability. They do not establish the native RNG stream,
precise fractional birth clock, arbitrary operators, or Plasma scene playback.
