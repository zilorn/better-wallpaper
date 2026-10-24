//! Synthetic sampling probes; expectations come from authored solid colors.
use super::*;
use better_wallpaper_scene_format::{TexAtlas, TextureImage};

type Mip = (u32, u32, Vec<u8>);

fn words(bytes: &mut Vec<u8>, values: &[u32]) {
    for value in values {
        bytes.extend(value.to_le_bytes());
    }
}

// TEXB2 / TEXS2, uncompressed RGBA, with independent image/mip chains.
fn atlas(images: &[Vec<Mip>], frames: &[(u32, [f32; 6])]) -> TexAtlas {
    let (width, height, _) = images[0][0];
    let mut bytes = b"TEXV0005\0TEXI0001\0".to_vec();
    words(&mut bytes, &[0, 4, width, height, width, height, 0]);
    bytes.extend(b"TEXB0002\0");
    words(&mut bytes, &[images.len() as u32]);
    for image in images {
        words(&mut bytes, &[image.len() as u32]);
        for (width, height, rgba) in image {
            words(
                &mut bytes,
                &[*width, *height, 0, rgba.len() as u32, rgba.len() as u32],
            );
            bytes.extend(rgba);
        }
    }
    bytes.extend(b"TEXS0002\0");
    words(&mut bytes, &[frames.len() as u32]);
    for (image, basis) in frames {
        words(&mut bytes, &[*image]);
        bytes.extend(0.5_f32.to_le_bytes());
        for value in basis {
            bytes.extend(value.to_le_bytes());
        }
    }
    TexAtlas::parse(&bytes).unwrap()
}

fn assets(atlas: &TexAtlas) -> Scene2dAssets {
    let matrix = matrix();
    let sample = matrix
        .samples
        .iter()
        .find(|sample| sample.id == "multi-image")
        .unwrap();
    let mut assets = sample_assets(sample).unwrap();
    let mut images = atlas.to_texture_images().unwrap().into_iter();
    let draw = &mut assets.draws[0];
    draw.texture = images.next().unwrap();
    draw.additional_textures = images.collect::<Vec<TextureImage>>();
    draw.animation = atlas.sprite_animation().unwrap();
    assets
}

fn solid(context: &Offscreen, size: u32, expected: [u8; 4]) {
    let actual = context.pixels(size, size);
    let expected = expected.repeat((size * size) as usize);
    let score = metrics(&actual, &expected, size as usize, size as usize);
    println!("Atlas sampling {size}x{size}: {score:?}");
    assert!(passes(&score), "{score:?}");
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; run explicitly with software rendering"]
fn packed_frame_minification_excludes_neighboring_frames() {
    let context = Offscreen::new();
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let pixels = [green.repeat(7), red.repeat(32), green.repeat(25)]
        .concat()
        .repeat(32);
    let atlas = atlas(
        &[vec![(64, 32, pixels)]],
        &[
            (0, [7., 0., 32., 0., 0., 32.]),
            (0, [39., 0., 0., 32., -32., 0.]),
            (0, [39., 0., -32., 0., 0., 32.]),
            (0, [7., 32., 32., 0., 0., -32.]),
            (0, [39., 0., 25., 0., 0., 32.]),
        ],
    );
    let assets = assets(&atlas);
    let mut renderer = SceneGpuRenderer::new(context.gl(), 32, 32).unwrap();
    renderer.upload_scene_textures(&assets).unwrap();
    for size in [32, 8, 4, 1] {
        renderer.resize(size, size);
        for (time, color) in [
            (0., red),
            (0.5, red),
            (1., red),
            (1.5, red),
            (2., green),
            (2.5, red),
        ] {
            renderer.draw_scene(&assets, time).unwrap();
            solid(&context, size, color);
        }
    }
}

fn mip_atlas() -> TexAtlas {
    let mip =
        |width, height, color: [u8; 4]| (width, height, color.repeat((width * height) as usize));
    atlas(
        &[
            vec![mip(64, 64, [255, 0, 0, 255]), mip(32, 32, [0, 0, 255, 255])],
            vec![
                mip(32, 64, [255, 255, 0, 255]),
                mip(16, 32, [0, 255, 0, 255]),
                mip(8, 16, [255, 0, 255, 255]),
            ],
        ],
        &[
            (0, [0., 0., 64., 0., 0., 64.]),
            (0, [7., 9., 32., 0., 0., 32.]),
            (0, [64., 0., 0., 64., -64., 0.]),
            (1, [32., 0., -32., 0., 0., 64.]),
        ],
    )
}

#[test]
fn sampling_probes_preserve_independent_mips_and_classify_frame_coverage() {
    let atlas = mip_atlas();
    assert_eq!(
        atlas
            .images
            .iter()
            .map(|i| i.mipmaps.len())
            .collect::<Vec<_>>(),
        [2, 3]
    );
    let animation = atlas.sprite_animation().unwrap().unwrap();
    assert_eq!(
        animation
            .frames
            .iter()
            .map(|f| f.full_image)
            .collect::<Vec<_>>(),
        [true, false, true, true]
    );
    let mut skewed = atlas;
    skewed.frames.truncate(1);
    skewed.frames[0].horizontal = [32., 0.];
    skewed.frames[0].vertical = [32., 64.];
    assert!(!skewed.sprite_animation().unwrap().unwrap().frames[0].full_image);
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; run explicitly with software rendering"]
fn independent_authored_mips_survive_crop_transitions_and_reload() {
    let mut context = Some(Offscreen::new());
    let assets = assets(&mip_atlas());
    let mut renderer = Some(SceneGpuRenderer::new(context.as_ref().unwrap().gl(), 8, 8).unwrap());
    for lifecycle in 0..3 {
        if lifecycle == 1 {
            renderer.as_mut().unwrap().clear_textures();
        } else if lifecycle == 2 {
            drop(renderer.take());
            drop(context.take());
            context = Some(Offscreen::new());
            renderer = Some(SceneGpuRenderer::new(context.as_ref().unwrap().gl(), 8, 8).unwrap());
        }
        let renderer = renderer.as_mut().unwrap();
        renderer.upload_scene_textures(&assets).unwrap();
        for size in [64, 8, 1] {
            renderer.resize(size, size);
            for time in [0., 0.5, 1., 1.5, 1.5, 2.] {
                renderer.draw_scene(&assets, time).unwrap();
                // Level zero has a different color from every retained tail.
                // All smaller mips are generated from the last authored mip,
                // so no GPU-derived reference or LOD rounding is needed.
                let color = if time == 0.5 || (size == 64 && time != 1.5) {
                    [255, 0, 0, 255]
                } else if time == 1.5 {
                    if size == 64 {
                        [255, 255, 0, 255]
                    } else {
                        [255, 0, 255, 255]
                    }
                } else {
                    [0, 0, 255, 255]
                };
                solid(context.as_ref().unwrap(), size, color);
            }
        }
    }
    drop(renderer);
}
