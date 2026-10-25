# Rendering Interfaces and Invariants

| Interface | Source and contract |
| --- | --- |
| `build_scene_2d_plan(&SceneGraph, Scene2dOptions)` | `better-wallpaper-renderer/src/scene2d.rs`; returns `Result<Scene2dPlan, Scene2dError>`. Rejects empty viewports and invalid scene hierarchy; preserves deterministic draw planning. |
| `resolve_scene_2d_assets(&PkgReader, Scene2dPlan)` | Same source; returns `Result<Scene2dAssets, Scene2dError>`. Resolves model/material/texture dependencies without creating GPU objects. |
| `SceneGpuRenderer::upload_scene_textures(&Scene2dAssets)` | `gpu-renderer/src/scene_renderer.rs`; uploads validated assets into the owning GL context. |
| `SceneGpuRenderer::resize(width, height)` / `draw_scene(&Scene2dAssets, elapsed_seconds)` | Same source; output size and elapsed scene time are separate inputs. Drawing can fail and must not be reported as successful playback. |
| `GpuRenderer::upload_frame(data, width, height, stride)` | `gpu-renderer/src/lib.rs`; compact RGBA requires `stride == width * 4` and sufficient bytes. CUDA upload is a separate method requiring available interop. |
| `GpuRenderer::draw(output_width, output_height, FillMode)` | Same source; supports cover, contain, stretch. Preserve shared GL state when embedding in another renderer. |
| `GpuResourceCache<T>` | `better-wallpaper-renderer/src/cache.rs`; owns resources with normalized path/content-hash keys, budget accounting, and deterministic LRU eviction. Removal/drop must release the actual GPU objects. |

Decoded video storage is selected by `DecodedFrame.cuda`. CPU frames have compact
RGBA pixels and `stride == width * 4`; CUDA frames have two device-resident NV12
planes with byte pitches, empty `pixels` and zero CPU stride. The `format` field
describes CPU storage only. Retained frames own their memory/context across decoder
seek and teardown. Consumers must select the CUDA upload path before inspecting CPU
pixels. `DecodeOptions.hardware` requests acceleration but permits software fallback.
`max_height` caps either path without upscaling: native-size NV12 stays on the GPU;
downscaled or non-NV12 hardware frames transfer to CPU RGBA. This transfer has a
performance cost and is not a zero-copy path.

All paths above are beneath `crates/`. Scene format types and parsing live in
`crates/better-wallpaper-scene-format/src/`; public rendering exports are listed
in `crates/better-wallpaper-renderer/src/lib.rs`.

`gpu-renderer` is a Rust library consumed by the Wayland/niri backend. The unused
Plasma C ABI and static/shared library build outputs were removed; construct video
renderers with `GpuRenderer::new(glow::Context, width, height)`. Plasma currently
uses Qt Multimedia for HTTP video playback and has no shared GPU scene consumer.

Keep scene semantics in shared parsing/planning rather than independently in niri
and Plasma. Preserve unsupported/skipped-node reporting and library compatibility
metadata. Unknown private-format semantics need legal sample evidence.

`scene::compute_compatibility` reports the highest supported feature tier actually
identified in a scene: L0 for metadata only, L1 for 2D layers/text, L2 for supported
scalar timelines, referenced sprite animation, package video textures or background audio, and L3 for
whitelisted 2D effects or the bounded native audio-spectrum pattern. No current
feature awards L4. `supported_features`, `unsupported_features`, and `warnings`
must be read together: mixed scenes retain their supported tier and all known
limitations. The tier is metadata analysis, not asset validation, visual parity,
or proof that playback works on the selected backend.

`analyse_scene_with_package(scene_json, &PkgReader)` adds TEX animation detection
through referenced image model/material base textures; unused textures do not
raise the tier. Library scanning and `scene-validate` use this package-aware path;
unpacked `scene-inspect` can only classify features visible in scene.json.
Effect identifiers and audio-spectrum recognition are shared with the IR parser.
Scalar timelines cover opacity, camera zoom and composition backdrop fade. A4
also admits bounded autonomous linear XY origin/scale offsets on ordinary quads;
curves, scripts, scheduling, angle/size/depth channels and mesh consumers retain
explicit limitations. Bloom and camera shake still generate ignored-feature warnings. Basic camera
parallax is identified only with enabled camera settings and authored layer depth;
its report includes the wallpaper-surface pointer-focus limitation.

