use std::path::{Path, PathBuf};

use better_wallpaper_core::{DecodeOptions, DecodedFrame, PixelFormat, VideoDecoder, VideoError};
use better_wallpaper_ffmpeg::FfmpegDecoder;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn assert_frame_storage(frame: &DecodedFrame, size: (u32, u32), allow_cuda: bool) {
    assert_eq!((frame.width, frame.height), size);
    if let Some(cuda) = &frame.cuda {
        assert!(allow_cuda, "expected a CPU frame");
        assert!(frame.pixels.is_empty());
        assert_eq!(frame.stride, 0);
        assert!(cuda.planes.iter().all(|&plane| plane != 0));
        assert_ne!(cuda.planes[0], cuda.planes[1]);
        assert!(cuda.strides.iter().all(|&stride| stride >= size.0 as usize));
        assert!(
            cuda.retain_owner().is_some(),
            "CUDA frame must retain its owner"
        );
    } else {
        assert_eq!(frame.format, PixelFormat::Rgba);
        assert_eq!(frame.stride, size.0 as usize * 4);
        assert_eq!(frame.pixels.len(), frame.stride * size.1 as usize);
        assert!(frame.pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }
}

fn decode_fixture(name: &str, hardware: bool, max_height: u32, allow_cuda: bool) {
    let mut decoder = FfmpegDecoder::new();
    let media = decoder
        .open(
            &fixture(name),
            DecodeOptions {
                hardware,
                max_height,
            },
        )
        .expect("fixture should open");
    assert_eq!((media.width, media.height), (64, 36));

    let size = if max_height > 0 && max_height < 36 {
        (64 * max_height / 36, max_height)
    } else {
        (64, 36)
    };
    let mut frames = Vec::new();
    let mut first_frame = None;
    loop {
        match decoder.next_frame() {
            Ok(frame) => {
                assert_frame_storage(&frame, size, allow_cuda);
                assert_eq!(frame.time_base, media.time_base);
                frames.push(frame.pts);
                if first_frame.is_none() {
                    first_frame = Some(frame);
                }
            }
            Err(VideoError::EndOfStream) => break,
            Err(error) => panic!("fixture decode failed: {error}"),
        }
    }

    assert_eq!(frames.len(), 5);
    assert!(frames.windows(2).all(|pair| pair[0] < pair[1]));
    decoder.seek_start().expect("fixture should seek");
    let loop_frame = decoder.next_frame().expect("fixture should restart");
    assert_eq!(loop_frame.pts, frames[0]);
    assert_frame_storage(&loop_frame, size, allow_cuda);
    drop(loop_frame);
    drop(decoder);

    // A queued frame must keep its storage/context alive after seek and decoder teardown.
    let first_frame = first_frame.unwrap();
    assert_frame_storage(&first_frame, size, allow_cuda);
    eprintln!(
        "{name}: hardware={hardware}, max_height={max_height}, storage={}, frames={}",
        if first_frame.cuda.is_some() {
            "CUDA NV12"
        } else {
            "CPU RGBA"
        },
        frames.len()
    );
}

#[test]
fn decodes_h264_vp9_and_av1_fixtures() {
    for name in ["h264.mp4", "vp9.webm", "av1.mkv"] {
        decode_fixture(name, false, 0, false);
    }
}

#[test]
fn hardware_request_decodes_or_falls_back_to_software() {
    decode_fixture("h264.mp4", true, 0, true);
}

#[test]
fn hardware_request_without_cuda_falls_back_to_software() {
    const CHILD_ENV: &str = "BETTER_WALLPAPER_TEST_WITHOUT_CUDA";
    if std::env::var_os(CHILD_ENV).is_some() {
        decode_fixture("h264.mp4", true, 0, false);
        return;
    }

    // CUDA reads device visibility at initialization. Use a child process so parallel
    // tests neither initialize the driver first nor observe a changed environment.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "hardware_request_without_cuda_falls_back_to_software",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .env("CUDA_VISIBLE_DEVICES", "-1")
        .output()
        .expect("fallback test process should start");
    assert!(
        output.status.success(),
        "fallback test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn software_decode_respects_height_limit() {
    decode_fixture("h264.mp4", false, 18, false);
}

#[test]
fn hardware_request_respects_height_limit() {
    decode_fixture("h264.mp4", true, 18, false);
}

#[test]
fn height_limit_does_not_upscale() {
    decode_fixture("h264.mp4", false, 72, false);
    decode_fixture("h264.mp4", true, 72, true);
}

#[test]
fn seek_start_restarts_decoding() {
    let mut decoder = FfmpegDecoder::new();
    decoder
        .open(
            &fixture("h264.mp4"),
            DecodeOptions {
                hardware: false,
                max_height: 0,
            },
        )
        .unwrap();
    let first_pts = decoder.next_frame().unwrap().pts;

    while !matches!(decoder.next_frame(), Err(VideoError::EndOfStream)) {}
    decoder.seek_start().unwrap();

    assert_eq!(decoder.next_frame().unwrap().pts, first_pts);
}

#[test]
fn preserves_variable_frame_timestamps() {
    let mut decoder = FfmpegDecoder::new();
    decoder
        .open(
            &fixture("vfr-h264.mp4"),
            DecodeOptions {
                hardware: false,
                max_height: 0,
            },
        )
        .unwrap();
    let mut presentation_times = Vec::new();

    loop {
        match decoder.next_frame() {
            Ok(frame) => presentation_times.push(frame.presentation_time()),
            Err(VideoError::EndOfStream) => break,
            Err(error) => panic!("variable framerate fixture decode failed: {error}"),
        }
    }

    assert_eq!(presentation_times.len(), 5);
    let intervals: Vec<_> = presentation_times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    assert!(intervals.windows(2).any(|pair| pair[0] != pair[1]));
}

#[test]
fn rejects_corrupt_input() {
    let mut decoder = FfmpegDecoder::new();
    let error = decoder
        .open(
            &fixture("corrupt-video.bin"),
            DecodeOptions {
                hardware: false,
                max_height: 0,
            },
        )
        .expect_err("corrupt input should not open successfully");
    assert!(matches!(error, VideoError::Open { .. }));
}
