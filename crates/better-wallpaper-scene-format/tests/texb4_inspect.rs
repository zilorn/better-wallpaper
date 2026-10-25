use sha2::{Digest, Sha256};
use std::{path::Path, process::Command};

#[test]
fn inspection_reports_editor_records_separately_from_video_and_hashes_payloads() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/texb4");
    for (name, video) in [("video-metadata.tex", true), ("static-metadata.tex", false)] {
        let output = Command::new(env!("CARGO_BIN_EXE_scene-inspect"))
            .args([
                fixtures.join(name).as_os_str(),
                "--atlas".as_ref(),
                "--json".as_ref(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["is_video"], video);
        assert_eq!(json["editor_metadata"].as_array().unwrap().len(), 2);
        let bytes: &[u8] = if video {
            include_bytes!("../../../tests/fixtures/scene-video.mp4")
        } else {
            &[23, 67, 119, 255]
        };
        assert_eq!(
            json["images"][0]["mips"][0]["decoded_sha256"],
            format!("{:x}", Sha256::digest(bytes))
        );
        assert_eq!(
            json["payload_status"],
            if video {
                "encoded_video_requires_decoder"
            } else {
                "validated_2d_images"
            }
        );
        assert_eq!(json["native_visual_validation"], "pending");
    }
}