`SceneConfig::mouse` and `parallax` gate basic niri camera parallax together.
`particle_limit` enforces one aggregate admitted-particle capacity (0–100,000,
default 10,000), in authored draw order; zero disables particles. GTK exposes it.
No arbitrary mouse scripts or depth-map effect is implied.
The native GTK client also exposes mouse/parallax/audio settings and preserves
library compatibility details; it omits the reserved particle/script controls.
Its scene property editor changes configuration overrides, not renderer support.

A6 adds `particle::ParticleSystem`, `resolve_particle` and shared node admission
in the format crate. One boxrandom emitter, explicit fixed lifetime, linear
position/velocity/size/color ranges, force-free movement and a default sprite
renderer are admitted. Package-owned unpadded single-image/base-mip TEX and one
genericparticle translucent/additive pass are required. Unknown fields/operators,
atlas/video frames, effects, control-point offsets, scripts, parented/animated or
nonplanar particle transforms fail admission with specific compatibility/log
reasons. Package-aware analysis alone can grant L3 for validated particles;
scene.json-only analysis requests package validation. Mixed scenes retain limits.

`Scene2dDraw.particles` retains one immutable system and the FNV-1a node seed.
`Scene2dAssets::set_particle_limit` partitions capacity across systems; the daemon
applies saved configuration during scene preparation. Sampling uses f64 scene
time and SplitMix64 counter channels, without per-frame integration or history.
Budget pressure drops fixed birth residues, with no late resurrection; this and
the RNG are documented application policies, not native random/maxcount parity.
At most 64 systems, 100,000 scene particles, 8 MiB per particle texture and 32 MiB
retained particle texture bytes are admitted. Repeated definitions share pixels.
Each system submits one triangle batch, at most 24 MiB of vertex staging for the
hard scene limit. GPU clear releases batch storage and textures; drop deletes the
buffers with their owning context. Ordinary draws restore their white vertex
color and original layout. Native static probes calibrate rendered width to half
the authored size and preserve texture aspect. See the [subset and matrix](../../../../docs/validation/a6-particle-subset.md)
and [acceptance](../../../../docs/validation/a6-particles.md) for verified boundaries.

TEX container contracts are verified against local Wallpaper Engine 2.8.42
native-reader evidence in `docs/reverse-engineering/wallpaper-engine-textures.md`.
TEXB1–4 store image count first; TEXB3/4 then store the FreeImage codec.
TEXB4 next stores an editor-record count, not a video boolean. Video is TEXI
flag `0x20`; raw video requires one image/mip, codec -1 (or no codec in TEXB1/2),
valid dimensions, nonempty/exactly decompressed media and no trailing bytes.
The existing 64 MiB encoded/decoded TEX mip and caller budget limits apply.
Both TEXB4 adapters enforce mip counts, exact LZ4 lengths and pixel-size checks;
retained TEXB4 bytes cannot exceed 128 MiB even through the legacy adapter.
`TexAtlas.editor_metadata` and `TexTexture.editor_metadata` retain three opaque
words and UTF-8 JSON per record. Caps are 256 records, 64 KiB per string and
1 MiB total serialized metadata; malformed JSON/UTF-8/NUL termination fails.
A nonzero editor count also requires a per-mip variant-table count after the
main payload. Nonempty conditional tables and volume editor metadata return
explicit unsupported errors; no property/script condition evaluation occurs.
`scene-inspect FILE.tex --atlas --json` reports metadata, video detection and
payload SHA-256; `encoded_video_requires_decoder` does not claim decoding.
See [native TEXB4 evidence](../../../../docs/reverse-engineering/wallpaper-engine-texb4.md)
and [A3 validation](../../../../docs/validation/a3-texb4.md).
TEXS1 reads signed integer coordinates, TEXS2/3 read floats, and TEXS3 adds logical
frame dimensions. Frame records contain image index, duration, origin and two
basis vectors. The legacy `TexTexture` adapter (tools, fallback/effect textures)
represents one image with positive axis-aligned rectangles. Multiple images,
volume flag 0x40, off-axis or negative-direction bases and missing frame images return
explicit errors. Unknown TEXS versions and over-limit/truncated frame records
are rejected. Zero-duration particle atlases do not imply timed sprite playback.
`cargo run -p better-wallpaper-scene-format --example texture-audit -- /local/path`
inspects loose TEX files without following symlinks; pixel validation is not GPU
or scene-playback validation.

