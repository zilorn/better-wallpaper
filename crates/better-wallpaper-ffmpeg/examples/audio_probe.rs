use std::{env, path::PathBuf};

use better_wallpaper_core::VideoError;
use better_wallpaper_ffmpeg::FfmpegAudioDecoder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: audio_probe <media>")?;
    let (mut decoder, info) = FfmpegAudioDecoder::open(&path)?;
    let mut chunks = 0_u64;
    let mut samples = 0_usize;
    loop {
        match decoder.next_samples() {
            Ok(chunk) => {
                chunks += 1;
                samples += chunk.len();
            }
            Err(VideoError::EndOfStream) => break,
            Err(error) => return Err(error.into()),
        }
    }
    println!(
        "decoded {chunks} chunks and {samples} samples ({} channels at {} Hz)",
        info.channels, info.sample_rate
    );
    Ok(())
}
