use super::*;
use crate::PkgReader;

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/scene-behavior")
        .join(name)
        .join("scene.pkg");
    let pkg = PkgReader::parse(std::fs::read(path).unwrap()).unwrap();
    pkg.read_entry(pkg.find_by_ext("tex")[0]).to_vec()
}

#[test]
fn native_sprite_uvs_keep_authored_edges_and_shared_admission() {
    let mut atlas = TexAtlas::parse(&fixture("rotated")).unwrap();
    let native = atlas.sprite_animation_with_sampling(true).unwrap().unwrap();
    assert_eq!(native.frames[0].uv.at([0., 0.]), [1., 0.]);
    assert_eq!(native.frames[0].uv.at([1., 1.]), [0., 1.]);
    assert_ne!(
        native.frames[0].uv,
        atlas.sprite_animation().unwrap().unwrap().frames[0].uv
    );
    atlas.flags |= 3;
    let sampler = atlas
        .sprite_animation_with_sampling(true)
        .unwrap()
        .unwrap()
        .native_sampling
        .unwrap();
    assert!(sampler.clamp && sampler.nearest);
    atlas.frames[1].origin = [100., 0.];
    assert!(atlas.sprite_animation_with_sampling(true).is_err());
    assert!(atlas.sprite_animation().is_err());
}

#[test]
fn native_sprite_rejects_partial_authored_mips_instead_of_changing_filtering() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/atlas-native/packed-mips/scene.pkg");
    let pkg = PkgReader::parse(std::fs::read(path).unwrap()).unwrap();
    let mut atlas = TexAtlas::parse(pkg.read_entry(pkg.find_by_ext("tex")[0])).unwrap();
    assert!(atlas.sprite_animation_with_sampling(true).is_ok());
    atlas.images[0].mipmaps.pop();
    assert!(
        atlas
            .sprite_animation_with_sampling(true)
            .unwrap_err()
            .to_string()
            .contains("complete contiguous mip chains")
    );
    assert!(atlas.sprite_animation().is_ok());
}

fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}

fn multi_version(container: u32, animation: u32) -> Vec<u8> {
    let mut bytes = b"TEXV0005\0TEXI0001\0".to_vec();
    for value in [0, 4, 2, 2, 2, 2, 0] {
        word(&mut bytes, value);
    }
    bytes.extend(format!("TEXB{container:04}\0").bytes());
    word(&mut bytes, 2);
    if container >= 3 {
        word(&mut bytes, u32::MAX);
    }
    if container == 4 {
        word(&mut bytes, 0);
    }
    for width in [2, 1] {
        word(&mut bytes, 1);
        word(&mut bytes, width);
        word(&mut bytes, 2);
        if container >= 2 {
            word(&mut bytes, 0);
            word(&mut bytes, width * 8);
        }
        word(&mut bytes, width * 8);
        bytes.extend([10, 40, 200, 255].repeat(width as usize * 2));
    }
    bytes.extend(format!("TEXS{animation:04}\0").bytes());
    word(&mut bytes, 2);
    if animation == 3 {
        word(&mut bytes, 1);
        word(&mut bytes, 2);
    }
    for image in [0, 1] {
        word(&mut bytes, image);
        bytes.extend(0.5_f32.to_le_bytes());
        for coordinate in [2_i32, 0, 0, 2, -2, 0] {
            bytes.extend(if animation == 1 {
                coordinate.to_le_bytes()
            } else {
                (coordinate as f32).to_le_bytes()
            });
        }
    }
    bytes
}

#[test]
fn keeps_independent_mip_chains_and_frame_image_indices() {
    let atlas = TexAtlas::parse(&fixture("multi-image")).unwrap();
    let images = atlas.to_texture_images().unwrap();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].levels.len(), 2);
    assert_eq!(images[1].levels.len(), 1);
    assert_eq!(
        (images[0].levels[0].width, images[0].levels[0].height),
        (2, 2)
    );
    assert_eq!(
        (images[0].levels[1].width, images[0].levels[1].height),
        (1, 1)
    );
    assert_eq!(
        (images[1].levels[0].width, images[1].levels[0].height),
        (1, 2)
    );
    assert_eq!(images[0].levels[1].data.as_ref(), &[200, 40, 10, 255]);
    assert_eq!(
        images[1].levels[0].data.as_ref(),
        &[20, 180, 60, 255].repeat(2)
    );
    assert_eq!(atlas.frames[0].image_index, 0);
    assert_eq!(atlas.frames[1].image_index, 1);
    assert_eq!(atlas.frames[1].horizontal, [1.0, 0.0]);
    assert_eq!(atlas.frames[1].vertical, [0.0, 2.0]);
}