`TexVolume::parse` / `parse_with_budget` provide a separate inspection-only model
for single-image volumes with raw ARGB8888 or vertically stacked PNG payloads.
TEXB1–4 raw, TEXB3/4 PNG and optional LZ4 are supported; PNG output is RGBA8,
raw bytes retain their format, and x varies fastest, then y, then z. Preserve
source dimensions separately from storage width/height/depth and the opaque final
TEXI word. Storage width/height are bounded by 8192, depth by 128, source dimensions
by 16384, each decoded or encoded mip by 64 MiB, and retained pixels by 128 MiB
(or a lower caller budget). Check the complete retained-byte sum before allocating
each mip. One encoded/decompression buffer and bounded PNG scratch may coexist.
Mip counts are bounded by 32 and the complete chain length; dimensions must be a
consecutive floor-halved prefix. Reject other formats/codecs, multiple images,
animation/video, malformed LZ4/PNG, trailing data and unsupported layouts.
`scene-inspect FILE.tex --volume --json` bounds input at 128 MiB and reports
dimensions, raw format, decoded byte hashes and `scene_playback_available: false`.
The 2D parsers continue to reject volumes as image textures. A separate bounded
`ColorLut` effect path now admits a stricter volume subset, described below.
The 28 local base-level LUT PNGs pass inspection and independent decoded-hash
checks; that is not LUT playback or native visual parity. See
[layout evidence](../../../../docs/reverse-engineering/wallpaper-engine-volumes.md)
and [validation](../../../../docs/validation/a2-volume-parser.md).

`ColorLut::parse` admits one base-only raw ARGB8888/PNG volume with axes 2–128,
point/clamp flags only and at most 8 MiB decoded bytes. `resolve_color_lut` reads
package effect/material/instance definitions and grants the known `ccsimple`
LUT-only operation only with one visible effect/pass, texture slot one and static
`lutparams` in 0–1. COL/HDR, unknown fields/combos/constants, extra targets/passes,
arbitrary fragment/vertex overrides and unsafe paths fail admission. The owned
MVP probe adapter is the only supported package vertex override. Image-model
quads preserve raw effect instances for this package check; text, procedural
solids, direct video and fallback shortcuts retain their previous limits.

`Scene2dDraw.color_lut` carries shared RGBA voxels, dimensions/sampler flags,
package path and blend amount. LUT bytes deduplicate by path with a separate
32 MiB scene budget; one additional bounded decode can coexist during admission.
GPU contexts keep a separate LUT cache, counted with images against the 256
resident-object limit. A nearest-filtered 2D slice stack and explicit voxel
interpolation/addressing implement normalized 3D sampling in GLES2. Stack width
and height*depth must fit GL_MAX_TEXTURE_SIZE; there is no resizing fallback.
Failed upload/missing required LUT returns an error; clear/drop releases every
LUT in the owning context. Reloaded assets require cache clear as for 2D images.
Package-aware compatibility only adds L3 for valid effect binding plus LUT layout;
volume parsing alone never grants the tier. No Plasma scene consumer is implied.

Native normalized sampling uses c*N-0.5 for linear and floor(c*N) for point,
with per-axis clamp/repeat. RGB is sampled directly in the verified LDR path;
alpha is retained and strength zero is exact. The standard 32³ neutral lattice
has an observed native bias of up to four byte values; this is matched, not
silently corrected. Eleven independently recaptured native probes pass 176
comparisons per device on NVIDIA and llvmpipe, max error 1; neutral is exact.
See [LUT evidence](../../../../docs/reverse-engineering/wallpaper-engine-luts.md)
and [acceptance](../../../../docs/validation/a2-lut-rendering.md). Broader source
minification/post-process order, app-global filters, HDR/COL and animated property
bindings remain outside this bounded operation.

`TexAtlas::parse` is the model for inspection and main scene material textures: independent image/mip lists,
full signed affine frame bases and TEXS3 logical frame dimensions. It caps 64
images, 32 mips per image, 10,000 frames and 128 MiB retained bytes across images;
`parse_with_budget` can lower the byte cap. The existing per-mip/ratio limits
apply, with one temporary mip during decoding. It requires finite frame fields,
nonnegative durations, valid indices, complete animation blocks and validated
2D payload sizes. Zero-duration, degenerate and out-of-range bases remain data,
not proof of draw support until sprite admission. Editor metadata follows the bounded TEXB4 contract above; volume and
multi-video layouts fail. `to_texture_images` validates independent pixel chains;
`try_into_single_texture` enforces the legacy single-image layout gate.
`scene-inspect /path/atlas.tex --atlas --json` adds bounded (128 MiB file input)
structural inspection with explicit payload/layout/native-validation fields.
`single_image_layout` reports the legacy adapter; `sprite_layout` reports shared
sprite admission without a GPU or native-parity claim. See the
[atlas evidence and contracts](../../../../docs/reverse-engineering/wallpaper-engine-atlases.md).

