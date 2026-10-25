use super::*;

fn words(bytes: &mut Vec<u8>, values: &[u32]) {
    for value in values {
        bytes.extend(value.to_le_bytes());
    }
}

fn tex(version: u32, video: bool, metadata: &[&str], compressed: bool) -> Vec<u8> {
    let mut bytes = b"TEXV0005\0TEXI0001\0".to_vec();
    words(&mut bytes, &[0, if video { 32 } else { 0 }, 1, 1, 1, 1, 0]);
    bytes.extend(format!("TEXB{version:04}\0").bytes());
    words(&mut bytes, &[1]);
    if version >= 3 {
        words(&mut bytes, &[u32::MAX]);
    }
    if version == 4 {
        words(&mut bytes, &[metadata.len() as u32]);
        for (id, json) in metadata.iter().enumerate() {
            words(&mut bytes, &[id as u32, 100 + id as u32, 200 + id as u32]);
            bytes.extend(json.as_bytes());
            bytes.push(0);
        }
    }
    words(&mut bytes, &[1, 1, 1]);
    let payload: &[u8] = if video {
        b"encoded-media"
    } else {
        &[23, 67, 119, 255]
    };
    let encoded = if compressed {
        lz4_flex::compress(payload)
    } else {
        payload.to_vec()
    };
    if version >= 2 {
        words(&mut bytes, &[u32::from(compressed), payload.len() as u32]);
    }
    words(&mut bytes, &[encoded.len() as u32]);
    bytes.extend(encoded);
    if !metadata.is_empty() {
        words(&mut bytes, &[0]);
    }
    bytes
}

