# Owned A7 SceneScript probes

`python3 tests/fixtures/scenescript-native/generate.py` verifies the 20 owned
inputs. `--write` regenerates inputs only; it never writes native screenshots.
`manifest.json` fixes all inputs, the original-program identity, eight captures,
the seven positive references, negative evidence and comparison thresholds.
There is no random, audio, cursor or wall-clock input. The canvas is 128x128;
a 16x16 RGBA (255,64,16,255) marker uses point/clamp sampling over black.

| Probe | Expected result / role |
| --- | --- |
| origin | Absolute marker center (80,48), in Y-up canvas units |
| scale | Scale (2,0.5,1), centered at (64,64) |
| alpha | Math abs/min gives alpha 0.5 |
| layer | Static own origin.x / 128 gives alpha 0.5 |
| scene-layer | Literal getLayer('Probe').size.x / 32 gives alpha 0.5 |
| property | Declared numeric slider opacity defaults to 0.5; override 0.25 tested separately |
| clock | Absolute X=80 + sin(runtime*0); invariant native clock/API probe |
| base-value | Native adds (16,-16) each frame and leaves canvas; **negative** evidence, rejected by runtime |
| motion | Analytical X=64+runtime*16 at fixed scene times; no synchronized native reference |
| fault | Division by zero at runtime 0.5; authored alpha retained until fresh plan |

Seven native time-invariant images are positive pixel references; base-value's
black screenshot must never be treated as a positive fallback reference. All
eight were independently recaptured with identical RGB. The program's observed
previous-value semantics prompted rejection of stateful updates. Native captures
do not establish fractional clock synchronization or arbitrary script fidelity.

The explicit GPU harness compares against native pixels with SSIM >=0.995,
RGBA MSE <=1 and max channel error <=2. Analytical integer motion/disabled/fault
fallback images use exact RGBA assertions. Each device runs 184 comparisons over
two contexts and repeated upload/reload, with frozen/rewound time and property
override checks. Existing PNG references elsewhere are unchanged.

Provision a copied app and dedicated Wine prefix as in the
[Tint protocol](../tint-native/README.md), preserving symlinks when copying a
prefix (`cp -a` or `shutil.copytree(..., symlinks=True)`). Never dereference Wine
drive links into unrelated directories. Then:

```sh
python3 tests/fixtures/scenescript-native/capture.py --work /absolute/private-directory
python3 tests/fixtures/scenescript-native/capture.py --work /absolute/private-directory --capture-name repeat
```

Both output directories must be new. The script verifies program/input hashes,
runs only manifest native cases in a private Xvfb window and joins Wine/Xvfb.
It writes into the private directory, never repository references. No program
binary or commercial script/shader body is committed. See
[acceptance](../../../docs/validation/a7-scenescript.md) for results and limits.
