# A1 fractional atlas animation timing

Date: 2026-10-24. Implementation commit: `2ee21a2`.

## Reproduction and correction

`TexAtlas::sprite_animation` previously summed stored frame durations in `f32`,
while `SpriteAnimation::frame_at` subtracted them using the `f64` scene clock.
The resulting loop period could disagree with the time needed to display all
frames. For 10,000 frames whose stored duration is `0.1_f32`:

| Quantity | Seconds |
| --- | --- |
| Previous accumulated period | 999.902893066406 |
| Sum of the exact stored durations | 1000.000014901161 |
| Premature loop | 0.097121834755 |

The new regression failed before the fix at cycle 0, frame 9,999, one quarter
of the way through that frame: playback had already returned to frame zero.

The loop period is now accumulated and stored as `f64`. Per-frame values retain
their original `f32` representation, and the total still cannot exceed
`f32::MAX` seconds. Existing invalid-duration and whole-sequence admission rules
remain in force. This changes the public shared `SpriteAnimation.duration_seconds`
field from `f32` to `f64`; all workspace consumers compile against it.

## Regression inputs

- CPU: the committed multi-image fixture supplies valid frame/image layouts.
  Constructed 10,000-frame sequences use uniform `0.1_f32` durations and repeating
  `0.1_f32`, `0.2_f32`, `0.3_f32` durations. There are 150 frame-identity checks
  at selected starts/interiors, including the last two frames, across cycle
  offsets 0, 1, 28, 1,000 and -1.
- GPU: two synthetic 2×2 solid red/green images alternate across 10,000 frames.
  Expectations use the stored duration multiplied by the known frame count,
  independent of the runtime's computed period. Eight timestamp/color checks
  run on initial upload, after clear/reload, and in a replacement EGL context:
  24 comparisons in total. They include the formerly truncated last frame,
  exact loop/frame boundaries, paused time, resumed time and clock reset.
- No committed package or reference PNG was modified. GPU expectations are
  analytical colors; no output from this renderer is used as a reference.

## Validation

| Check | Result |
| --- | --- |
| New CPU regression before fix | Failed at cycle 0, frame 9,999, fraction 0.25 |
| `cargo test -p better-wallpaper-scene-format --lib sprite_ -- --nocapture` | 8 passed after fix |
| `cargo clippy` | Passed |
| `cargo clippy -p better-wallpaper-scene-format -p gpu-renderer --all-targets -- -D warnings` | Passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo test --workspace` with normal local filesystem access | 239 passed, 0 failed, 6 explicit GPU cases ignored |
| Explicit GPU command below | 6 passed on llvmpipe (LLVM 23.1.1, 256 bits) |

```sh
LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture
```

The explicit GPU run includes all 24 new comparisons, the 78 existing atlas
sampling comparisons, 55 existing PNG comparisons and two exact odd-mip byte
assertions. The new comparisons have SSIM=1 and zero channel error.

The first restricted workspace run stopped at the existing daemon test
`web_wallpaper::tests::web_projects_keep_nested_paths_defaults_and_asset_boundaries`
because creating its symbolic link returned `Operation not permitted`. The
complete rerun with normal local filesystem access passed; no test was skipped
to work around that failure.

## Remaining limits

This verifies deterministic frame selection at supplied scene times, including
times near eight hours, not eight hours of actual playback. Native Wallpaper
Engine visual/timing parity, real niri output lifecycle and release soak testing
remain pending. The legacy single-image adapter's informational
`TexTexture.spritesheet_duration` remains `f32`; main atlas playback uses the
corrected shared animation model.