`TexAtlas::sprite_animation_with_sampling` is shared by resource resolution and
package-aware compatibility analysis; `sprite_animation` retains the inferred
legacy sampling contract. Material passes carry `spritesheet: Option<bool>`: an
explicit case-insensitive `SPRITESHEET` combo must be integer 0 or 1. Zero disables
animation; one selects native sampling; absence preserves inferred animation. It requires a complete nonempty sequence for animated or
multi-image data, positive finite durations, nondegenerate bases at least one
texel long, and all four corners inside the selected image. Any failed frame
rejects the whole draw; frames are never filtered into a partial animation.
One valid frame is a fixed transformed crop; two or more identify sprite animation
for L2 metadata. `SceneMetadata.unsupported_textures` preserves concrete errors
in `CompatibilityReport.unsupported_features`. Tier remains metadata, not proof
of successful GPU upload or backend playback.

Shared `SpriteFrame`/`SpriteAnimation` types now live in the format crate and are
re-exported by the renderer. A frame has an image index and affine `TextureUv`
(origin and two basis vectors); `frame_at` uses elapsed scene time only.
Authored frame durations remain `f32`, but `SpriteAnimation.duration_seconds`
is their `f64` sum, matching the playback clock and frame selection precision.
This prevents early/late loop boundaries in long fractional-duration sequences;
the existing maximum admitted total of `f32::MAX` seconds remains enforced.
`Scene2dDraw.texture` is image zero, `additional_textures` are indices 1 and above.
Main material TEX payloads share decoded bytes by path within a scene and have a
512 MiB aggregate retained-byte cap, in addition to each atlas's 128 MiB cap.
This aggregate does not include direct media, effect/fallback textures or fonts.
GPU cache entries contain complete image groups; failed uploads roll back the
group. The 256 resident-image cap counts each image/mask separately and returns
an error instead of evicting part of an active atlas. Clear/drop releases every
image with the owning context current.

Quads, puppet UVs and effect displacements use the full affine basis. Explicit
native sprite materials use authored normalized UV edges without an inset.
`SpriteAnimation.native_sampling: Option<SpriteSampling>` carries TEX flag bit 0
(point rather than linear) and bit 1 (clamp rather than repeat). Native sampling
uses authored complete contiguous mip chains, including packed frames, and does
not use generated mips for base-only inputs. Partial/noncontiguous multi-level
chains fail shared admission with a concrete error; GLES2 cannot bound their
accessible mip range without inventing data. Reset main sampler state after
optional texture bindings, since bindings may share a texture object.

For missing-combo inferred animation, UVs retain the half-texel inset along each
basis; one-texel axes sample their center. `SpriteFrame.full_image` distinguishes
whole-image rotations/reflections from crops. This legacy path uses base-level
linear filtering for crops and restores available mip filtering for full frames;
optional effect textures restore linear magnification and their own mip filter.
This is the established compatibility policy, not native missing-combo behavior.

Fourteen owned original-program captures and 134 controlled comparisons per
renderer validate this bounded subset on NVIDIA RTX 3060 Ti and Mesa llvmpipe,
including image switching, flips/skew, edges, authored mips, loop/frozen time,
reload and context replacement. The native uniform chain and sample acceptance
are documented in [native evidence](../../../../docs/reverse-engineering/wallpaper-engine-atlas-uniforms.md)
and [validation](../../../../docs/validation/a1-native-atlases.md).
Atlas-backed effect masks and system-texture fallbacks retain the legacy adapter;
advanced effects, broader alpha/color fidelity and Plasma scene playback are not
established. Real niri submissions are tested separately; native CLI pause did not
freeze the Wine probe, and no precise native clock equivalence is claimed.

