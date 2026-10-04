//! Run GPU cases explicitly on a host with surfaceless EGL/GLES:
//! LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1
use better_wallpaper_core::{DecodeOptions, VideoDecoder};
use better_wallpaper_ffmpeg::FfmpegDecoder;
use better_wallpaper_renderer::{
    Scene2dAssets, Scene2dOptions, SceneVideoFrame, build_scene_2d_plan, resolve_scene_2d_assets,
};
use better_wallpaper_scene_format::{PkgReader, parse_scene_graph};
use glow::HasContext;
use gpu_renderer::SceneGpuRenderer;
use khronos_egl as egl;
use serde_json::{Value, json};
use std::{io::Write, path::Path, sync::Arc};

struct Offscreen {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    surface: egl::Surface,
    context: egl::Context,
}

impl Offscreen {
    fn new() -> Self {
        // EGL_MESA_platform_surfaceless accepts a null native display and does
        // not connect to the user's desktop or create a visible surface.
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
            .expect("EGL library required for explicit GPU goldens");
        let display =
            unsafe { egl.get_platform_display(0x31DD, std::ptr::null_mut(), &[egl::ATTRIB_NONE]) }
                .expect("surfaceless EGL platform unavailable");
        egl.initialize(display).unwrap();
        egl.bind_api(egl::OPENGL_ES_API).unwrap();
        let config = egl
            .choose_first_config(
                display,
                &[
                    egl::SURFACE_TYPE,
                    egl::PBUFFER_BIT,
                    egl::RED_SIZE,
                    8,
                    egl::GREEN_SIZE,
                    8,
                    egl::BLUE_SIZE,
                    8,
                    egl::ALPHA_SIZE,
                    8,
                    egl::RENDERABLE_TYPE,
                    egl::OPENGL_ES2_BIT,
                    egl::NONE,
                ],
            )
            .unwrap()
            .unwrap();
        let context = egl
            .create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE],
            )
            .unwrap();
        let surface = egl
            .create_pbuffer_surface(
                display,
                config,
                &[egl::WIDTH, 64, egl::HEIGHT, 64, egl::NONE],
            )
            .unwrap();
        egl.make_current(display, Some(surface), Some(surface), Some(context))
            .unwrap();
        let this = Self {
            egl,
            display,
            surface,
            context,
        };
        let gl = this.gl();
        unsafe {
            gl.disable(glow::DITHER);
        }
        println!("Golden renderer: {}", unsafe {
            gl.get_parameter_string(glow::RENDERER)
        });
        this
    }

    fn gl(&self) -> glow::Context {
        unsafe {
            glow::Context::from_loader_function(|name| {
                self.egl
                    .get_proc_address(name)
                    .map_or(std::ptr::null(), |proc| proc as *const _)
            })
        }
    }

    fn pixels(&self, width: u32, height: u32) -> Vec<u8> {
        let gl = self.gl();
        let stride = width as usize * 4;
        let mut pixels = vec![0; stride * height as usize];
        unsafe {
            gl.finish();
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(
                0,
                0,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(&mut pixels),
            );
            assert_eq!(gl.get_error(), glow::NO_ERROR);
        }
        // OpenGL readback is bottom-up; committed PNG references are top-down.
        for y in 0..height as usize / 2 {
            let opposite = height as usize - 1 - y;
            let (first, rest) = pixels.split_at_mut(opposite * stride);
            first[y * stride..(y + 1) * stride].swap_with_slice(&mut rest[..stride]);
        }
        pixels
    }
}

impl Drop for Offscreen {
    fn drop(&mut self) {
        self.egl
            .make_current(self.display, None, None, None)
            .unwrap();
        self.egl
            .destroy_surface(self.display, self.surface)
            .unwrap();
        self.egl
            .destroy_context(self.display, self.context)
            .unwrap();
        self.egl.terminate(self.display).unwrap();
    }
}

