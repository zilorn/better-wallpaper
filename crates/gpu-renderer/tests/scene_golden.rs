//! Run GPU cases explicitly on a host with surfaceless EGL/GLES:
//! LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1
use better_wallpaper_core::{DecodeOptions, VideoDecoder};
use better_wallpaper_ffmpeg::FfmpegDecoder;
use better_wallpaper_renderer::{
    Scene2dAssets, Scene2dOptions, SceneVideoFrame, build_scene_2d_plan, resolve_scene_2d_assets,
};
use better_wallpaper_scene_format::{PkgReader, TexTexture, parse_scene_graph};
use glow::HasContext;
use gpu_renderer::SceneGpuRenderer;
use khronos_egl as egl;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Matrix {
    schema_version: u32,
    provenance: String,
    pointer_coordinates: String,
    randomness: String,
    reference_kind: String,
    gate: Gate,
    samples: Vec<Sample>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Gate {
    ssim_min: f64,
    rgba_mse_max: f64,
    channel_error_max: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sample {
    id: String,
    purpose: String,
    artifacts: BTreeMap<String, Artifact>,
    canvas: [u32; 2],
    properties: BTreeMap<String, serde_json::Value>,
    random_seed: Option<u64>,
    native_reference: String,
    steps: Vec<Step>,
    outcome: Outcome,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    path: String,
    bytes: usize,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Outcome {
    Render { draws: usize, skipped_nodes: usize },
    RejectTexture { error: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    time_seconds: f64,
    viewport: [u32; 2],
    pointer: Option<[f32; 2]>,
    video_action: Option<VideoAction>,
    recreate_context: bool,
    reference: String,
    reference_sha256: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum VideoAction {
    DecodeNext,
    ReplaceSolid { size: [u32; 2], rgba: [u8; 4] },
}

fn fixture_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/scene-behavior")
}

fn checked_artifact(artifact: &Artifact) -> Vec<u8> {
    assert!(
        Path::new(&artifact.path)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
    );
    let data = std::fs::read(fixture_root().join(&artifact.path)).unwrap();
    assert_eq!(data.len(), artifact.bytes, "{}", artifact.path);
    assert_eq!(
        format!("{:x}", Sha256::digest(&data)),
        artifact.sha256,
        "{}",
        artifact.path
    );
    data
}

fn matrix() -> Matrix {
    let matrix: Matrix =
        serde_json::from_slice(&std::fs::read(fixture_root().join("matrix.json")).unwrap())
            .unwrap();
    assert_eq!(matrix.schema_version, 1);
    assert_eq!(matrix.provenance, "independently_authored_synthetic");
    assert_eq!(
        matrix.pointer_coordinates,
        "normalized_y_up_minus_one_to_one"
    );
    assert_eq!(
        matrix.randomness,
        "unused; null seed means no random operators"
    );
    assert_eq!(
        matrix.reference_kind,
        "analytical_png_not_wallpaper_engine_capture"
    );
    // Keep recorded thresholds identical to the independently tested comparator.
    assert_eq!(
        (
            matrix.gate.ssim_min,
            matrix.gate.rgba_mse_max,
            matrix.gate.channel_error_max
        ),
        (0.995, 1.0, 2)
    );
    let mut ids = BTreeSet::new();
    for sample in &matrix.samples {
        assert!(ids.insert(&sample.id), "duplicate sample {}", sample.id);
        assert!(!sample.purpose.is_empty());
        assert_eq!(sample.native_reference, "pending");
        assert!(sample.properties.is_empty() && sample.random_seed.is_none());
        assert_eq!(sample.artifacts.len(), 2);
        assert!(sample.canvas.iter().all(|v| (1..=64).contains(v)));
        for filename in ["scene.pkg", "project.json"] {
            assert_eq!(
                sample.artifacts[filename].path,
                format!("{}/{filename}", sample.id)
            );
        }
        let project: serde_json::Value =
            serde_json::from_slice(&checked_artifact(&sample.artifacts["project.json"])).unwrap();
        assert_eq!(project["type"], "scene");
        assert_eq!(project["file"], "scene.json");
        for step in &sample.steps {
            assert!(step.time_seconds.is_finite() && step.time_seconds >= 0.0);
            assert!(step.viewport.iter().all(|v| (1..=64).contains(v)));
            if let Some(pointer) = step.pointer {
                assert!(
                    pointer
                        .iter()
                        .all(|v| v.is_finite() && (-1.0..=1.0).contains(v))
                );
            }
            if let Some(VideoAction::ReplaceSolid { size, .. }) = &step.video_action {
                assert!(size.iter().all(|v| (1..=64).contains(v)));
            }
            assert!(
                step.reference
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            );
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/golden")
                .join(format!("{}.png", step.reference));
            let data = std::fs::read(path).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(&data)),
                step.reference_sha256
            );
            let reader = png::Decoder::new(std::io::Cursor::new(data))
                .read_info()
                .unwrap();
            assert_eq!([reader.info().width, reader.info().height], step.viewport);
            assert_eq!(reader.info().color_type, png::ColorType::Rgba);
        }
    }
    assert_eq!(matrix.samples.len(), 13);
    assert_eq!(
        matrix.samples.iter().map(|s| s.steps.len()).sum::<usize>(),
        20
    );
    matrix
}

fn sample_assets(
    sample: &Sample,
) -> Result<Scene2dAssets, better_wallpaper_renderer::Scene2dError> {
    let pkg = PkgReader::parse(checked_artifact(&sample.artifacts["scene.pkg"])).unwrap();
    let scene = pkg
        .read_entry_string(pkg.find("scene.json").unwrap())
        .unwrap();
    let graph = parse_scene_graph(&scene).unwrap();
    assert!(
        graph.unsupported_features.is_empty(),
        "{}: {:?}",
        sample.id,
        graph.unsupported_features
    );
    let plan = build_scene_2d_plan(
        &graph,
        Scene2dOptions {
            viewport_width: sample.canvas[0],
            viewport_height: sample.canvas[1],
        },
    )
    .unwrap();
    resolve_scene_2d_assets(&pkg, plan)
}

#[test]
fn committed_behavior_samples_match_declared_outcomes_and_hashes() {
    for sample in matrix().samples {
        match &sample.outcome {
            Outcome::Render {
                draws,
                skipped_nodes,
            } => {
                let assets = sample_assets(&sample).unwrap();
                assert_eq!(assets.draws.len(), *draws, "{}", sample.id);
                assert_eq!(assets.skipped_nodes, *skipped_nodes, "{}", sample.id);
                assert!(!sample.steps.is_empty());
            }
            Outcome::RejectTexture { error } => {
                let pkg =
                    PkgReader::parse(checked_artifact(&sample.artifacts["scene.pkg"])).unwrap();
                let textures = pkg.find_by_ext("tex");
                assert_eq!(textures.len(), 1);
                let actual = TexTexture::parse(pkg.read_entry(textures[0])).unwrap_err();
                assert!(
                    actual.to_string().contains(error),
                    "{}: {actual}",
                    sample.id
                );
                // Asset resolution intentionally retains supported scene nodes.
                // These single-image samples must skip the failed texture draw.
                let assets = sample_assets(&sample).unwrap();
                assert!(assets.draws.is_empty(), "{}", sample.id);
                assert_eq!(assets.skipped_nodes, 1, "{}", sample.id);
                assert!(sample.steps.is_empty());
            }
        }
    }
}

#[test]
#[ignore = "requires surfaceless EGL/GLES; run explicitly with software rendering"]
fn gpu_scene_goldens() {
    let matrix = matrix();
    let mut context = Some(Offscreen::new());
    for sample in matrix.samples {
        let Outcome::Render { draws, .. } = sample.outcome else {
            continue;
        };
        println!("Sample: {}", sample.id);
        let assets = sample_assets(&sample).unwrap();
        assert_eq!(assets.draws.len(), draws);
        let video = assets.draws.iter().find_map(|draw| draw.video.as_ref());
        let mut media = tempfile::NamedTempFile::new().unwrap();
        let mut decoder = FfmpegDecoder::for_package_media();
        if let Some(video) = video {
            media.write_all(&video.encoded).unwrap();
            decoder
                .open(media.path(), DecodeOptions::default())
                .unwrap();
        }
        let mut generation = 0;
        let mut renderer = Some(
            SceneGpuRenderer::new(
                context.as_ref().unwrap().gl(),
                sample.canvas[0],
                sample.canvas[1],
            )
            .unwrap(),
        );
        renderer
            .as_mut()
            .unwrap()
            .upload_scene_textures(&assets)
            .unwrap();
        for step in sample.steps {
            if let Some(action) = step.video_action {
                generation += 1;
                let frame = match action {
                    VideoAction::DecodeNext => {
                        let frame = decoder.next_frame().unwrap();
                        SceneVideoFrame {
                            rgba: Arc::from(frame.pixels),
                            width: frame.width,
                            height: frame.height,
                            generation,
                        }
                    }
                    VideoAction::ReplaceSolid { size, rgba } => SceneVideoFrame {
                        rgba: Arc::from(rgba.repeat((size[0] * size[1]) as usize)),
                        width: size[0],
                        height: size[1],
                        generation,
                    },
                };
                *video
                    .expect("video action requires a video asset")
                    .frame
                    .write()
                    .unwrap() = Some(frame);
            }
            if step.recreate_context {
                drop(renderer.take());
                drop(context.take());
                context = Some(Offscreen::new());
                let mut replacement = SceneGpuRenderer::new(
                    context.as_ref().unwrap().gl(),
                    step.viewport[0],
                    step.viewport[1],
                )
                .unwrap();
                replacement.upload_scene_textures(&assets).unwrap();
                renderer = Some(replacement);
            }
            let renderer = renderer.as_mut().unwrap();
            renderer.resize(step.viewport[0], step.viewport[1]);
            renderer
                .draw_scene_with_pointer(&assets, step.time_seconds, step.pointer)
                .unwrap();
            compare(
                &step.reference,
                &context
                    .as_ref()
                    .unwrap()
                    .pixels(step.viewport[0], step.viewport[1]),
                step.viewport[0],
                step.viewport[1],
            );
        }
        // The owning EGL context is still current during resource deletion.
        drop(renderer);
    }
}