#[test]
fn preserves_candidate_bases_and_legacy_adapter_rejection() {
    for (name, horizontal, vertical) in [
        ("rotated", [0.0, 2.0], [-2.0, 0.0]),
        ("skewed", [1.0, 0.0], [1.0, 2.0]),
        ("flipped-x", [-2.0, 0.0], [0.0, 2.0]),
        ("flipped-y", [2.0, 0.0], [0.0, -2.0]),
    ] {
        let bytes = fixture(name);
        let atlas = TexAtlas::parse(&bytes).unwrap();
        assert_eq!(atlas.frames[0].horizontal, horizontal, "{name}");
        assert_eq!(atlas.frames[0].vertical, vertical, "{name}");
        assert_eq!(atlas.frames[0].duration_seconds, 0.5);
        assert_eq!(
            atlas.try_into_single_texture().unwrap_err().to_string(),
            TexTexture::parse(&bytes).unwrap_err().to_string()
        );
    }
    assert!(
        TexAtlas::parse(&fixture("multi-image"))
            .unwrap()
            .try_into_single_texture()
            .is_err()
    );
}

#[test]
fn sprite_admission_preserves_image_selection_orientation_and_paused_time() {
    let animation = TexAtlas::parse(&fixture("multi-image"))
        .unwrap()
        .sprite_animation()
        .unwrap()
        .unwrap();
    for (time, index) in [(0.0, 0), (0.499999, 0), (0.5, 1), (0.5, 1), (1.0, 0)] {
        assert_eq!(animation.frame_at(time).unwrap().image_index, index);
    }
    assert_eq!(animation.uv_at(0.5).at([0.0, 0.0]), [0.5, 0.25]);
    assert_eq!(animation.uv_at(0.5).at([1.0, 1.0]), [0.5, 0.75]);
    assert_eq!(animation.frame_at(f64::NAN).unwrap().image_index, 0);
    for (name, top_left, bottom_right) in [
        ("rotated", [0.75, 0.25], [0.25, 0.75]),
        ("flipped-x", [0.75, 0.25], [0.25, 0.75]),
        ("flipped-y", [0.25, 0.75], [0.75, 0.25]),
    ] {
        let animation = TexAtlas::parse(&fixture(name))
            .unwrap()
            .sprite_animation()
            .unwrap()
            .unwrap();
        assert_eq!(animation.uv_at(0.0).at([0.0, 0.0]), top_left);
        assert_eq!(animation.uv_at(0.0).at([1.0, 1.0]), bottom_right);
        assert_eq!(animation.uv_at(0.5).at([0.0, 0.0]), [0.25, 0.25]);
    }
    // Preserve the old rectangular half-texel inset exactly for each TEXS version.
    for version in 1..=3 {
        let animation = TexAtlas::parse(&fixture(&format!("sprite-v{version}")))
            .unwrap()
            .sprite_animation()
            .unwrap()
            .unwrap();
        assert_eq!(
            animation.uv_at(0.0).at([0.0, 0.0]),
            [0.5 / 64.0, 0.5 / 32.0]
        );
        assert_eq!(
            animation.uv_at(0.5).at([1.0, 1.0]),
            [63.5 / 64.0, 31.5 / 32.0]
        );
    }
}

