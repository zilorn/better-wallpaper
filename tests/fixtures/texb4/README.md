# Owned TEXB4 fixtures

These files follow the verified reader layout documented in
[the TEXB4 investigation](../../../docs/reverse-engineering/wallpaper-engine-texb4.md).
They contain original generated pixels/media, not commercial wallpaper resources
or native screenshots. `manifest.json` records hashes and fixed inputs.

- `video.tex`: flag `0x22`, raw codec, no editor records, existing 16×16 lossless
  `scene-video.mp4` payload (red/green/blue/yellow, 0.5 seconds each, no audio).
- `video-metadata.tex`: the same media with two opaque editor records and an
  empty per-mip variant table. Editor records do not imply video by themselves.
- `static-metadata.tex`: one RGBA pixel `[23,67,119,255]`, two records and an empty
  per-mip variant table. It must remain static in parsing and asset resolution.

Run `python tests/fixtures/texb4/generate.py` for a read-only byte-for-byte check;
`--write` deliberately refreshes the three inputs and manifest. The generator
never writes output references. Unit tests add all TEXB1–4 versions, optional
LZ4, malformed counts/strings/lengths, metadata byte budgets and every truncated
prefix. Existing PNG tests cover non-video TEXB3/4 mip chains.

The corrected `scene-behavior/video/scene.pkg` drives the GPU regression and the
daemon clock/pause/loop/reload test. Renderer tests consume these metadata
fixtures directly. Conditional mip variants, volume editor metadata and property
or script evaluation remain explicitly unsupported. Parsing these synthetic
records does not establish original-program editor behavior.
