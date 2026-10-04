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
Scalar timelines currently cover opacity, camera zoom and composition backdrop
fade; arbitrary transform timelines are not implied. Bloom and camera shake still generate ignored-feature warnings. Basic camera
parallax is identified only with enabled camera settings and authored layer depth;
its report includes the wallpaper-surface pointer-focus limitation.

`SceneConfig::mouse` and `parallax` gate basic niri camera parallax together.
`particle_limit` remains reserved, reported as ignored and disabled in the Web UI.
No arbitrary mouse scripts, depth-map effect or particle budgeting is implied.

Texture dimensions, RGBA channel layout, mip data, and UV coordinates must agree
with actual uploads. Respect `GL_MAX_TEXTURE_SIZE` and GLES restrictions. Existing
mip selection and filtering behavior is implemented; do not classify it as missing
without a reproduction. Release GPU resources with the owning context alive.

The niri backend retains CPU scene assets across output removal and uploads them
into each replacement EGL context. Its upload, render, and teardown paths restore
the owning context before GL calls; missing outputs defer upload, while a ready
output without EGL remains an error. See the niri skill for output/retry lifecycle
contracts and pending desktop verification.

Video submission: `SceneNodeKind::Video` accepts package-relative `video` resources
and media paths in `image`; image model/material base textures also accept direct
media and embedded video TEX payloads (TEXB0004 and the TEX video flag). Encoded
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
reports references, not successful decoding. Desktop/GPU and eight-hour stability
verification remain separate requirements.

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
