use super::*;

const NEUTRAL: &[u8] = include_bytes!("../../../tests/fixtures/volume-textures/neutral-png.tex");
const RAW: &[u8] = include_bytes!("../../../tests/fixtures/volume-textures/noncubic-raw.tex");

fn set_word(data: &mut [u8], offset: usize, value: u32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn words(data: &mut Vec<u8>, values: &[u32]) {
    for value in values {
        data.extend(value.to_le_bytes());
    }
}

fn raw_volume(version: u32, compressed: bool) -> Vec<u8> {
    let mut data = RAW[..50].to_vec();
    data.extend(format!("TEXB{version:04}\0").as_bytes());
    words(&mut data, &[1]);
    if version >= 3 {
        words(&mut data, &[u32::MAX]);
    }
    if version == 4 {
        words(&mut data, &[0]);
    }
    words(&mut data, &[3]);
    let mut offset = 67;
    for _ in 0..3 {
        let dimensions = [
            read_u32_le(RAW, &mut offset).unwrap(),
            read_u32_le(RAW, &mut offset).unwrap(),
            read_u32_le(RAW, &mut offset).unwrap(),
        ];
        let size = read_u32_le(RAW, &mut offset).unwrap();
        let pixels = get_slice(RAW, &mut offset, size as usize).unwrap();
        words(&mut data, &dimensions);
        let encoded = if compressed {
            lz4_flex::compress(pixels)
        } else {
            pixels.to_vec()
        };
        if version >= 2 {
            words(&mut data, &[u32::from(compressed), size]);
        }
        words(&mut data, &[encoded.len() as u32]);
        data.extend(encoded);
    }
    data
}

#[test]
fn volume_png_preserves_neutral_cube_axes_and_source_metadata() {
    let volume = TexVolume::parse(NEUTRAL).unwrap();
    assert_eq!(volume.format, TexFormat::RGBA8888);
    assert_eq!((volume.width, volume.height), (16, 4));
    assert_eq!(
        (volume.texture_width, volume.texture_height, volume.depth),
        (4, 4, 4)
    );
    assert_eq!(volume.info_extra, 0xFF123456);
    let mip = &volume.mipmaps[0];
    for z in 0..4 {
        for y in 0..4 {
            for x in 0..4 {
                let index = ((z * 4 + y) * 4 + x) * 4;
                assert_eq!(
                    &mip.data[index..index + 4],
                    &[x as u8 * 85, y as u8 * 85, z as u8 * 85, 255]
                );
            }
        }
    }
    // Inspection must not change either 2D parser's playback boundary.
    for error in [
        TexTexture::parse(NEUTRAL).unwrap_err(),
        TexAtlas::parse(NEUTRAL).unwrap_err(),
    ] {
        assert!(
            error
                .to_string()
                .contains("Volume textures are not supported")
        );
    }
}

#[test]
fn volume_raw_versions_and_lz4_preserve_independent_mip_depths_and_bytes() {
    for version in 1..=4 {
        for compressed in [false, true] {
            if version == 1 && compressed {
                continue;
            }
            let volume = TexVolume::parse(&raw_volume(version, compressed)).unwrap();
            assert_eq!(volume.format, TexFormat::ARGB8888);
            assert_eq!(volume.container_version, version);
            for (level, (mip, dimensions)) in volume
                .mipmaps
                .iter()
                .zip([[5, 3, 2], [2, 1, 1], [1, 1, 1]])
                .enumerate()
            {
                assert_eq!([mip.width, mip.height, mip.depth], dimensions);
                for z in 0..mip.depth {
                    for y in 0..mip.height {
                        for x in 0..mip.width {
                            let index = (((z * mip.height + y) * mip.width + x) * 4) as usize;
                            assert_eq!(
                                &mip.data[index..index + 4],
                                &[10 + x as u8, 40 + y as u8, 70 + z as u8, 200 + level as u8]
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn volume_png_accepts_texb3_and_lz4_wrapped_payloads() {
    for version in [3, 4] {
        for compressed in [false, true] {
            let mut data = NEUTRAL[..59].to_vec();
            data[57] = b'0' + version;
            words(&mut data, &[1, 13]);
            if version == 4 {
                words(&mut data, &[0]);
            }
            words(
                &mut data,
                &[
                    1,
                    4,
                    4,
                    4,
                    u32::from(compressed),
                    (NEUTRAL.len() - 99) as u32,
                ],
            );
            let encoded = if compressed {
                lz4_flex::compress(&NEUTRAL[99..])
            } else {
                NEUTRAL[99..].to_vec()
            };
            words(&mut data, &[encoded.len() as u32]);
            data.extend(encoded);
            assert_eq!(
                TexVolume::parse(&data).unwrap().mipmaps[0].data,
                TexVolume::parse(NEUTRAL).unwrap().mipmaps[0].data
            );
        }
    }
}

#[test]
fn volume_png_validates_each_mip_and_preserves_alpha() {
    let mut data = NEUTRAL[..99].to_vec();
    set_word(&mut data, 71, 3);
    data.extend(&NEUTRAL[99..]);
    for dimension in [2, 1] {
        let pixels = [23, 67, 119, 151].repeat((dimension * dimension * dimension) as usize);
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, dimension, dimension * dimension);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
        words(
            &mut data,
            &[dimension, dimension, dimension, 0, 0, encoded.len() as u32],
        );
        data.extend(encoded);
    }
    let volume = TexVolume::parse_with_budget(&data, 292).unwrap();
    assert_eq!(volume.mipmaps.len(), 3);
    assert!(
        volume.mipmaps[1]
            .data
            .chunks_exact(4)
            .all(|pixel| pixel == [23, 67, 119, 151])
    );
    assert_eq!(&*volume.mipmaps[2].data, &[23, 67, 119, 151]);
    assert!(matches!(
        TexVolume::parse_with_budget(&data, 291),
        Err(TexError::MipmapTooLarge {
            size: 292,
            max: 291
        })
    ));
}

#[test]
fn volume_rejects_every_truncated_prefix_and_trailing_data() {
    for data in [NEUTRAL, RAW, &raw_volume(2, true)] {
        for end in 0..data.len() {
            assert!(
                TexVolume::parse(&data[..end]).is_err(),
                "accepted prefix {end}"
            );
        }
        let mut extra = data.to_vec();
        extra.push(0);
        assert!(
            TexVolume::parse(&extra)
                .unwrap_err()
                .to_string()
                .contains("trailing data")
        );
    }
}

#[test]
fn volume_checks_dimensions_counts_versions_formats_and_compression() {
    for (offset, value) in [
        (18, 7),
        (18, u32::MAX), // Unsupported BCn and unknown format.
        (22, 0),
        (22, 0x44),
        (22, 0x60), // Non-volume, animation, video.
        (26, 0),
        (30, 8193),
        (34, 0),
        (38, 16385),
        (42, 0),
        (42, 129),
        (59, 0),
        (59, 2),
        (59, u32::MAX),
        (63, 2),
        (67, 1), // Images/codec/video.
        (71, 0),
        (71, 33),
        (71, 4), // Mips beyond the complete chain.
        (75, 0),
        (79, 0),
        (83, 0),
        (83, 2),
        (87, 2),
        (95, 0),
        (95, u32::MAX),
    ] {
        let mut bad = NEUTRAL.to_vec();
        set_word(&mut bad, offset, value);
        assert!(
            TexVolume::parse(&bad).is_err(),
            "accepted offset {offset} = {value}"
        );
    }
    for offset in [0, 9, 50] {
        let mut bad = NEUTRAL.to_vec();
        bad[offset] = b'X';
        assert!(TexVolume::parse(&bad).is_err());
    }
    // The second mip may not borrow base dimensions or retain the old depth.
    let mut bad = RAW.to_vec();
    set_word(&mut bad, 211, 2);
    assert!(TexVolume::parse(&bad).is_err());
}

#[test]
fn volume_budget_counts_all_depth_slices_and_the_entire_mip_chain() {
    assert!(TexVolume::parse_with_budget(NEUTRAL, 256).is_ok());
    assert!(matches!(
        TexVolume::parse_with_budget(NEUTRAL, 255),
        Err(TexError::MipmapTooLarge {
            size: 256,
            max: 255
        })
    ));
    assert!(TexVolume::parse_with_budget(RAW, 132).is_ok());
    assert!(matches!(
        TexVolume::parse_with_budget(RAW, 131),
        Err(TexError::MipmapTooLarge {
            size: 132,
            max: 131
        })
    ));
    // A corrupt payload must not be decoded once the declared dimensions exceed
    // the caller's budget (including a zero budget).
    assert!(matches!(
        TexVolume::parse_with_budget(&NEUTRAL[..87], 0),
        Err(TexError::MipmapTooLarge { max: 0, .. })
    ));
    let mut huge = NEUTRAL.to_vec();
    set_word(&mut huge, 26, 8192);
    set_word(&mut huge, 30, 8192);
    set_word(&mut huge, 42, 128);
    assert!(matches!(
        TexVolume::parse_with_budget(&huge, u64::MAX),
        Err(TexError::MipmapTooLarge {
            max: MAX_MIPMAP_SIZE,
            ..
        })
    ));
}

#[test]
fn volume_rejects_png_shape_mismatch_and_broken_lz4() {
    let mut bad = NEUTRAL.to_vec();
    // Equal voxel count, different flattening: PNG is 4x16, not 8x8.
    for (offset, value) in [(26, 8), (30, 2), (75, 8), (79, 2)] {
        set_word(&mut bad, offset, value);
    }
    assert!(
        TexVolume::parse(&bad)
            .unwrap_err()
            .to_string()
            .contains("Invalid mipmap data size")
    );
    let mut bad = raw_volume(2, true);
    // First raw payload has 120 bytes; declaration mismatch fails before decode.
    set_word(&mut bad, 83, 121);
    assert!(matches!(
        TexVolume::parse(&bad),
        Err(TexError::InvalidMipmapDataSize { .. })
    ));
    let mut bad = raw_volume(2, true);
    set_word(&mut bad, 87, 1);
    bad[91] = 0xf0; // Truncated LZ4 literal length.
    assert!(TexVolume::parse(&bad).is_err());

    let mut short = raw_volume(2, true);
    let encoded = lz4_flex::compress(&[0; 119]);
    set_word(&mut short, 87, encoded.len() as u32);
    short.truncate(91);
    short.extend(encoded);
    assert!(
        TexVolume::parse(&short)
            .unwrap_err()
            .to_string()
            .contains("decoded 119 bytes, expected 120")
    );

    let mut bad = NEUTRAL.to_vec();
    set_word(&mut bad, 87, 1);
    set_word(&mut bad, 91, 1000);
    set_word(&mut bad, 95, 1);
    assert!(matches!(
        TexVolume::parse(&bad),
        Err(TexError::CompressionRatioExceeded { .. })
    ));
}
