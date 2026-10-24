use std::process::Command;

const NEUTRAL: &[u8] = include_bytes!("../../../tests/fixtures/volume-textures/neutral-png.tex");

#[test]
fn volume_inspection_reports_structure_without_playback_admission() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("lut.tex");
    std::fs::write(&input, NEUTRAL).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scene-inspect"))
        .arg(&input)
        .args(["--volume", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["storage_size"], serde_json::json!([4, 4, 4]));
    assert_eq!(report["logical_size"], serde_json::json!([16, 4]));
    assert_eq!(report["format"], "RGBA8888");
    assert_eq!(report["mips"][0]["depth"], 4);
    assert_eq!(report["mips"][0]["decoded_bytes"], 256);
    assert_eq!(report["payload_status"], "validated_volume");
    assert_eq!(report["scene_playback_available"], false);
    assert_eq!(report["native_visual_validation"], "pending");
    for args in [vec!["--json"], vec!["--atlas", "--json"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_scene-inspect"))
            .arg(&input)
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Volume textures are not supported")
        );
    }
}

#[test]
fn volume_inspection_rejects_bad_input_and_conflicting_modes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("bad.tex");
    std::fs::write(&input, &NEUTRAL[..98]).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scene-inspect"))
        .arg(&input)
        .args(["--volume", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("volume validation failed"));
    let output = Command::new(env!("CARGO_BIN_EXE_scene-inspect"))
        .arg(dir.path().join("scene.pkg"))
        .arg("--volume")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires an unpacked .tex file"));
    for extra in ["--atlas", "--textures", "--entry"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_scene-inspect"));
        command.arg(&input).args(["--volume", extra]);
        if extra == "--entry" {
            command.arg("scene.json");
        }
        assert!(!command.output().unwrap().status.success());
    }
    // The input cap is checked before parsing; a sparse file avoids allocating
    // a test payload of this size.
    std::fs::File::create(&input)
        .unwrap()
        .set_len(128 * 1024 * 1024 + 1)
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scene-inspect"))
        .arg(&input)
        .args(["--volume", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("volume input exceeds 128 MiB"));
}

#[test]
fn volume_fixtures_match_recorded_hashes() {
    use sha2::{Digest, Sha256};
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/volume-textures/manifest.json"
    ))
    .unwrap();
    for (index, data) in [
        NEUTRAL,
        include_bytes!("../../../tests/fixtures/volume-textures/noncubic-raw.tex").as_slice(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(manifest["assets"][index]["bytes"], data.len());
        assert_eq!(
            manifest["assets"][index]["sha256"],
            format!("{:x}", Sha256::digest(data))
        );
    }
}
