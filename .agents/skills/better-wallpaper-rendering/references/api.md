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

Keep scene semantics in shared parsing/planning rather than independently in niri
and Plasma. Preserve unsupported/skipped-node reporting and library compatibility
metadata. Unknown private-format semantics need legal sample evidence.

Texture dimensions, RGBA channel layout, mip data, and UV coordinates must agree
with actual uploads. Respect `GL_MAX_TEXTURE_SIZE` and GLES restrictions. Existing
mip selection and filtering behavior is implemented; do not classify it as missing
without a reproduction. Release GPU resources with the owning context alive.
