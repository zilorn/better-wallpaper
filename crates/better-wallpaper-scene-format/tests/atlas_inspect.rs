use better_wallpaper_scene_format::PkgReader;
use std::process::Command;

#[test]
fn atlas_inspection_distinguishes_structure_from_current_playback_layout() {
    let package = PkgReader::parse(
        include_bytes!("../../../tests/fixtures/scene-behavior/multi-image/scene.pkg").as_slice(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("atlas.tex");
    std::fs::write(&input, package.read_entry(package.find_by_ext("tex")[0])).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_scene-inspect"))
        .arg(&input)
        .args(["--atlas", "--json"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["images"].as_array().unwrap().len(), 2);
    assert_eq!(report["images"][0]["mips"].as_array().unwrap().len(), 2);
    assert_eq!(report["images"][1]["mips"][0]["width"], 1);
    assert_eq!(report["frames"][1]["image_index"], 1);
    assert_eq!(report["single_image_layout"]["accepted"], false);
    assert_eq!(report["sprite_layout"]["accepted"], true);
    assert_eq!(report["payload_status"], "validated_2d_images");
    assert_eq!(report["native_visual_validation"], "pending");

    // The default CLI entry point retains the legacy single-image adapter.
    let legacy = Command::new(env!("CARGO_BIN_EXE_scene-inspect"))
        .arg(&input)
        .arg("--json")
        .output()
        .unwrap();
    assert!(!legacy.status.success());
    assert!(String::from_utf8_lossy(&legacy.stderr).contains("multiple atlas images"));
}

#[test]
fn atlas_inspection_rejects_truncation_and_wrong_input_kind() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("bad.tex");
    std::fs::write(&input, b"TEXV0005\0").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scene-inspect"))
        .arg(&input)
        .args(["--atlas", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("atlas validation failed"));

    let output = Command::new(env!("CARGO_BIN_EXE_scene-inspect"))
        .arg(dir.path().join("scene.pkg"))
        .arg("--atlas")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires an unpacked .tex file"));
}
