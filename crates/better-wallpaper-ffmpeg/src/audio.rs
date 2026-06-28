use std::path::Path;

use better_wallpaper_core::VideoError;
use ffmpeg_next as ffmpeg;
use tracing::{debug, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioInfo {
    pub channels: u16,
    pub sample_rate: u32,
}

pub struct FfmpegAudioDecoder {
    input: ffmpeg::format::context::Input,
    decoder: ffmpeg::decoder::Audio,
    resampler: ffmpeg::software::resampling::Context,
    stream_index: usize,
    info: AudioInfo,
    draining: bool,
}

impl FfmpegAudioDecoder {
    pub fn open(path: &Path) -> Result<(Self, AudioInfo), VideoError> {
        ffmpeg::init().map_err(|error| VideoError::Open {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        let input = ffmpeg::format::input(path).map_err(|error| VideoError::Open {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Audio)
            .ok_or(VideoError::NoAudioStream)?;
        let stream_index = stream.index();
        let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|error| VideoError::AudioDecode(error.to_string()))?;
        let decoder = context
            .decoder()
            .audio()
            .map_err(|error| VideoError::AudioDecode(error.to_string()))?;
        let channels = decoder.channels();
        let sample_rate = decoder.rate();
        let input_layout = if decoder.channel_layout().is_empty() {
            ffmpeg::ChannelLayout::default(i32::from(channels))
        } else {
            decoder.channel_layout()
        };
        let output_format = ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed);
        let resampler = ffmpeg::software::resampling::Context::get(
            decoder.format(),
            input_layout,
            sample_rate,
            output_format,
            input_layout,
            sample_rate,
        )
        .map_err(|error| VideoError::AudioDecode(error.to_string()))?;
        let info = AudioInfo {
            channels,
            sample_rate,
        };
        let codec = decoder.codec().map(|codec| codec.name().to_owned());
        info!(
            path = %path.display(),
            channels,
            sample_rate,
            ?codec,
            "FFmpeg audio stream opened"
        );
        Ok((
            Self {
                input,
                decoder,
                resampler,
                stream_index,
                info,
                draining: false,
            },
            info,
        ))
    }

    pub fn next_samples(&mut self) -> Result<Vec<f32>, VideoError> {
        loop {
            let mut decoded = ffmpeg::frame::Audio::empty();
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => {
                    let mut output = ffmpeg::frame::Audio::empty();
                    self.resampler
                        .run(&decoded, &mut output)
                        .map_err(|error| VideoError::AudioDecode(error.to_string()))?;
                    let sample_count = output.samples() * usize::from(self.info.channels);
                    if sample_count == 0 {
                        continue;
                    }
                    // SAFETY: swresample allocated a packed native-endian f32 plane containing
                    // `samples * channels` values for the requested output format.
                    let samples = unsafe {
                        std::slice::from_raw_parts(
                            output.data(0).as_ptr().cast::<f32>(),
                            sample_count,
                        )
                    };
                    return Ok(samples.to_vec());
                }
                Err(ffmpeg::Error::Eof) => return Err(VideoError::EndOfStream),
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {}
                Err(error) => return Err(VideoError::AudioDecode(error.to_string())),
            }
            if self.draining {
                return Err(VideoError::EndOfStream);
            }
            let packet = self.input.packets().find_map(|(stream, packet)| {
                (stream.index() == self.stream_index).then_some(packet)
            });
            match packet {
                Some(packet) => self
                    .decoder
                    .send_packet(&packet)
                    .map_err(|error| VideoError::AudioDecode(error.to_string()))?,
                None => {
                    self.decoder
                        .send_eof()
                        .map_err(|error| VideoError::AudioDecode(error.to_string()))?;
                    self.draining = true;
                }
            }
        }
    }

    pub fn seek_start(&mut self) -> Result<(), VideoError> {
        self.input
            .seek(0, ..0)
            .map_err(|error| VideoError::AudioSeek(error.to_string()))?;
        self.decoder.flush();
        self.draining = false;
        debug!("FFmpeg seeked to audio start and reset decoder");
        Ok(())
    }
}