#[test]
fn sprite_loops_preserve_every_frame_in_long_fractional_sequences() {
    let mut atlas = TexAtlas::parse(&fixture("multi-image")).unwrap();
    let source_frames = atlas.frames.clone();
    for durations in [&[0.1_f32][..], &[0.1_f32, 0.2, 0.3][..]] {
        atlas.frames = (0..10_000)
            .map(|index| TexAtlasFrame {
                duration_seconds: durations[index % durations.len()],
                ..source_frames[index % source_frames.len()].clone()
            })
            .collect();
        // Sum the exact stored f32 durations in the playback clock's precision.
        let mut boundaries = vec![0.0_f64];
        for frame in &atlas.frames {
            boundaries.push(boundaries.last().unwrap() + f64::from(frame.duration_seconds));
        }
        let period = *boundaries.last().unwrap();
        let animation = atlas.sprite_animation().unwrap().unwrap();
        for cycle in [0.0, 1.0, 28.0, 1_000.0, -1.0] {
            for index in [0, 1, 4_999, 9_998, 9_999] {
                for fraction in [0.0, 0.25, 0.75] {
                    let time = cycle * period
                        + boundaries[index]
                        + f64::from(atlas.frames[index].duration_seconds) * fraction;
                    assert!(
                        std::ptr::eq(animation.frame_at(time).unwrap(), &animation.frames[index]),
                        "wrong frame at cycle {cycle}, frame {index}, fraction {fraction}"
                    );
                }
            }
        }
    }
}

#[test]
fn sprite_admission_rejects_entire_invalid_sequences_without_static_fallback() {
    let atlas = TexAtlas::parse(&fixture("rotated")).unwrap();
    for frame in [
        TexAtlasFrame {
            duration_seconds: 0.0,
            ..atlas.frames[1].clone()
        },
        TexAtlasFrame {
            duration_seconds: -1.0,
            ..atlas.frames[1].clone()
        },
        TexAtlasFrame {
            origin: [-1.0, 0.0],
            ..atlas.frames[1].clone()
        },
        TexAtlasFrame {
            horizontal: [1.0, 1.0],
            vertical: [1.0, 1.0],
            ..atlas.frames[1].clone()
        },
        TexAtlasFrame {
            horizontal: [0.1, 0.0],
            ..atlas.frames[1].clone()
        },
        TexAtlasFrame {
            origin: [f32::NAN, 0.0],
            ..atlas.frames[1].clone()
        },
        TexAtlasFrame {
            image_index: 1,
            ..atlas.frames[1].clone()
        },
    ] {
        let mut invalid = atlas.clone();
        invalid.frames[1] = frame;
        assert!(invalid.sprite_animation().is_err());
    }
    let mut missing = atlas.clone();
    missing.frames.clear();
    assert!(missing.sprite_animation().is_err());
    let mut single = atlas.clone();
    single.frames.truncate(1);
    assert_eq!(single.sprite_animation().unwrap().unwrap().frames.len(), 1);
    let mut overflow = atlas;
    overflow
        .frames
        .iter_mut()
        .for_each(|frame| frame.duration_seconds = f32::MAX);
    assert!(overflow.sprite_animation().is_err());
}

#[test]
fn retains_signed_bases_and_texs3_dimensions_for_all_container_versions() {
    for container in 1..=4 {
        for animation in 1..=3 {
            let atlas = TexAtlas::parse(&multi_version(container, animation)).unwrap();
            assert_eq!(atlas.container_version, container);
            assert_eq!(atlas.images.len(), 2);
            assert_eq!(atlas.frames[1].image_index, 1);
            assert_eq!(atlas.frames[0].origin, [2.0, 0.0]);
            assert_eq!(atlas.frames[0].horizontal, [0.0, 2.0]);
            assert_eq!(atlas.frames[0].vertical, [-2.0, 0.0]);
            assert_eq!(atlas.frame_dimensions, (animation == 3).then_some([1, 2]));
        }
    }
}

#[test]
fn enforces_budget_across_images_not_per_image() {
    let bytes = fixture("multi-image");
    assert!(TexAtlas::parse_with_budget(&bytes, 28).is_ok());
    assert!(matches!(
        TexAtlas::parse_with_budget(&bytes, 27),
        Err(TexError::MipmapTooLarge { size: 28, max: 27 })
    ));
    assert!(matches!(
        TexAtlas::parse_with_budget(&bytes, 0),
        Err(TexError::MipmapTooLarge { .. })
    ));
    // A later image must not borrow the global TEXI storage size to accept
    // mismatched payload bytes (the second image is 1x2, not 2x2).
    let mut atlas = TexAtlas::parse(&bytes).unwrap();
    atlas.images[1].mipmaps[0].data = Arc::from([0_u8; 16]);
    assert!(atlas.to_texture_images().is_err());
}