`cargo run -p better-wallpaper-scene-format --bin asset-audit -- --help` describes
the bounded JSON inventory for named loose TEX/JSON roots, explicit shared roots
and selected PKGs. It records SHA-256, package member offsets, format/payload
results and JSON-field candidate references without loading unselected resources.
`TexTexture::parse_with_budget` adds a caller-selected retained mip-byte ceiling;
TEXB4 also has the unconditional 128 MiB ceiling described above. Exit 2 still emits a report but indicates
incomplete traversal/read/reference coverage. Audit JSON accepts line/block comments
and trailing commas after strict parsing fails, reporting `json_extended_syntax_valid`
separately from `json_syntax_valid`. Its temporary normalization preserves strings
and diagnostic positions; record hashes and archive offsets remain based on original
bytes. The 2 MiB input cap, UTF-8/nesting checks and candidate limits apply to both
forms. Malformed input contributes no partial references. Runtime JSON parsers are
unchanged. Candidate matches and pixel-size
validation are not proof of supported scene playback. See
[audit contracts](../../../../docs/asset-audit.md) for limits and coverage.

Texture dimensions, RGBA channel layout, mip data, and UV coordinates must agree
with actual uploads. Respect `GL_MAX_TEXTURE_SIZE` and GLES restrictions. Existing
mip selection and filtering behavior is implemented; do not classify it as missing
without a reproduction. Release GPU resources with the owning context alive.

Generated RGBA mip levels use integer area-weighted box filtering. For odd
dimensions, source texels crossing a destination footprint boundary contribute
proportionally to both sides (for example 5→2 uses two 2.5-texel footprints).
All four byte channels retain truncating division; authored mip levels and the
compressed-texture path are unchanged. This does not establish native filtering,
linear-light/alpha-aware filtering, or packed-atlas edge parity. The explicit GPU
case `odd_texture_minification_preserves_area_and_authored_mips` checks a 5×5→1×1
readback against analytical bytes and verifies authored mip precedence.

The niri backend retains CPU scene assets across output removal and uploads them
into each replacement EGL context. Its upload, render, and teardown paths restore
the owning context before GL calls; missing outputs defer upload, while a ready
output without EGL remains an error. See the niri skill for output/retry lifecycle
contracts and pending desktop verification.

Video submission: `SceneNodeKind::Video` accepts package-relative `video` resources
and media paths in `image`; image model/material base textures also accept direct
media and embedded video TEX payloads (TEXI video flag `0x20`, including TEXB3/4). Encoded
bytes and `SceneVideoTexture.frame` are shared across draws and outputs. Each GL
context tracks frame generations, updates RGBA storage on size changes, and
reuploads the retained CPU frame after output replacement. Media uses full UVs.
The niri daemon owns `SceneVideoRuntime`: at most eight decoders, two queued frames
per decoder, 128 MiB per encoded payload and 64 MiB per RGBA frame. Quality limits
height to 720/1080/2160. PTS follows the pause-aware shared scene clock, loops advance
by media duration/last frame end, and dropped frames are released immediately.
Private extracted files and workers are released on scene reload/cancellation.
FFmpeg package decoding permits file-only MOV/Matroska/AVI containers and disables
external MOV references; playlists and network demuxers are rejected. Video texture
audio is not mixed; use the scene background audio node. Compatibility detection
reports references, not successful decoding. A3 verifies nine local TEXB3/4
payloads with exact byte hashes and first-frame decoding at 720p; two over-limit
video mips remain explicitly rejected. Owned inputs cover scene-clock boundaries,
pause, looping, three reloads, shared decoders, the eight-decoder ceiling, partial
startup failure, private-file permissions/removal and release of CPU/encoded bytes.
All nine explicit GPU regressions pass on NVIDIA and llvmpipe. Texture audio is
still not mixed, and no native video clock/visual equivalence is claimed. Desktop
lifecycle and eight-hour stability verification remain separate requirements.

