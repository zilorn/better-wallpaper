# Test fixtures

[`texb4/`](texb4/README.md) contains owned raw video and static TEXB4 inputs with
zero/two editor records. A3's parser boundaries, asset resolution, scene-clock
pause/loop/reload and extracted-file cleanup checks use these inputs or the
corrected `scene-behavior/video/scene.pkg`. Commercial sample identities and
explicit size-limit exclusions are recorded separately in
[`a3-texb4-samples.json`](../../docs/validation/a3-texb4-samples.json).

[`scene-behavior/`](scene-behavior/README.md) contains 13 reproducible standalone
synthetic scene projects and a fixed-input matrix. GPU goldens consume these
packages directly: eight scenes retain the existing 20 comparisons; five A1
probes require explicit layout rejection. The matrix records hashes and
analytical reference provenance, not native captures.

`dynamic-scene/scene.pkg` is an original generated 4×2 scene containing one 4×2
RGBA TEXB2/TEXS2 atlas with two 2×2 frames: cyan `[0,220,255,255]`, then orange
`[255,90,0,255]`. Both frames select atlas image zero and last 0.5 seconds.
Expected sample times are 0 seconds (cyan), 0.5 seconds (orange), and 1 second
(cyan after looping). Properties are empty, no random seed is used, and pointer
input is irrelevant. Regenerate with:

```sh
cargo run -p better-wallpaper-scene-format --example generate-dynamic-scene -- tests/fixtures/dynamic-scene
```

The checked-in package was regenerated after the A0 audit found stale frame
bases from the old coordinate layout. Its current SHA-256 is
`2408281477093c829fa10ceeeadf590c4aa40e867bdf9e0f2e54ec1d558d3aae`.
`checked_in_dynamic_scene_uses_current_sprite_layout` parses the actual checked-in
bytes and verifies image indices, rectangles and timing, so a correct generator
alone cannot hide a stale package again. This is a synthetic input, not an
independently captured Wallpaper Engine reference.

`h264.mp4`, `vp9.webm` and `av1.mkv` are the existing 64×36, five-frame decoder
fixtures. `vfr-h264.mp4` has five frames with unequal presentation intervals;
`corrupt-video.bin` must fail to open. The H.264 fixture is YUV420P, 10 FPS,
0.5 seconds, with time base 1/10240. No pointer, property or random input is used
in these decoder regressions. Run them with:

```sh
cargo test -p better-wallpaper-ffmpeg --test fixture_decode -- --nocapture
```

The output names the observed CPU RGBA or CUDA NV12 storage. With GPU access,
the H.264 hardware request exercises CUDA; without a usable device it exercises
software fallback. A separate subprocess test sets `CUDA_VISIBLE_DEVICES=-1` and
requires CPU output even on a CUDA host. Height caps of 18 and 72 check reduction
to 32×18 and no upscaling respectively. Complete decode, timestamps, seek and
retained storage after decoder teardown are checked for both storage variants.
The tests validate CUDA plane/pitch/ownership metadata, not rendered GPU pixels.

Current fixture SHA-256 identities (assets are unchanged):

| Fixture | SHA-256 |
| --- | --- |
| `h264.mp4` | `98096298b05622e33c35e17ed75bf192dea091a4136819c1ef990cb87302cf0f` |
| `vp9.webm` | `05a939ae54a38747371e80c251b8270c9eed64173e966629ad523c7de7f8fcc6` |
| `av1.mkv` | `379bef1482ec0fd06905794a1aca2785c4ded5eb99e6afa1f9851fc39c8d6306` |
| `vfr-h264.mp4` | `56f73059fa8436a722ef8bd41d50fa1e30bfe07cff7cd78fc17a6c3269d3dc11` |
| `corrupt-video.bin` | `f085014a82fbe0e5b9cf46d2cfb2edaeec005599de96c374838b36794e7a76ae` |

`scene-video.mp4` is an original, generated 16×16, 2 FPS, two-second lossless
H.264 RGB clip: red, green, blue, yellow. It contains no audio. Regenerate from
four RGBA frames using `ffmpeg -f rawvideo -pixel_format rgba -video_size 16x16
-framerate 2 -i colors.rgba -c:v libx264rgb -crf 0 -preset ultrafast scene-video.mp4`.
It verifies scene-clock scheduling, loop boundaries, pause, shared CPU frames,
and cancellation while the bounded queue is full.
