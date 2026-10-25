# A4 bounded Tint acceptance

Date: 2026-10-25. Scope: one visible, package-validated LDR Tint operation on
ordinary image-model layers. Other effects retain their established behavior.

## Selection and evidence

The [survey](a4-scene-survey.json) records scene member SHA-256, offset and size
for all 30 sorted local Workshop `*/scene.pkg` files under Steam app 431960.
Its initial counts are authored occurrences (excluding literal `visible:false`),
not measured whole-project failures. A follow-up unwrapping `visible.value`
finds 31 default-enabled Tint instances, with 52 disabled bindings. Opacity has
38 default-enabled instances and remains a separate candidate. Tint was selected
for its 83 authored instances and directly reproducible color difference. All
83 are unmasked; 82 select mode 30 and one mode 20 (still unsupported).
The seven observed position/scale timelines are script/Bezier-controlled and
are not claimed to be fixed by an autonomous linear-timeline implementation.

The installed reference is Wallpaper Engine 2.8.42/build 23967692:
`wallpaper64.exe`, SHA-256
`40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0`.
The native operation follows `effects/tint/effect.json` →
`materials/effects/tint.json` → `shaders/effects/tint.vert`/`.frag` →
`ApplyBlending` in `common_blending.h`. These are shipped-resource observations,
corroborated by native execution, rather than reconstructed original C++ source.

| Installed resource | SHA-256 |
| --- | --- |
| Tint fragment | `02b9397cec32de6f0d8caa2e1af7c07510752d474bd64048c9d78f55926973f1` |
| Tint vertex | `62b2f5853fc565d706c1b6c789981385254f8195e0e1579c3d737796b41ddf2a` |
| common_blending.h | `f1cf5473991614784a7feafbd34c65b83a27851bfe9d763ca9c8c3fcbb2dcc6b` |

The fragment samples source RGB and blends it with authored RGB using alpha
times the mask's red channel. Mode 30 multiplies tint RGB by the maximum source
channel; modes 2 and 7 multiply/screen; mode 0 replaces RGB and forces output
alpha to one. Other admitted modes retain source alpha. The mask vertex scales
coordinates by logical/storage dimensions. No linear-light conversion is added.
All original-program RGB captures were repeated independently and agree exactly.

## Contracts and validation

Shared admission checks the actual effect/material/pass and known shader hashes,
not just a filename. One visible effect/pass, LDR RGB/alpha, static properties
and a bounded autonomous linear alpha timeline are supported. Unknown modes,
fields, shader variants, stacks, render targets, scripts and nonlinear animation
are rejected with concrete diagnostics. Runtime retains the base layer when Tint
is unsupported; package-aware compatibility preserves supported and unsupported
parts of mixed scenes. The GTK report consumes these existing lists unchanged.

Point/clamp masks are base-only RGBA8 with 16 MiB per-image and 32 MiB aggregate
limits; decoded masks deduplicate by path. GPU masks share the existing 256-image
budget, clear/drop lifecycle and owning-context rules. A missing uploaded mask
fails drawing instead of silently removing it. Linear masks remain explicitly
unsupported after the measured edge mismatch; the acceptance threshold is unchanged.

Commands:

```sh
cargo clippy
cargo fmt --all -- --check
cargo test -p better-wallpaper-scene-format -p better-wallpaper-renderer -p gpu-renderer
cargo test -p gpu-renderer --test scene_golden tint_ -- --include-ignored --test-threads=1 --nocapture
python3 tests/fixtures/tint-native/generate.py
```

Initial explicit GPU validation on Mesa llvmpipe: 176 native comparisons pass,
maximum channel error 1. This includes zero strength, disable, normal-mode alpha,
mask red/padding, frozen/rewound times, two reloads and two contexts. The retained
linear-mask negative probe is excluded from supported comparisons and tested for
admission failure. Final integration results and other-device/desktop boundaries
are recorded with [A4's motion acceptance](a4-motion.md). The final targeted
matrix adds fractional-alpha and missing-mask recovery checks: 184 comparisons
on each of NVIDIA RTX 3060 Ti and Mesa llvmpipe, maximum channel error one.

The first native capture attempt was blocked by local-display socket permissions;
the bounded capture succeeded with local access. Logs are `/tmp/a4-tint-*.log`;
native temporary packages and repeat captures are under `/tmp/a4-native/`.
No original shader source, commercial assets or executable is redistributed.