#[test]
fn texb4_metadata_count_is_independent_of_video_and_precedes_images() {
    for metadata in [vec![], vec![r#"{"label":"owned"}"#], vec!["{}", "{}"]] {
        for video in [false, true] {
            for compressed in [false, true] {
                let bytes = tex(4, video, &metadata, compressed);
                let atlas = TexAtlas::parse(&bytes).unwrap();
                assert_eq!(atlas.is_video, video);
                assert_eq!(atlas.editor_metadata.len(), metadata.len());
                for (id, record) in atlas.editor_metadata.iter().enumerate() {
                    assert_eq!(record.fields, [id as u32, 100 + id as u32, 200 + id as u32]);
                    assert_eq!(record.json, metadata[id]);
                }
                let single = atlas.try_into_single_texture().unwrap();
                assert_eq!(single.editor_metadata.len(), metadata.len());
                assert_eq!(
                    &*single.mipmaps[0].data,
                    if video {
                        &b"encoded-media"[..]
                    } else {
                        &[23, 67, 119, 255]
                    }
                );
                if video {
                    assert!(single.to_texture_image().is_err());
                }
            }
        }
    }
}

#[test]
fn texb4_video_versions_compression_and_every_truncated_prefix() {
    for version in 1..=4 {
        for compressed in [false, true] {
            if version == 1 && compressed {
                continue;
            }
            let bytes = tex(version, true, &[], compressed);
            for end in 0..bytes.len() {
                assert!(
                    TexAtlas::parse(&bytes[..end]).is_err(),
                    "version {version}, prefix {end}"
                );
                assert!(
                    TexTexture::parse(&bytes[..end]).is_err(),
                    "legacy version {version}, prefix {end}"
                );
            }
            assert!(TexAtlas::parse(&bytes).unwrap().is_video);
        }
    }
    let bytes = tex(4, false, &["{}", "{\"label\":\"héllo\"}"], true);
    for end in 0..bytes.len() {
        assert!(
            TexAtlas::parse(&bytes[..end]).is_err(),
            "metadata prefix {end}"
        );
    }
}

fn rejects_both(bytes: &[u8]) {
    assert!(TexAtlas::parse(bytes).is_err());
    assert!(TexTexture::parse(bytes).is_err());
}

#[test]
fn texb4_rejects_malformed_counts_strings_lengths_and_media_layouts() {
    let base = tex(4, true, &[], false);
    for (offset, values) in [
        (22, vec![36, 96]), // animation/volume cannot masquerade as media
        (26, vec![0, MAX_TEXTURE_DIMENSION + 1]),
        (55, vec![0, 2, u32::MAX]),
        (59, vec![13, 2]), // encoded image codecs are not media containers
        (63, vec![MAX_EDITOR_RECORDS + 1, u32::MAX]),
        (67, vec![0, 2, u32::MAX]),
        (71, vec![0, MAX_TEXTURE_DIMENSION + 1]),
        (75, vec![0, MAX_TEXTURE_DIMENSION + 1]),
        (79, vec![2, u32::MAX]),
        (87, vec![0, 64 * 1024 * 1024 + 1, u32::MAX]),
    ] {
        for value in values {
            let mut damaged = base.clone();
            damaged[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            rejects_both(&damaged);
        }
    }
    let mut trailing = base;
    trailing.push(0);
    rejects_both(&trailing);
    for bad in ["{", "[]", "{\"a\":NaN}"] {
        rejects_both(&tex(4, false, &[bad], false));
    }
    let mut invalid_utf8 = tex(4, false, &["{}"], false);
    invalid_utf8[79] = 0xff;
    rejects_both(&invalid_utf8);
    let mut unterminated = tex(4, false, &["{}"], false);
    unterminated.truncate(81);
    rejects_both(&unterminated);
    let mut variants = tex(4, false, &["{}"], false);
    let end = variants.len();
    variants[end - 4..].copy_from_slice(&1_u32.to_le_bytes());
    for parse in [
        TexAtlas::parse(&variants).map(|_| ()),
        TexTexture::parse(&variants).map(|_| ()),
    ] {
        assert!(parse.unwrap_err().to_string().contains("variants"));
    }
}

#[test]
fn texb4_editor_and_media_budgets_fail_before_payload_allocation() {
    let json = format!(
        "{{\"label\":\"{}\"}}",
        "x".repeat(MAX_EDITOR_STRING_BYTES - 12)
    );
    assert_eq!(json.len(), MAX_EDITOR_STRING_BYTES);
    assert!(TexAtlas::parse(&tex(4, false, &[&json], false)).is_ok());
    rejects_both(&tex(4, false, &[&(json + " ")], false));
    let json = format!("{{\"label\":\"{}\"}}", "x".repeat(65500));
    rejects_both(&tex(4, false, &[json.as_str(); 17], false));
    for compressed in [false, true] {
        let bytes = tex(4, true, &[], compressed);
        assert!(TexAtlas::parse_with_budget(&bytes, 12).is_err());
        assert!(TexTexture::parse_with_budget(&bytes, 12).is_err());
        assert!(TexAtlas::parse_with_budget(&bytes, 13).is_ok());
    }
    let mut lz4 = tex(4, true, &[], true);
    // Exact decoded length, malformed compressed bytes, ratio and size ceilings.
    for expected in [12_u32, 14, 64 * 1024 * 1024, 64 * 1024 * 1024 + 1, u32::MAX] {
        lz4[83..87].copy_from_slice(&expected.to_le_bytes());
        rejects_both(&lz4);
    }
    let mut lz4 = tex(4, true, &[], true);
    lz4[91..].fill(0xff);
    rejects_both(&lz4);
}

#[test]
fn texb4_owned_fixtures_preserve_metadata_and_exact_media_bytes() {
    let media = include_bytes!("../../../tests/fixtures/scene-video.mp4");
    for (data, video, count) in [
        (
            &include_bytes!("../../../tests/fixtures/texb4/video.tex")[..],
            true,
            0,
        ),
        (
            &include_bytes!("../../../tests/fixtures/texb4/video-metadata.tex")[..],
            true,
            2,
        ),
        (
            &include_bytes!("../../../tests/fixtures/texb4/static-metadata.tex")[..],
            false,
            2,
        ),
    ] {
        let atlas = TexAtlas::parse(data).unwrap();
        assert_eq!(atlas.is_video, video);
        assert_eq!(atlas.editor_metadata.len(), count);
        assert_eq!(
            &*atlas.images[0].mipmaps[0].data,
            if video {
                &media[..]
            } else {
                &[23, 67, 119, 255]
            }
        );
        for end in 0..data.len() {
            rejects_both(&data[..end]);
        }
    }
}

#[test]
fn texb4_static_adapter_rejects_malformed_mips_as_the_atlas_parser_does() {
    let base = tex(4, false, &[], false);
    for (offset, value) in [(67, 0_u32), (67, 33), (79, 2), (87, 0)] {
        let mut damaged = base.clone();
        damaged[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        rejects_both(&damaged);
    }
    let mut lz4 = tex(4, false, &[], true);
    lz4[83..87].copy_from_slice(&8_u32.to_le_bytes());
    rejects_both(&lz4);
}
