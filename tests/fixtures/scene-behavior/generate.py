#!/usr/bin/env python3
"""Own synthetic scene inputs; never derive references from renderer output.

Default is a read-only reproducibility check. --write deliberately refreshes
the committed inputs and matrix, but never touches the analytical PNGs.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[2]
GOLDEN = REPO / "crates/gpu-renderer/tests/golden"


def words(*values):
    return struct.pack("<" + "I" * len(values), *values)


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def package(entries):
    def string(value):
        data = value.encode()
        return words(len(data)) + data
    index, payload = bytearray(), bytearray()
    for name, data in sorted(entries.items()):
        index += string(name) + words(len(payload), len(data))
        payload += data
    return string("PKGV0001") + words(len(entries)) + index + payload


def texture(width, height, storage_width, rgba):
    return (b"TEXV0005\0TEXI0001\0" + words(0, 0, storage_width, height, width, height, 0)
            + b"TEXB0002\0" + words(1, 1, storage_width, height, 0, len(rgba), len(rgba)) + rgba)


def animation(tex, version, frames, logical=(32, 32)):
    tex = bytearray(tex)
    tex[22:26] = words(4)
    tex += f"TEXS{version:04}\0".encode() + words(len(frames))
    if version == 3:
        tex += words(*logical)
    for image, basis in frames:
        tex += words(image) + struct.pack("<f", 0.5)
        tex += struct.pack("<" + ("i" if version == 1 else "f") * 6, *basis)
    return bytes(tex)


def scene_for(name):
    return {"general": {"orthogonalprojection": {"width": 32, "height": 32}},
            "objects": [{"image": f"models/{name}.json", "origin": "16 16 0", "size": "32 32"}]}


def step(time, reference, *, viewport=(32, 32), pointer=None, video=None, recreate=False):
    png = (GOLDEN / f"{reference}.png").read_bytes()
    return {"time_seconds": time, "viewport": viewport, "pointer": pointer,
            "video_action": video, "recreate_context": recreate,
            "reference": reference, "reference_sha256": hashlib.sha256(png).hexdigest()}


def generate():
    files, samples = {}, []

    def add(name, scene, textures, steps, purpose, error=None):
        entries = {"scene.json": encoded(scene)}
        for key, tex, blend in textures:
            entries[f"models/{key}.json"] = encoded({"material": f"materials/{key}.json"})
            entries[f"materials/{key}.json"] = encoded({"passes": [
                {"shader": "genericimage4", "blending": blend, "textures": [key]}]})
            entries[f"materials/{key}.tex"] = tex
        artifacts = {}
        for filename, data in {
            "scene.pkg": package(entries),
            "project.json": encoded({"title": f"Better Wallpaper A0 {name}", "type": "scene", "file": "scene.json"}),
        }.items():
            path = f"{name}/{filename}"
            files[path] = data
            artifacts[filename] = {"path": path, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
        samples.append({"id": name, "purpose": purpose, "artifacts": artifacts,
                        "canvas": [32, 32], "properties": {}, "random_seed": None,
                        "native_reference": "pending", "steps": steps,
                        "legacy_rejection": error,
                        "outcome": {"kind": "render", "draws": len(textures),
                                    "skipped_nodes": len(scene["objects"]) - len(textures)}})

    layers = {"general": {"orthogonalprojection": {"width": 32, "height": 32},
                          "cameraparallax": True, "cameraparallaxamount": 0.25,
                          "cameraparallaxdelay": 0, "cameraparallaxmouseinfluence": 1}, "objects": [
        {"id": "bg", "image": "models/bg.json", "origin": "16 16 0", "size": "32 32", "parallaxDepth": "0 0"},
        {"id": "parent", "container": True, "origin": "4 0 0"},
        {"id": "fg", "parent": "parent", "image": "models/fg.json", "origin": "8 20 0", "size": "8 12",
         "angles": "0 0 1.5707963267948966", "alpha": 0.5, "parallaxDepth": "1 0"},
        {"id": "add", "image": "models/add.json", "origin": "24 8 0", "size": "8 8", "alpha": 0.5,
         "parallaxDepth": "0 1"}]}
    layer_textures = [(name, texture(1, 1, 1, bytes(color)), blend) for name, color, blend in [
        ("bg", [24, 40, 80, 255], "opaque"), ("fg", [200, 80, 32, 128], "translucent"),
        ("add", [32, 48, 16, 255], "additive")]]
    add("layers", layers, layer_textures,
        [step(0, "layers"), step(0, "cover", viewport=(64, 32)),
         step(1, "parallax", pointer=[1, 1]), step(1, "parallax", pointer=[-1, -1])],
        "Parent transform, radians, blending/order, cover, per-axis parallax and paused pointer.")
    for name, target, key, start, end, default in [
        ("opacity", "object", "alpha", 0, 1, 0.5), ("zoom", "general", "zoom", 1, 2, 1)]:
        scene = copy.deepcopy(layers)
        owner = scene["objects"][2] if target == "object" else scene["general"]
        owner[key] = {"value": default, "animation": {
            "c0": [{"frame": 0, "value": start}, {"frame": 60, "value": end}],
            "options": {"fps": 30, "length": 60, "mode": "single"}}}
        add(name, scene, layer_textures, [step(2, f"{name}-end")], f"Single scalar {name} timeline endpoint.")

    atlas = (bytes([255, 0, 0, 255]) * 32 + bytes([0, 255, 0, 255]) * 32) * 32
    for version in range(1, 4):
        tex = animation(texture(64, 32, 64, atlas), version,
                        [(0, [0, 0, 32, 0, 0, 32]), (0, [32, 0, 32, 0, 0, 32])])
        add(f"sprite-v{version}", scene_for("sprite"), [("sprite", tex, "opaque")],
            [step(0, "sprite-red"), step(0.5, "sprite-green"), step(1, "sprite-red")],
            f"TEXS{version} rectangular frame selection and one-second loop.")

    rgba = bytearray()
    for y in range(32):
        for x in range(64):
            if x >= 32:
                color = [255, 0, 255, 255]
            elif y < 16:
                color = [180, 20, 40, 255] if x < 16 else [20, 160, 60, 255]
            else:
                color = [30, 50, 170, 255] if x < 16 else [150, 120, 10, 255]
            rgba += bytes(color)
    add("padding", scene_for("uv"), [("uv", texture(32, 32, 64, rgba), "opaque")],
        [step(0, "uv-padding")], "Top-down asymmetric quadrants; magenta storage padding must not appear.")

    media = (ROOT.parent / "scene-video.mp4").read_bytes()
    video = (b"TEXV0005\0TEXI0001\0" + words(0, 32, 16, 16, 16, 16, 0) + b"TEXB0004\0"
             + words(1, 0xffffffff, 0, 1, 16, 16, 0, len(media), len(media)) + media)
    add("video", scene_for("video"), [("video", video, "opaque")],
        [step(0, "video-red", video={"kind": "decode_next"}),
         step(0.5, "video-green", video={"kind": "decode_next"}),
         step(1, "video-green", video={"kind": "replace_solid", "size": [8, 8], "rgba": [0, 255, 0, 255]}),
         step(1, "video-green", recreate=True)],
        "Native-reader TEXB4 layout, video flag 0x20 and zero editor records; decode, storage resize, context replacement.")

    def atlas_steps(first, second):
        return [step(0, first), step(0.499999, first), step(0.5, second),
                step(0.5, second), step(1, first), step(0.5, second, recreate=True)]

    # Complete independent image chains, not a header-only count mutation.
    multi = bytearray(texture(2, 2, 2, bytes([200, 40, 10, 255]) * 4))
    multi[55:59] = words(2)  # image count
    multi[59:63] = words(2)  # first image has two mips
    multi += words(1, 1, 0, 4, 4) + bytes([200, 40, 10, 255])
    multi += words(1, 1, 2, 0, 8, 8) + bytes([20, 180, 60, 255]) * 2
    multi = animation(multi, 2, [(0, [0, 0, 2, 0, 0, 2]), (1, [0, 0, 1, 0, 0, 2])])
    add("multi-image", scene_for("atlas"), [("atlas", multi, "opaque")], atlas_steps("atlas-image0", "atlas-image1"),
        "A1: image 0 has 2x2 and 1x1 mips; image 1 has one 1x2 mip; frames select image 0 then 1.",
        "multiple atlas images are not supported")
    asymmetric = bytes([240, 10, 20, 255, 20, 220, 30, 255, 30, 40, 200, 255, 180, 130, 10, 255])
    for name, basis, error in [
        ("rotated", [2, 0, 0, 2, -2, 0], "Rotated or skewed"),
        ("skewed", [0, 0, 1, 0, 1, 2], "Rotated or skewed"),
        ("flipped-x", [2, 0, -2, 0, 0, 2], "Flipped texture animation"),
        ("flipped-y", [0, 2, 2, 0, 0, -2], "Flipped texture animation"),
    ]:
        tex = animation(texture(2, 2, 2, asymmetric), 2, [(0, basis), (0, [0, 0, 2, 0, 0, 2])])
        add(name, scene_for("atlas"), [("atlas", tex, "opaque")], atlas_steps(f"atlas-{name}", "atlas-identity"),
            f"A1 {name} basis, then identity; synthetic rendering regression, native visual result still unverified.", error)

    manifest = {"schema_version": 1, "provenance": "independently_authored_synthetic",
                "pointer_coordinates": "normalized_y_up_minus_one_to_one",
                "randomness": "unused; null seed means no random operators",
                "reference_kind": "analytical_png_not_wallpaper_engine_capture",
                "gate": {"ssim_min": 0.995, "rgba_mse_max": 1.0, "channel_error_max": 2},
                "samples": samples}
    files["matrix.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    files = generate()
    for name, data in files.items():
        path = ROOT / name
        if args.write:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        elif not path.is_file() or path.read_bytes() != data:
            raise SystemExit(f"Fixture differs: {path}; inspect changes before --write")
    print(f"{'Wrote' if args.write else 'Verified'} {len(files)} deterministic files")


if __name__ == "__main__":
    main()