#[test]
fn rejects_counts_missing_images_and_truncated_records() {
    let bytes = multi_version(2, 2);
    for count in [0, MAX_ATLAS_IMAGES + 1, u32::MAX] {
        let mut damaged = bytes.clone();
        damaged[55..59].copy_from_slice(&count.to_le_bytes());
        assert!(
            TexAtlas::parse(&damaged)
                .unwrap_err()
                .to_string()
                .contains("Atlas image count")
        );
    }
    for count in [0, MAX_ATLAS_MIPS_PER_IMAGE + 1, u32::MAX] {
        let mut damaged = bytes.clone();
        damaged[59..63].copy_from_slice(&count.to_le_bytes());
        assert!(
            TexAtlas::parse(&damaged)
                .unwrap_err()
                .to_string()
                .contains("Atlas mip count")
        );
    }
    for end in 0..bytes.len() {
        assert!(
            TexAtlas::parse(&bytes[..end]).is_err(),
            "truncation at {end}"
        );
    }
    let block = bytes.windows(9).position(|b| b == b"TEXS0002\0").unwrap();
    let mut damaged = bytes.clone();
    damaged[block + 13..block + 17].copy_from_slice(&2_u32.to_le_bytes());
    assert!(
        TexAtlas::parse(&damaged)
            .unwrap_err()
            .to_string()
            .contains("missing texture image")
    );
    damaged = bytes;
    damaged[block + 9..block + 13].copy_from_slice(&(MAX_ANIMATION_FRAMES + 1).to_le_bytes());
    assert!(
        TexAtlas::parse(&damaged)
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
}

#[test]
fn rejects_nonfinite_coordinates_and_invalid_duration_but_keeps_particle_zero() {
    let bytes = multi_version(2, 2);
    let block = bytes.windows(9).position(|b| b == b"TEXS0002\0").unwrap();
    for component in 0..7 {
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut damaged = bytes.clone();
            let start = block + 17 + component * 4;
            damaged[start..start + 4].copy_from_slice(&invalid.to_le_bytes());
            assert!(TexAtlas::parse(&damaged).is_err());
        }
    }
    let mut damaged = bytes.clone();
    damaged[block + 17..block + 21].copy_from_slice(&(-0.5_f32).to_le_bytes());
    assert!(TexAtlas::parse(&damaged).is_err());
    damaged[block + 17..block + 21].copy_from_slice(&0.0_f32.to_le_bytes());
    assert_eq!(
        TexAtlas::parse(&damaged).unwrap().frames[0].duration_seconds,
        0.0
    );
}

#[test]
fn validates_lz4_output_length_before_accepting_pixel_bytes() {
    let bytes = multi_version(2, 2);
    for decoded_len in [16, 12] {
        let compressed = lz4_flex::compress(&bytes[83..83 + decoded_len]);
        let mut encoded = bytes.clone();
        encoded[71..75].copy_from_slice(&1_u32.to_le_bytes());
        encoded[79..83].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
        encoded.splice(83..99, compressed);
        let result = TexAtlas::parse(&encoded);
        if decoded_len == 16 {
            let atlas = result.unwrap();
            assert_eq!(atlas.images[0].mipmaps[0].data.as_ref(), &bytes[83..99]);
            assert_eq!(atlas.frames[1].image_index, 1);
        } else {
            assert!(result.unwrap_err().to_string().contains("decoded 12 bytes"));
        }
    }
}

#[test]
fn validates_pixel_sizes_and_preserves_the_existing_video_boundary() {
    let bytes = multi_version(2, 2);
    let mut damaged = bytes.clone();
    damaged[63..67].copy_from_slice(&3_u32.to_le_bytes());
    assert!(matches!(
        TexAtlas::parse(&damaged),
        Err(TexError::InvalidMipmapDataSize { .. })
    ));
    damaged = bytes;
    damaged[71..75].copy_from_slice(&2_u32.to_le_bytes());
    assert!(
        TexAtlas::parse(&damaged)
            .unwrap_err()
            .to_string()
            .contains("compression")
    );
    let video = TexAtlas::parse(&fixture("video")).unwrap();
    assert!(video.is_video);
    assert!(video.to_texture_images().is_err());
    assert!(video.try_into_single_texture().is_ok());
    assert_eq!(
        TexAtlas::parse(&fixture("padding"))
            .unwrap()
            .to_texture_images()
            .unwrap()[0]
            .levels[0]
            .width,
        64
    );
}