fn package(entries: Vec<(String, Vec<u8>)>) -> PkgReader {
    let string = |s: &str| {
        let mut b = (s.len() as u32).to_le_bytes().to_vec();
        b.extend(s.as_bytes());
        b
    };
    let mut bytes = string("PKGV0001");
    bytes.extend((entries.len() as u32).to_le_bytes());
    let mut offset = 0_u32;
    for (name, data) in &entries {
        bytes.extend(string(name));
        bytes.extend(offset.to_le_bytes());
        bytes.extend((data.len() as u32).to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in entries {
        bytes.extend(data);
    }
    PkgReader::parse(bytes).unwrap()
}

fn texture(width: u32, height: u32, storage_width: u32, rgba: Vec<u8>) -> Vec<u8> {
    let mut bytes = b"TEXV0005\0TEXI0001\0".to_vec();
    for word in [0_u32, 0, storage_width, height, width, height, 0] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(b"TEXB0002\0");
    for word in [
        1_u32,
        1,
        storage_width,
        height,
        0,
        rgba.len() as u32,
        rgba.len() as u32,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(rgba);
    bytes
}

fn assets(scene: Value, textures: Vec<(&str, Vec<u8>, &str)>) -> Scene2dAssets {
    let mut entries = vec![("scene.json".into(), scene.to_string().into_bytes())];
    for (name, texture, blend) in textures {
        entries.push((
            format!("models/{name}.json"),
            json!({"material":format!("materials/{name}.json")})
                .to_string()
                .into_bytes(),
        ));
        entries.push((
            format!("materials/{name}.json"),
            json!({"passes":[{"shader":"genericimage4","blending":blend,"textures":[name]}]})
                .to_string()
                .into_bytes(),
        ));
        entries.push((format!("materials/{name}.tex"), texture));
    }
    let pkg = package(entries);
    let graph = parse_scene_graph(&scene.to_string()).unwrap();
    assert!(
        graph.unsupported_features.is_empty(),
        "{:?}",
        graph.unsupported_features
    );
    let plan = build_scene_2d_plan(
        &graph,
        Scene2dOptions {
            viewport_width: 32,
            viewport_height: 32,
        },
    )
    .unwrap();
    resolve_scene_2d_assets(&pkg, plan).unwrap()
}

fn layers_scene() -> Value {
    json!({"general":{"orthogonalprojection":{"width":32,"height":32},"cameraparallax":true,
        "cameraparallaxamount":0.25,"cameraparallaxdelay":0,"cameraparallaxmouseinfluence":1},"objects":[
        {"id":"bg","image":"models/bg.json","origin":"16 16 0","size":"32 32","parallaxDepth":"0 0"},
        {"id":"parent","container":true,"origin":"4 0 0"},
        {"id":"fg","parent":"parent","image":"models/fg.json","origin":"8 20 0","size":"8 12","angles":"0 0 1.5707963267948966","alpha":0.5,"parallaxDepth":"1 0"},
        {"id":"add","image":"models/add.json","origin":"24 8 0","size":"8 8","alpha":0.5,"parallaxDepth":"0 1"}
    ]})
}

fn layers(scene: Value) -> Scene2dAssets {
    assets(
        scene,
        vec![
            ("bg", texture(1, 1, 1, vec![24, 40, 80, 255]), "opaque"),
            (
                "fg",
                texture(1, 1, 1, vec![200, 80, 32, 128]),
                "translucent",
            ),
            ("add", texture(1, 1, 1, vec![32, 48, 16, 255]), "additive"),
        ],
    )
}

#[derive(Debug)]
struct Metrics {
    ssim: f64,
    mse: f64,
    max_error: u8,
}

fn metrics(actual: &[u8], expected: &[u8], width: usize, height: usize) -> Metrics {
    assert_eq!(actual.len(), width * height * 4);
    assert_eq!(actual.len(), expected.len());
    let mse = actual
        .iter()
        .zip(expected)
        .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum::<f64>()
        / actual.len() as f64;
    let max_error = actual
        .iter()
        .zip(expected)
        .map(|(&a, &b)| a.abs_diff(b))
        .max()
        .unwrap();
    let luminance = |pixels: &[u8], x: usize, y: usize| {
        let i = (y * width + x) * 4;
        0.2126 * f64::from(pixels[i])
            + 0.7152 * f64::from(pixels[i + 1])
            + 0.0722 * f64::from(pixels[i + 2])
    };
    let mut scores = Vec::new();
    // Local 8x8 windows detect shifted/blurred edges; global SSIM can hide them.
    for y0 in (0..height).step_by(4) {
        for x0 in (0..width).step_by(4) {
            let mut pairs = Vec::new();
            for y in y0..(y0 + 8).min(height) {
                for x in x0..(x0 + 8).min(width) {
                    pairs.push((luminance(actual, x, y), luminance(expected, x, y)));
                }
            }
            let n = pairs.len() as f64;
            let ma = pairs.iter().map(|p| p.0).sum::<f64>() / n;
            let mb = pairs.iter().map(|p| p.1).sum::<f64>() / n;
            let va = pairs.iter().map(|p| (p.0 - ma).powi(2)).sum::<f64>() / n;
            let vb = pairs.iter().map(|p| (p.1 - mb).powi(2)).sum::<f64>() / n;
            let cov = pairs.iter().map(|p| (p.0 - ma) * (p.1 - mb)).sum::<f64>() / n;
            let c1 = 2.55_f64.powi(2);
            let c2 = 7.65_f64.powi(2);
            scores.push(
                ((2.0 * ma * mb + c1) * (2.0 * cov + c2))
                    / ((ma * ma + mb * mb + c1) * (va + vb + c2)),
            );
        }
    }
    Metrics {
        ssim: scores.iter().sum::<f64>() / scores.len() as f64,
        mse,
        max_error,
    }
}

fn passes(metrics: &Metrics) -> bool {
    metrics.ssim >= 0.995 && metrics.mse <= 1.0 && metrics.max_error <= 2
}

fn write_png(path: &Path, width: u32, height: u32, pixels: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(pixels)
        .unwrap();
}

fn compare(name: &str, actual: &[u8], width: u32, height: u32) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.png"));
    let mut reader = png::Decoder::new(std::fs::File::open(path).unwrap())
        .read_info()
        .unwrap();
    let mut expected = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut expected).unwrap();
    assert_eq!(
        (info.width, info.height, info.color_type),
        (width, height, png::ColorType::Rgba)
    );
    let score = metrics(actual, &expected, width as usize, height as usize);
    println!("{name}: {score:?}");
    if !passes(&score) {
        let output =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scene-golden-failures");
        std::fs::create_dir_all(&output).unwrap();
        write_png(
            &output.join(format!("{name}-actual.png")),
            width,
            height,
            actual,
        );
        let diff = actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .flat_map(|(a, b)| {
                [
                    a[0].abs_diff(b[0]).saturating_mul(4),
                    a[1].abs_diff(b[1]).saturating_mul(4),
                    a[2].abs_diff(b[2]).saturating_mul(4),
                    255,
                ]
            })
            .collect::<Vec<_>>();
        write_png(
            &output.join(format!("{name}-diff.png")),
            width,
            height,
            &diff,
        );
        panic!(
            "{name} failed SSIM >= 0.995, MSE <= 1, max error <= 2: {score:?}; artifacts: {}",
            output.display()
        );
    }
}

#[test]
fn ssim_and_pixel_gates_reject_channel_errors_and_local_damage() {
    let pixels = vec![30_u8; 16 * 16 * 4];
    assert!(passes(&metrics(&pixels, &pixels, 16, 16)));
    let mut damaged = pixels.clone();
    damaged[0] = 255;
    assert!(!passes(&metrics(&damaged, &pixels, 16, 16)));
    let mut shifted = pixels.clone();
    for y in 0..8 {
        for x in 0..8 {
            shifted[(y * 16 + x) * 4 + 1] = 200;
        }
    }
    assert!(metrics(&shifted, &pixels, 16, 16).ssim < 0.995);
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; run explicitly with software rendering"]
fn gpu_scene_goldens() {
    let context = Offscreen::new();
    let mut renderer = SceneGpuRenderer::new(context.gl(), 32, 32).unwrap();
    let layered = layers(layers_scene());
    assert_eq!(layered.draws.len(), 3);
    renderer.upload_scene_textures(&layered).unwrap();
    renderer.draw_scene(&layered, 0.0).unwrap();
    compare("layers", &context.pixels(32, 32), 32, 32);
    renderer.resize(64, 32);
    renderer.draw_scene(&layered, 0.0).unwrap();
    compare("cover", &context.pixels(64, 32), 64, 32);
    renderer.resize(32, 32);
    renderer
        .draw_scene_with_pointer(&layered, 1.0, Some([1.0, 1.0]))
        .unwrap();
    compare("parallax", &context.pixels(32, 32), 32, 32);
    // A paused scene must retain the prior pointer transform.
    renderer
        .draw_scene_with_pointer(&layered, 1.0, Some([-1.0, -1.0]))
        .unwrap();
    compare("parallax", &context.pixels(32, 32), 32, 32);
    drop(renderer);
    let mut scene = layers_scene();
    scene["objects"][2]["alpha"] = json!({"value":0.5,"animation":{"c0":[{"frame":0,"value":0},{"frame":60,"value":1}],"options":{"fps":30,"length":60,"mode":"single"}}});
    let animated = layers(scene);
    let mut renderer = SceneGpuRenderer::new(context.gl(), 32, 32).unwrap();
    renderer.upload_scene_textures(&animated).unwrap();
    renderer.draw_scene(&animated, 2.0).unwrap();
    compare("opacity-end", &context.pixels(32, 32), 32, 32);
    drop(renderer);
    let mut scene = layers_scene();
    scene["general"]["zoom"] = json!({"value":1,"animation":{"c0":[{"frame":0,"value":1},{"frame":60,"value":2}],"options":{"fps":30,"length":60,"mode":"single"}}});
    let zoomed = layers(scene);
    let mut renderer = SceneGpuRenderer::new(context.gl(), 32, 32).unwrap();
    renderer.upload_scene_textures(&zoomed).unwrap();
    renderer.draw_scene(&zoomed, 2.0).unwrap();
    compare("zoom-end", &context.pixels(32, 32), 32, 32);
    drop(renderer);
    let mut atlas = Vec::new();
    for _ in 0..32 {
        atlas.extend([255, 0, 0, 255].repeat(32));
        atlas.extend([0, 255, 0, 255].repeat(32));
    }
    let mut tex = texture(64, 32, 64, atlas);
    tex[22..26].copy_from_slice(&4_u32.to_le_bytes()); // TEX animated flag.
    tex.extend(b"TEXS0002\0");
    tex.extend(2_u32.to_le_bytes());
    for frame in 0..2_u32 {
        tex.extend(frame.to_le_bytes());
        for value in [0.5_f32, frame as f32 * 32.0, 0.0, 32.0, 32.0, 0.0, 0.0] {
            tex.extend(value.to_le_bytes());
        }
    }
    let scene = json!({"general":{"orthogonalprojection":{"width":32,"height":32}},"objects":[{"image":"models/sprite.json","origin":"16 16 0","size":"32 32"}]});
    let sprites = assets(scene, vec![("sprite", tex, "opaque")]);
    assert_eq!(sprites.draws[0].animation.as_ref().unwrap().frames.len(), 2);
    let mut renderer = SceneGpuRenderer::new(context.gl(), 32, 32).unwrap();
    renderer.upload_scene_textures(&sprites).unwrap();
    for (time, name) in [
        (0.0, "sprite-red"),
        (0.5, "sprite-green"),
        (1.0, "sprite-red"),
    ] {
        renderer.draw_scene(&sprites, time).unwrap();
        compare(name, &context.pixels(32, 32), 32, 32);
    }
    drop(renderer);
    let mut rgba = vec![0; 64 * 32 * 4];
    for y in 0..32 {
        for x in 0..64 {
            let color = if x >= 32 {
                [255, 0, 255, 255]
            } else if y < 16 {
                if x < 16 {
                    [180, 20, 40, 255]
                } else {
                    [20, 160, 60, 255]
                }
            } else if x < 16 {
                [30, 50, 170, 255]
            } else {
                [150, 120, 10, 255]
            };
            rgba[(y * 64 + x) * 4..(y * 64 + x + 1) * 4].copy_from_slice(&color);
        }
    }
    let scene = json!({"general":{"orthogonalprojection":{"width":32,"height":32}},"objects":[{"image":"models/uv.json","origin":"16 16 0","size":"32 32"}]});
    let padded = assets(scene, vec![("uv", texture(32, 32, 64, rgba), "opaque")]);
    let mut renderer = SceneGpuRenderer::new(context.gl(), 32, 32).unwrap();
    renderer.upload_scene_textures(&padded).unwrap();
    renderer.draw_scene(&padded, 0.0).unwrap();
    compare("uv-padding", &context.pixels(32, 32), 32, 32);
    drop(renderer);
    // A real TEXB0004 video payload passes through package and scene resolution.
    let media = include_bytes!("../../../tests/fixtures/scene-video.mp4");
    let mut tex = b"TEXV0005\0TEXI0001\0".to_vec();
    for value in [0_u32, 32, 16, 16, 16, 16, 0] {
        tex.extend(value.to_le_bytes());
    }
    tex.extend(b"TEXB0004\0");
    for value in [1_u32, 0, 1, 1, 0, 0] {
        tex.extend(value.to_le_bytes());
    }
    tex.push(0);
    for value in [0_u32, 16, 16, 0, media.len() as u32, media.len() as u32] {
        tex.extend(value.to_le_bytes());
    }
    tex.extend(media);
    let scene = json!({"general":{"orthogonalprojection":{"width":32,"height":32}},"objects":[{"image":"models/video.json","origin":"16 16 0","size":"32 32"}]});
    let video_assets = assets(scene, vec![("video", tex, "opaque")]);
    assert_eq!(video_assets.draws.len(), 1);
    let video = video_assets.draws[0].video.as_ref().unwrap();
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&video.encoded).unwrap();
    let mut decoder = FfmpegDecoder::for_package_media();
    decoder.open(file.path(), DecodeOptions::default()).unwrap();
    let mut renderer = SceneGpuRenderer::new(context.gl(), 32, 32).unwrap();
    renderer.upload_scene_textures(&video_assets).unwrap();
    for (generation, name) in [(1, "video-red"), (2, "video-green")] {
        let frame = decoder.next_frame().unwrap();
        *video.frame.write().unwrap() = Some(SceneVideoFrame {
            rgba: Arc::from(frame.pixels),
            width: frame.width,
            height: frame.height,
            generation,
        });
        renderer
            .draw_scene(&video_assets, 0.5 * (generation - 1) as f64)
            .unwrap();
        compare(name, &context.pixels(32, 32), 32, 32);
    }
    // Force the storage-reallocation path, then recreate the owning GL context.
    *video.frame.write().unwrap() = Some(SceneVideoFrame {
        rgba: Arc::from([0, 255, 0, 255].repeat(64)),
        width: 8,
        height: 8,
        generation: 3,
    });
    renderer.draw_scene(&video_assets, 1.0).unwrap();
    compare("video-green", &context.pixels(32, 32), 32, 32);
    drop(renderer);
    drop(context);
    let replacement = Offscreen::new();
    let mut renderer = SceneGpuRenderer::new(replacement.gl(), 32, 32).unwrap();
    renderer.upload_scene_textures(&video_assets).unwrap();
    renderer.draw_scene(&video_assets, 1.0).unwrap();
    compare("video-green", &replacement.pixels(32, 32), 32, 32);
}
