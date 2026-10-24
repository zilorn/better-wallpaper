# A2 bounded volume parser validation

Date: 2026-10-24. Implementation commit: `c70a60a`
(`feat: inspect bounded volume texture payloads`).
This completes the bounded data-model subitem of A2; the work package remains in
progress. See [native layout evidence](../reverse-engineering/wallpaper-engine-volumes.md)
for binary identity, Evidence IDs, file observations and unresolved semantics.

## Delivered behavior

`TexVolume::parse` and `parse_with_budget` inspect one nonanimated, nonvideo volume
image. Raw ARGB8888 retains its bytes; embedded PNG becomes RGBA8. TEXB1–4 raw,
TEXB3/4 PNG and LZ4 are supported. Each mip preserves width, height and depth;
source dimensions and the opaque final TEXI0001 word remain separate. Consecutive
mips halve each axis with floor division, clamped to one; base-only chains are valid.

Storage width/height ≤8192, depth ≤128, source dimensions ≤16384, at most 32 mips
and no more than a complete chain. Checked multiplication validates every
width×height×depth×4 size. Encoded/decompressed mip payloads and pixel mips each
have a 64 MiB ceiling; retained pixels have a 128 MiB aggregate ceiling, reducible
by the caller. Pixel budgets are checked before payload allocation/decoding. One
bounded encoded/decompression buffer and PNG scratch can coexist with retained
pixels. LZ4 must decode exactly its declaration and stay within the 512:1 limit.
PNG dimensions must match width×(height*depth).

`scene-inspect FILE.tex --volume --json` caps input at 128 MiB and includes decoded
SHA-256 identities. It rejects conflicting modes, malformed files and unsupported
formats. It reports `scene_playback_available: false` and
`native_visual_validation: pending`. Default inspection, atlas parsing, resource
resolution, compatibility levels and GPU/effect paths retain their existing limits.

## Automated and local checks

| Check | Result |
| --- | --- |
| `cargo clippy` | Passed |
| `cargo clippy -p better-wallpaper-scene-format --all-targets -- -D warnings` | Passed |
| `cargo fmt --all -- --check`; `git diff --check` | Passed |
| `cargo test --workspace` with local filesystem access | 266 passed, zero failures, 10 existing explicit desktop/GPU tests ignored |
| Changed format crate, included above | 126 passed: 108 library, 13 audit, 2 atlas CLI, 3 volume CLI |
| Owned fixture regeneration | Two TEX files and manifest reproduce byte-for-byte in a fresh directory |
| Selected native LUTs | 28/28 parsed; 3,670,016 retained RGBA bytes total |
| Repeated local inspection | All 28 reports identical; input size/hash identities match A0 |
| Independent Pillow decoding | All 28 decoded hashes match the Rust parser |
| Neutral 32³ cube | All 32,768 voxels match the analytical x/y/z ramp |

The first restricted full-suite run stopped at the existing daemon
`web_projects_keep_nested_paths_defaults_and_asset_boundaries` symlink-creation
test (`Operation not permitted`); the full local-access rerun passed. Test logs are
local temporary artifacts at `/tmp/better-wallpaper-a2-workspace.log` and
`/tmp/better-wallpaper-a2-workspace-local.log`.

Eight new library tests exercise voxel/channel ordering, odd noncubic raw chains,
TEXB versions and PNG/LZ4 combinations, independent PNG mip/alpha retention,
dimension/count/codec/version/length errors, cumulative budgets, corrupt/short
LZ4, equal-byte-count PNG shape mismatches, every truncated prefix of PNG/raw/LZ4
fixtures and trailing data. Three CLI tests exercise structural reporting,
continued 2D rejection, input/mode failures and fixture hashes. The
[owned inputs](../../tests/fixtures/volume-textures/README.md) contain no commercial
resources. Existing packages, analytical reference PNGs and native captures were
not modified.

## Reproduce the selected local inventory

The [manifest](a2-volumes.json) selects only direct `assets/materials/lut/*.tex`
files from the identified installation. It is separate from the historical A0
inventory; the existing `asset-audit` intentionally continues to report its 2D
volume limitation. Parser success is not scene playback or GPU validation.

Build the inspector, set the installation path explicitly, and generate a new
report for comparison (the script exits on any parse failure):

```sh
cargo build -p better-wallpaper-scene-format --bin scene-inspect
python3 - /absolute/path/to/wallpaper_engine /tmp/better-wallpaper-a2-volumes.json <<'PY'
import hashlib, json, subprocess, sys
from pathlib import Path
root = Path(sys.argv[1])
assets = []
for path in sorted((root / 'assets/materials/lut').glob('*.tex')):
    data = path.read_bytes()
    result = subprocess.run(
        ['target/debug/scene-inspect', str(path), '--volume', '--json'],
        capture_output=True, text=True, check=True)
    report = json.loads(result.stdout)
    report.pop('path')
    assets.append({'path': path.relative_to(root).as_posix(), 'bytes': len(data),
                   'sha256': hashlib.sha256(data).hexdigest(), **report})
assert assets, 'No selected LUT textures found'
report = {
    'schema_version': 1,
    'scope': 'Direct *.tex files in assets/materials/lut of the selected installation; no PKG traversal or playback claims.',
    'target_sha256': hashlib.sha256((root / 'wallpaper64.exe').read_bytes()).hexdigest(),
    'assets': assets,
}
Path(sys.argv[2]).write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
PY
cmp docs/validation/a2-volumes.json /tmp/better-wallpaper-a2-volumes.json
```

For the independent decoder check, use Pillow to open the PNG beginning at byte
99 of each selected file, convert to RGBA and compare `sha256(image.tobytes())`
with its recorded `decoded_sha256`. For `neutral.tex`, compare every voxel at
`((z*32+y)*32+x)*4` with
`(round(255*x/31), round(255*y/31), round(255*z/31), 255)`. The observed axis
order and per-file hashes are reproducible without running Wallpaper Engine.

## Remaining validation

No GPU/desktop code changed, so the 8 ignored GLES cases, niri submission case
and GTK UI case were not rerun explicitly. Native LUT screenshots, GPU volume
upload, capability fallbacks, interpolation/color space, effect parameter binding,
BCn/other codecs, alternate mip layouts and long-running fuzz/soak checks remain
pending. Original-program reader evidence and neutral file bytes do not establish
color-grading visual parity. A2 GPU/effect and A9 acceptance items remain open.
