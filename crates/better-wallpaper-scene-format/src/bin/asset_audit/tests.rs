use super::*;
use std::os::unix::fs::symlink;

fn args() -> Args {
    Args::parse_from(["asset-audit"])
}

fn source(name: &str, path: &Path) -> String {
    format!("{name}={}", path.display())
}

fn texture(mips: u32) -> Vec<u8> {
    let mut bytes = b"TEXV0005\0TEXI0001\0".to_vec();
    for value in [0_u32, 0, 1, 1, 1, 1, 0] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(b"TEXB0002\0");
    for value in [1_u32, mips] {
        bytes.extend(value.to_le_bytes());
    }
    for _ in 0..mips {
        for value in [1_u32, 1, 0, 0, 4] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend([17, 34, 51, 255]);
    }
    bytes
}

fn package(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = 8_u32.to_le_bytes().to_vec();
    bytes.extend(b"PKGV0001");
    bytes.extend((files.len() as u32).to_le_bytes());
    let mut offset = 0_u32;
    for (name, data) in files {
        bytes.extend((name.len() as u32).to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend(offset.to_le_bytes());
        bytes.extend((data.len() as u32).to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in files {
        bytes.extend(*data);
    }
    bytes
}

#[test]
fn inventories_hashes_formats_and_exact_reference_chain_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    let tex = texture(1);
    let bytes = package(&[
        ("scene.json", br#"{"objects":[{"image":"models/a.json"}]}"#),
        ("models/a.json", br#"{"material":"materials/a.json"}"#),
        ("materials/a.json", br#"{"passes":[{"textures":["a"]}]}"#),
        ("materials/a.tex", &tex),
        ("opaque.txt", b"abc"),
    ]);
    let pkg = dir.path().join("scene.pkg");
    fs::write(&pkg, &bytes).unwrap();
    let mut input = args();
    input.pkg.push(source("sample", &pkg));
    let audit = run(input).unwrap();
    assert!(audit.complete);
    assert_eq!(audit.records.len(), 6);
    assert_eq!(
        audit.records[0].sha256.as_deref(),
        Some(hash(&bytes).as_str())
    );
    let opaque = audit
        .records
        .iter()
        .find(|r| r.path == "opaque.txt")
        .unwrap();
    assert_eq!(
        opaque.sha256.as_deref(),
        Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    for record in audit.records.iter().filter(|r| !r.path.is_empty()) {
        let offset = record.format.as_ref().unwrap()["archive_offset"]
            .as_u64()
            .unwrap() as usize;
        assert_eq!(
            record.sha256.as_deref(),
            Some(hash(&bytes[offset..offset + record.size as usize]).as_str())
        );
    }
    assert_eq!(audit.summary["tex:validated_2d"], 1);
    assert_eq!(audit.references.len(), 3);
    assert!(
        audit
            .references
            .iter()
            .all(|r| r.status == "candidate_found" && r.matches.len() == 1)
    );
    assert!(
        audit
            .references
            .iter()
            .any(|r| r.pointer == "/passes/0/textures/0"
                && r.candidate.as_deref() == Some("materials/a.tex"))
    );
    let mut input = args();
    input.pkg.push(source("sample", &pkg));
    assert_eq!(
        serde_json::to_value(&audit).unwrap(),
        serde_json::to_value(run(input).unwrap()).unwrap()
    );
}

#[test]
fn loose_projects_use_nearest_root_and_only_explicit_shared_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    for sub in ["project/models", "project/materials"] {
        fs::create_dir_all(dir.path().join(sub)).unwrap();
    }
    fs::create_dir(shared.path().join("materials")).unwrap();
    fs::write(
        dir.path().join("project/project.json"),
        br#"{"file":"scene.json"}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("project/scene.json"),
        br#"{"image":"models/a.json"}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("project/models/a.json"),
        br#"{"material":"materials/a.json"}"#,
    )
    .unwrap();
    fs::write(dir.path().join("project/materials/a.json"), br#"{"textures":["shared","missing","../secret","_rt_frame"],"z/~":{"video":"https://invalid/video"}}"#).unwrap();
    fs::write(shared.path().join("materials/shared.tex"), texture(1)).unwrap();
    let sibling = dir.path().join("other.pkg");
    fs::write(&sibling, package(&[("materials/missing.tex", &texture(1))])).unwrap();
    let mut input = args();
    input.root.push(source("projects", dir.path()));
    input.shared.push(source("common", shared.path()));
    input.pkg.push(source("other", &sibling));
    let audit = run(input).unwrap();
    assert!(audit.complete);
    assert_eq!(audit.excluded_loose_files, 1);
    let references = &audit.references;
    assert!(references.iter().any(|r| r.value == "models/a.json"
        && r.matches == [json!({"source":"projects", "path":"project/models/a.json"})]));
    assert!(references.iter().any(|r| r.value == "shared"
        && r.matches == [json!({"source":"common", "path":"materials/shared.tex"})]));
    assert!(
        references
            .iter()
            .any(|r| r.value == "missing" && r.status == "not_found_in_scope")
    );
    assert_eq!(
        references
            .iter()
            .filter(|r| r.status == "non_file_or_unsafe")
            .count(),
        3
    );
    assert!(references.iter().any(|r| r.pointer == "/z~1~0/video"));
}

#[test]
fn classifies_volume_malformed_and_invalid_payload_without_losing_hashes() {
    let dir = tempfile::tempdir().unwrap();
    let mut volume = texture(1);
    volume[22..26].copy_from_slice(&64_u32.to_le_bytes());
    let mut bad_payload = texture(1);
    bad_payload[79..83].copy_from_slice(&3_u32.to_le_bytes());
    bad_payload.pop();
    fs::write(dir.path().join("volume.tex"), volume).unwrap();
    fs::write(dir.path().join("bad.tex"), b"bad").unwrap();
    fs::write(dir.path().join("pixels.tex"), bad_payload).unwrap();
    let mut input = args();
    input.root.push(source("loose", dir.path()));
    let audit = run(input).unwrap();
    assert!(audit.complete); // All files inventoried, even when their content is rejected.
    assert!(audit.records.iter().all(|r| r.sha256.is_some()));
    assert_eq!(audit.summary["failure:unsupported_volume"], 1);
    assert_eq!(audit.summary["failure:malformed_texture"], 1);
    assert_eq!(audit.summary["failure:invalid_pixel_payload"], 1);
    assert_eq!(
        audit
            .records
            .iter()
            .find(|r| r.path == "volume.tex")
            .unwrap()
            .format
            .as_ref()
            .unwrap()["flags"],
        64
    );
}

#[test]
fn aggregate_mip_budget_is_enforced_without_changing_default_parser() {
    let bytes = texture(2);
    assert_eq!(TexTexture::parse(&bytes).unwrap().mipmaps.len(), 2);
    assert!(TexTexture::parse_with_budget(&bytes, 8).is_ok());
    assert!(matches!(
        TexTexture::parse_with_budget(&bytes, 7),
        Err(TexError::MipmapTooLarge { size: 8, max: 7 })
    ));
}

#[test]
fn record_traversal_depth_and_byte_limits_mark_incomplete_coverage() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("nested/deeper")).unwrap();
    for name in ["a.tex", "b.tex", "nested/deeper/c.tex"] {
        fs::write(dir.path().join(name), texture(1)).unwrap();
    }
    for limit in ["records", "walk", "depth", "file", "total"] {
        let mut input = args();
        input.root.push(source("limited", dir.path()));
        match limit {
            "records" => input.limits.max_records = 1,
            "walk" => input.limits.max_walk_entries = 1,
            "depth" => input.limits.max_depth = 1,
            "file" => input.limits.max_file_bytes = 8,
            "total" => input.limits.max_total_bytes = texture(1).len() as u64,
            _ => unreachable!(),
        }
        let audit = run(input).unwrap();
        assert!(!audit.complete, "{limit}");
        assert!(audit.records.len() <= audit.limits.max_records as usize);
        assert!(audit.walk_entries <= audit.limits.max_walk_entries);
        assert!(audit.input_bytes <= audit.limits.max_total_bytes);
        if limit == "walk" {
            assert!(audit.records.is_empty());
        }
    }
}

#[test]
fn symlinks_and_bad_packages_never_load_external_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.tex"), texture(1)).unwrap();
    symlink(
        outside.path().join("secret.tex"),
        dir.path().join("escape.tex"),
    )
    .unwrap();
    symlink(outside.path(), dir.path().join("escape-dir")).unwrap();
    symlink(
        outside.path().join("secret.tex"),
        dir.path().join("escape.pkg"),
    )
    .unwrap();
    fs::write(dir.path().join("bad.pkg"), b"PKG").unwrap();
    let mut input = args();
    input.root.push(source("root", dir.path()));
    input
        .root
        .push(source("symlink-root", &dir.path().join("escape-dir")));
    input
        .pkg
        .push(source("escape", &dir.path().join("escape.pkg")));
    input.pkg.push(source("bad", &dir.path().join("bad.pkg")));
    let audit = run(input).unwrap();
    assert!(!audit.complete);
    assert_eq!(audit.input_bytes, 3);
    assert_eq!(audit.records.len(), 2);
    assert_eq!(audit.summary["failure:package_rejected"], 1);
    assert_eq!(audit.summary["failure:non_regular_or_symlink"], 1);
}

#[test]
fn reference_and_json_limits_remain_visible() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.json"), br#"{"textures":["a","b"]}"#).unwrap();
    fs::write(dir.path().join("b.json"), b"{").unwrap();
    fs::write(dir.path().join("c.json"), vec![b' '; 2 * 1024 * 1024 + 1]).unwrap();
    let mut input = args();
    input.root.push(source("root", dir.path()));
    input.limits.max_references = 1;
    let audit = run(input).unwrap();
    assert!(!audit.complete);
    assert_eq!(audit.references.len(), 1);
    assert!(
        audit
            .notices
            .iter()
            .any(|n| n["class"] == "reference_limit")
    );
    assert_eq!(audit.summary["failure:invalid_json"], 1);
    assert_eq!(audit.summary["failure:json_byte_limit"], 1);
}

#[test]
fn invalid_source_selection_is_rejected() {
    assert!(run(args()).is_err());
    let mut input = args();
    input.root = vec!["same=/tmp".into(), "same=/tmp".into()];
    assert!(run(input).is_err());
}