Basic parallax: `SceneCamera.parallax` parses `cameraparallaxamount` (0–100),
`cameraparallaxdelay` (0–10 seconds) and `cameraparallaxmouseinfluence` (0–10),
including property-bound values. Defaults are 0.1, 0.1 and 1 respectively.
`SceneNode.parallax_depth` parses `parallaxDepth` as two finite axes in -100–100;
missing/zero depth leaves the layer fixed. Shared plans/assets preserve both.
`SceneParallaxState` smooths normalized, Y-up pointer coordinates [-1,1] on the
pause-aware scene clock; elapsed-time rewind resets it. Per-axis translation is
`-pointer * amount * mouse_influence * depth`, limited to ±2 NDC before cover/zoom.
`draw_scene_with_pointer` consumes this state; `draw_scene` supplies no pointer.
This is a bounded basic approximation, not demonstrated Wallpaper Engine pixel
parity. Input loss returns smoothly to center. Source semantics:
[Wallpaper Engine parallax](https://docs.wallpaperengine.io/en/scene/parallax/introduction.html)
and [IScene parameters](https://docs.wallpaperengine.io/en/scene/scenescript/reference/class/IScene.html).

GPU regression command:
`LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture`.
The explicit surfaceless EGL/GLES test builds real shared plans/assets, invokes
production shaders and compares readback against seventeen independent synthetic PNG
references. Gates: local 8×8 SSIM ≥0.995, RGBA MSE ≤1, maximum byte error ≤2.
Inputs now come from 13 committed projects and `tests/fixtures/scene-behavior/matrix.json`:
all thirteen renderable samples with 50 ordered steps. Five A1 samples also retain
explicit rejection checks for the legacy adapter. A separate GPU case adds five
comparisons for upload rollback, capacity preservation, reload and rotation/effect
composition. The ordinary suite checks package/project/PNG hashes and asset outcomes.
`python3 tests/fixtures/scene-behavior/generate.py` checks deterministic regeneration;
`--write` deliberately refreshes inputs without changing PNGs.
`python3 crates/gpu-renderer/tests/golden/generate_atlas_references.py` checks seven
analytical A1 PNGs independently of renderer output. Native acceptance
and reference captures are pending; video frame replacement is a consumer probe.
Failure emits actual/difference images under `target/scene-golden-failures`.
The ordinary workspace suite runs comparator checks but explicitly ignores the
GPU cases; missing EGL fails an explicit GPU run. See
`crates/gpu-renderer/tests/golden/README.md` for regeneration and scope.
Straight-alpha translucent draws now use separate color and alpha factors:
source-over preserves alpha=1 on opaque backgrounds. Additive draws saturate
color/alpha. The golden exposed the previous incorrect alpha multiplication.

## Bounded A4 Tint operation

`resolve_tint(package, effects, &mut TintMaskCache)` admits one visible Tint
effect with one material/instance pass, verified stock shader identities, LDR
RGB/alpha, modes 0/2/7/30 and autonomous linear alpha animation. Property-bound
defaults/overrides pass through shared IR. Static point/clamp RGBA8 masks retain
logical/storage UV scaling and use red, with 16 MiB per-image/32 MiB aggregate
limits and path deduplication. `Scene2dDraw.tint` carries the admitted operation.
Masks share the existing 256-image GPU budget and clear/drop context ownership.
Missing required uploaded masks fail drawing. Mode 0 forces output alpha one;
other modes preserve source alpha before layer composition. Unverified shaders,
stacks/multiple passes, other modes, HDR, curves/scripts and linear masks retain
concrete unsupported diagnostics; runtime preserves the base image. Package-aware
compatibility adds Tint to supported features only after the shared check. Mixed
scenes keep their limitations. See [A4 Tint acceptance](../../../../docs/validation/a4-tint.md).

## A4 motion and calibrated parallax

`SceneTransform.origin_animation`/`scale_animation` carry `VectorAnimation`: up
to 1,024 keys per channel, fps (0,1000], length (0,1e9], unique in-range frames,
offset magnitude <=1e6 and empty/zero Z. Native vector channels add to saved
transforms for either observed editor-relative flag. Scalar channels remain
absolute; frame arithmetic uses f64 scene time, including long-running loops.
Unsupported transform controls retain saved values and specific compatibility
diagnostics. `Scene2dQuad.motion` shares authored transforms in root-to-leaf
order, with at most 64 ancestors; `Scene2dMotion::vertices_at` rejects non-finite
results. Puppet and procedural spectrum motion consumers are explicitly excluded.

`scene_parallax_offset` now takes authored projection and layout viewport as well
as settings, pointer and depth. Displacement is in half-projection units followed
by cover/zoom, fixing wide-scene offsets. Defaults are native amount 0.5, delay
0.1 and influence 0.5. Delay zero is immediate; delay (0,3] uses the native capped
step min(1, dt*10*(1-delay/3)); 3 holds position. Equal paused timestamps freeze
input, rewind and clear/reload reset state. Native smoothing depends on cadence,
replacing the former exponential time-constant assumption. Input remains local
to the current exposed wallpaper surface.

Eighteen independently repeated native endpoint images, one measured native
smoothing trace and 230 motion comparisons per device pass on NVIDIA and Mesa.
Tint now has 184 comparisons per device including fractional alpha and missing
mask recovery. All eleven GPU cases and 108 live niri submissions pass. See
[A4 acceptance](../../../../docs/validation/a4-motion.md) for limits and commands.
