use std::{env, path::PathBuf};

use better_wallpaper_core::{DecodeOptions, VideoDecoder, VideoError};
use better_wallpaper_ffmpeg::FfmpegDecoder;

fn main() -> Result<(), VideoError> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| VideoError::Open {
            path: "<missing>".to_owned(),
            message: "用法: decode_probe <video>".to_owned(),
        })?;
    let mut decoder = FfmpegDecoder::new();
    let info = decoder.open(&path, DecodeOptions::default())?;
    let mut frames = 0_u64;
    loop {
        match decoder.next_frame() {
            Ok(frame) => {
                frames += 1;
                assert_eq!(frame.pixels.len(), frame.stride * frame.height as usize);
            }
            Err(VideoError::EndOfStream) => break,
            Err(error) => return Err(error),
        }
    }
    decoder.seek_start()?;
    let loop_frame = decoder.next_frame()?;
    println!(
        "decoded_frames={frames} width={} height={} loop_first_pts={}",
        info.width, info.height, loop_frame.pts
    );
    Ok(())
}
