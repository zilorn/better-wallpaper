use std::path::Path;

use better_wallpaper_core::video::{
    ColorInfo, DecodeOptions, DecodedFrame, MediaInfo, PixelFormat, Rational, VideoDecoder,
    VideoError,
};
use ffmpeg_next as ffmpeg;
use tracing::{debug, info, warn};

pub struct FfmpegDecoder {
    input: Option<ffmpeg::format::context::Input>,
    decoder: Option<ffmpeg::decoder::Video>,
    scaler: Option<ffmpeg::software::scaling::Context>,
    stream_index: usize,
    time_base: Rational,
    draining: bool,
}

impl Default for FfmpegDecoder {
    fn default() -> Self {
        Self {
            input: None,
            decoder: None,
            scaler: None,
            stream_index: 0,
            time_base: Rational {
                numerator: 0,
                denominator: 1,
            },
            draining: false,
        }
    }
}

impl FfmpegDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, VideoError> {
        let decoder = self
            .decoder
            .as_mut()
            .ok_or_else(|| VideoError::Decode("必须先调用 open 再读取视频帧".to_owned()))?;
        let mut source = ffmpeg::util::frame::Video::empty();
        match decoder.receive_frame(&mut source) {
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => return Err(VideoError::EndOfStream),
            Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {
                return Ok(None);
            }
            Err(error) => return Err(VideoError::Decode(error.to_string())),
        }

        let scaler = self.scaler.as_mut().expect("open 后 scaler 必须存在");
        let mut rgba = ffmpeg::util::frame::Video::empty();
        scaler
            .run(&source, &mut rgba)
            .map_err(|error| VideoError::Decode(format!("像素格式转换失败: {error}")))?;
        let width = rgba.width();
        let height = rgba.height();
        let row_bytes = width as usize * 4;
        let source_stride = rgba.stride(0);
        let data = rgba.data(0);
        let mut pixels = vec![0; row_bytes * height as usize];
        for row in 0..height as usize {
            let src_start = row * source_stride;
            let dst_start = row * row_bytes;
            pixels[dst_start..dst_start + row_bytes]
                .copy_from_slice(&data[src_start..src_start + row_bytes]);
        }
        let pts = source.pts().unwrap_or(0);
        Ok(Some(DecodedFrame {
            pixels,
            format: PixelFormat::Rgba,
            width,
            height,
            stride: row_bytes,
            pts,
            time_base: self.time_base,
            color: ColorInfo {
                space: Some(source.color_space() as i32),
                range: Some(source.color_range() as i32),
            },
        }))
    }
}

impl VideoDecoder for FfmpegDecoder {
    fn open(&mut self, path: &Path, options: DecodeOptions) -> Result<MediaInfo, VideoError> {
        ffmpeg::init().map_err(|error| VideoError::Open {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        if options.hardware {
            warn!("硬件解码尚未实现，本次使用 FFmpeg 软件解码");
        }
        let input = ffmpeg::format::input(path).map_err(|error| VideoError::Open {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or(VideoError::NoVideoStream)?;
        let stream_index = stream.index();
        let stream_time_base = stream.time_base();
        let time_base = Rational {
            numerator: stream_time_base.numerator(),
            denominator: stream_time_base.denominator(),
        };
        let frame_rate = stream.avg_frame_rate();
        let duration = (stream.duration() > 0).then(|| time_base.duration(stream.duration()));
        let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|error| VideoError::Decode(error.to_string()))?;
        let decoder = context
            .decoder()
            .video()
            .map_err(|error| VideoError::Decode(error.to_string()))?;
        let width = decoder.width();
        let height = decoder.height();
        let scaler = ffmpeg::software::scaling::Context::get(
            decoder.format(),
            width,
            height,
            ffmpeg::format::Pixel::RGBA,
            width,
            height,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|error| VideoError::Decode(format!("创建 RGBA 转换器失败: {error}")))?;
        let rate = (frame_rate.denominator() != 0)
            .then(|| frame_rate.numerator() as f64 / frame_rate.denominator() as f64);

        self.input = Some(input);
        self.decoder = Some(decoder);
        self.scaler = Some(scaler);
        self.stream_index = stream_index;
        self.time_base = time_base;
        self.draining = false;
        info!(
            path = %path.display(), width, height, frame_rate = ?rate,
            time_base_num = time_base.numerator, time_base_den = time_base.denominator,
            "FFmpeg 视频流已打开"
        );
        Ok(MediaInfo {
            width,
            height,
            duration,
            frame_rate: rate,
            time_base,
        })
    }

    fn next_frame(&mut self) -> Result<DecodedFrame, VideoError> {
        loop {
            if let Some(frame) = self.receive_frame()? {
                return Ok(frame);
            }
            if self.draining {
                return Err(VideoError::EndOfStream);
            }

            let packet = self
                .input
                .as_mut()
                .ok_or_else(|| VideoError::Decode("必须先调用 open".to_owned()))?
                .packets()
                .find_map(|(stream, packet)| {
                    (stream.index() == self.stream_index).then_some(packet)
                });
            match packet {
                Some(packet) => self
                    .decoder
                    .as_mut()
                    .expect("open 后 decoder 必须存在")
                    .send_packet(&packet)
                    .map_err(|error| VideoError::Decode(error.to_string()))?,
                None => {
                    debug!("输入到达 EOF，刷新 FFmpeg 解码器");
                    self.decoder
                        .as_mut()
                        .expect("open 后 decoder 必须存在")
                        .send_eof()
                        .map_err(|error| VideoError::Decode(error.to_string()))?;
                    self.draining = true;
                }
            }
        }
    }

    fn seek_start(&mut self) -> Result<(), VideoError> {
        self.input
            .as_mut()
            .ok_or_else(|| VideoError::Seek("必须先调用 open".to_owned()))?
            .seek(0, ..0)
            .map_err(|error| VideoError::Seek(error.to_string()))?;
        self.decoder
            .as_mut()
            .ok_or_else(|| VideoError::Seek("必须先调用 open".to_owned()))?
            .flush();
        self.draining = false;
        debug!("FFmpeg 已跳转到视频起点并重置解码器");
        Ok(())
    }
}
