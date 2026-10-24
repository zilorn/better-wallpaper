# Owned volume texture probes

These files contain independently generated pixels, not copied application assets.
`manifest.json` records their encoded byte counts and SHA-256 identities.

| File | Input contract | Expected result |
| --- | --- | --- |
| `neutral-png.tex` | TEXB4, non-video, PNG 4×16, volume 4×4×4, source 16×4, one mip | RGBA `(85*x, 85*y, 85*z, 255)` at every voxel; preserve the opaque TEXI word `0xff123456` |
| `noncubic-raw.tex` | TEXB1, raw ARGB8888, 5×3×2 → 2×1×1 → 1×1×1 | Preserve raw bytes `(10+x, 40+y, 70+z, 200+level)`; 132 retained bytes over three mips |

The ordering is x first, then y, then z. These are parser probes, with no scene,
clock, random seed, pointer input, GPU output or native screenshot expectation.
The raw format is preserved without asserting a channel conversion. PNG decoding
produces canonical RGBA8. Test-generated variants cover TEXB1–4 raw payloads,
TEXB3/4 PNGs, LZ4, multi-mip PNGs and malformed inputs.

Regenerate into a fresh directory and compare all three generated files:

```sh
python3 tests/fixtures/volume-textures/generate.py --output /tmp/better-wallpaper-volume-fixtures
```

Inspect without admitting these textures for scene playback:

```sh
cargo run -p better-wallpaper-scene-format --bin scene-inspect -- tests/fixtures/volume-textures/neutral-png.tex --volume --json
cargo test -p better-wallpaper-scene-format
```

The default and `--atlas` inspection paths continue to reject volume textures.
See [native layout evidence](../../../docs/reverse-engineering/wallpaper-engine-volumes.md)
and [validation](../../../docs/validation/a2-volume-parser.md).
