use std::path::{Path, PathBuf};

use better_wallpaper_core::{DecodeOptions, PixelFormat, VideoDecoder, VideoError};
use better_wallpaper_ffmpeg::FfmpegDecoder;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn decode_fixture(name: &str) {
    let mut decoder = FfmpegDecoder::new();
    let media = decoder
        .open(
            &fixture(name),
            DecodeOptions {
                hardware: false,
                max_height: 0,
            },
        )
        .expect("fixture should open");
    assert_eq!((media.width, media.height), (64, 36));

    let mut frames = Vec::new();
    loop {
        match decoder.next_frame() {
            Ok(frame) => {
                assert_eq!(frame.format, PixelFormat::Rgba);
                assert_eq!(frame.stride, 64 * 4);
                assert_eq!(frame.pixels.len(), 64 * 36 * 4);
                frames.push(frame.pts);
            }
            Err(VideoError::EndOfStream) => break,
            Err(error) => panic!("fixture decode failed: {error}"),
        }
    }

    assert_eq!(frames.len(), 5);
    assert!(frames.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn decodes_h264_vp9_and_av1_fixtures() {
    for name in ["h264.mp4", "vp9.webm", "av1.mkv"] {
        decode_fixture(name);
    }
}

#[test]
fn hardware_request_decodes_or_falls_back_to_software() {
    let mut decoder = FfmpegDecoder::new();
    decoder
        .open(
            &fixture("h264.mp4"),
            DecodeOptions {
                hardware: true,
                max_height: 0,
            },
        )
        .expect("hardware decode request should not prevent opening");
    let frame = decoder
        .next_frame()
        .expect("should produce frames after hardware or software fallback");
    assert_eq!(frame.format, PixelFormat::Rgba);
    assert_eq!(frame.pixels.len(), 64 * 36 * 4);
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
