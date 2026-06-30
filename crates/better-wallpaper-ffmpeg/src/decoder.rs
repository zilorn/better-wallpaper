use std::{
    path::Path,
    time::{Duration, Instant},
};

use better_wallpaper_core::video::{
    ColorInfo, CudaFrame, DecodeOptions, DecodedFrame, MediaInfo, PixelFormat, Rational,
    VideoDecoder, VideoError,
};
use ffmpeg_next as ffmpeg;
use tracing::{debug, info, warn};

unsafe fn release_cuda_frame(owner: *mut std::ffi::c_void) {
    let mut frame = owner.cast::<ffmpeg::ffi::AVFrame>();
    unsafe { ffmpeg::ffi::av_frame_free(&mut frame) };
}

/// FFmpeg 在打开解码器时调用此函数协商输出格式。CUDA 不可用时保留列表中的
/// 第一个软件格式，使同一个解码器可以自动降级。
unsafe extern "C" fn select_cuda_format(
    _context: *mut ffmpeg::ffi::AVCodecContext,
    formats: *const ffmpeg::ffi::AVPixelFormat,
) -> ffmpeg::ffi::AVPixelFormat {
    if formats.is_null() {
        return ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE;
    }
    let mut current = formats;
    let mut fallback = ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE;
    loop {
        // SAFETY: FFmpeg 传入以 AV_PIX_FMT_NONE 结尾的格式数组。
        let format = unsafe { *current };
        if format == ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            return fallback;
        }
        if fallback == ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            fallback = format;
        }
        if format == ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_CUDA {
            return format;
        }
        // SAFETY: 尚未遇到数组终止项。
        current = unsafe { current.add(1) };
    }
}

fn enable_cuda(context: &mut ffmpeg::codec::context::Context) -> Result<(), String> {
    let mut device = std::ptr::null_mut();
    // SAFETY: device 是有效出参；成功后其引用交给 AVCodecContext 管理。
    let result = unsafe {
        ffmpeg::ffi::av_hwdevice_ctx_create(
            &mut device,
            ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        )
    };
    if result < 0 {
        return Err(ffmpeg::Error::from(result).to_string());
    }
    // SAFETY: context 尚未打开，FFmpeg 允许在 avcodec_open2 前设置这两个字段。
    unsafe {
        let raw = context.as_mut_ptr();
        (*raw).hw_device_ctx = device;
        (*raw).get_format = Some(select_cuda_format);
    }
    Ok(())
}

pub struct FfmpegDecoder {
    input: Option<ffmpeg::format::context::Input>,
    decoder: Option<ffmpeg::decoder::Video>,
    scaler: Option<ffmpeg::software::scaling::Context>,
    scaler_source: Option<ffmpeg::format::Pixel>,
    stream_index: usize,
    time_base: Rational,
    draining: bool,
    hardware_requested: bool,
    max_height: u32,
    hardware_active: Option<bool>,
    perf_started: Instant,
    perf_frames: u64,
    perf_transfer: Duration,
    perf_convert: Duration,
    perf_copy: Duration,
}

impl Default for FfmpegDecoder {
    fn default() -> Self {
        Self {
            input: None,
            decoder: None,
            scaler: None,
            scaler_source: None,
            stream_index: 0,
            time_base: Rational {
                numerator: 0,
                denominator: 1,
            },
            draining: false,
            hardware_requested: false,
            max_height: 0,
            hardware_active: None,
            perf_started: Instant::now(),
            perf_frames: 0,
            perf_transfer: Duration::ZERO,
            perf_convert: Duration::ZERO,
            perf_copy: Duration::ZERO,
        }
    }
}

impl FfmpegDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, VideoError> {
        let decoder = self.decoder.as_mut().ok_or_else(|| {
            VideoError::Decode("open must be called before reading video frames".to_owned())
        })?;
        let mut source = ffmpeg::util::frame::Video::empty();
        match decoder.receive_frame(&mut source) {
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => return Err(VideoError::EndOfStream),
            Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {
                return Ok(None);
            }
            Err(error) => return Err(VideoError::Decode(error.to_string())),
        }

        let pts = source.pts().unwrap_or(0);
        let color = ColorInfo {
            space: Some(source.color_space() as i32),
            range: Some(source.color_range() as i32),
        };
        let hardware_frame = source.format() == ffmpeg::format::Pixel::CUDA;
        if self.hardware_requested && self.hardware_active != Some(hardware_frame) {
            self.hardware_active = Some(hardware_frame);
            if hardware_frame {
                info!("NVIDIA CUDA/NVDEC hardware decoding enabled");
            } else {
                warn!(
                    "Current video codec does not support CUDA hardware decoding, falling back to software decoding"
                );
            }
        }
        if hardware_frame {
            let frames = unsafe {
                (*(*source.as_ptr()).hw_frames_ctx)
                    .data
                    .cast::<ffmpeg::ffi::AVHWFramesContext>()
            };
            if unsafe { (*frames).sw_format } == ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NV12 {
                let raw = unsafe { ffmpeg::ffi::av_frame_clone(source.as_ptr()) };
                if raw.is_null() {
                    return Err(VideoError::Decode("Failed to retain CUDA frame".into()));
                }
                let cuda = unsafe {
                    CudaFrame::new(
                        [(*raw).data[0] as usize, (*raw).data[1] as usize],
                        [(*raw).linesize[0] as usize, (*raw).linesize[1] as usize],
                        raw.cast(),
                        release_cuda_frame,
                    )
                };
                return Ok(Some(DecodedFrame {
                    pixels: Vec::new(),
                    cuda: Some(cuda),
                    format: PixelFormat::Rgba,
                    width: source.width(),
                    height: source.height(),
                    stride: 0,
                    pts,
                    time_base: self.time_base,
                    color,
                }));
            }
        }
        let mut software_frame = ffmpeg::util::frame::Video::empty();
        let source = if hardware_frame {
            let transfer_started = Instant::now();
            // SAFETY: 两个 AVFrame 均有效，目标空帧由 FFmpeg 分配系统内存。
            let result = unsafe {
                ffmpeg::ffi::av_hwframe_transfer_data(
                    software_frame.as_mut_ptr(),
                    source.as_ptr(),
                    0,
                )
            };
            if result < 0 {
                return Err(VideoError::Decode(format!(
                    "Failed to transfer CUDA frame back to system memory: {}",
                    ffmpeg::Error::from(result)
                )));
            }
            self.perf_transfer += transfer_started.elapsed();
            &software_frame
        } else {
            &source
        };

        let source_format = source.format();
        let target_height = if self.max_height > 0 {
            self.max_height.min(source.height())
        } else {
            source.height()
        };
        let target_width = if target_height < source.height() {
            (source.width() as u64 * target_height as u64 / source.height() as u64) as u32
        } else {
            source.width()
        };
        if self.scaler_source != Some(source_format) {
            self.scaler = Some(
                ffmpeg::software::scaling::Context::get(
                    source_format,
                    source.width(),
                    source.height(),
                    ffmpeg::format::Pixel::RGBA,
                    target_width,
                    target_height,
                    ffmpeg::software::scaling::Flags::FAST_BILINEAR,
                )
                .map_err(|error| {
                    VideoError::Decode(format!("Failed to create RGBA converter: {error}"))
                })?,
            );
            self.scaler_source = Some(source_format);
            debug!(
                ?source_format,
                "Created RGBA converter for decoded frame format"
            );
        }
        let scaler = self.scaler.as_mut().expect("scaler was just initialized");
        let width = target_width;
        let height = target_height;
        let row_bytes = width as usize * 4;
        let mut pixels = vec![0; row_bytes * height as usize];
        let convert_started = Instant::now();
        let mut destination_data = [std::ptr::null_mut(); 8];
        destination_data[0] = pixels.as_mut_ptr();
        let mut destination_stride = [0_i32; 8];
        destination_stride[0] = row_bytes as i32;
        // 直接写入 DecodedFrame 最终缓冲，避免 FFmpeg RGBA AVFrame -> Vec 的整帧复制。
        let converted_rows = unsafe {
            ffmpeg::ffi::sws_scale(
                scaler.as_mut_ptr(),
                (*source.as_ptr()).data.as_ptr() as *const *const u8,
                (*source.as_ptr()).linesize.as_ptr(),
                0,
                height as i32,
                destination_data.as_mut_ptr(),
                destination_stride.as_mut_ptr(),
            )
        };
        if converted_rows != height as i32 {
            return Err(VideoError::Decode(format!(
                "Pixel format conversion returned {converted_rows}/{height} rows"
            )));
        }
        self.perf_convert += convert_started.elapsed();
        self.perf_frames += 1;
        let perf_elapsed = self.perf_started.elapsed();
        if perf_elapsed >= Duration::from_secs(1) {
            let frames = self.perf_frames.max(1) as f64;
            info!(
                frames = self.perf_frames,
                gpu_transfer_avg_ms = self.perf_transfer.as_secs_f64() * 1000.0 / frames,
                rgba_convert_avg_ms = self.perf_convert.as_secs_f64() * 1000.0 / frames,
                rgba_copy_avg_ms = self.perf_copy.as_secs_f64() * 1000.0 / frames,
                "FFmpeg real-time frame processing performance"
            );
            self.perf_started = Instant::now();
            self.perf_frames = 0;
            self.perf_transfer = Duration::ZERO;
            self.perf_convert = Duration::ZERO;
            self.perf_copy = Duration::ZERO;
        }
        Ok(Some(DecodedFrame {
            pixels,
            cuda: None,
            format: PixelFormat::Rgba,
            width,
            height,
            stride: row_bytes,
            pts,
            time_base: self.time_base,
            color,
        }))
    }
}

impl VideoDecoder for FfmpegDecoder {
    fn open(&mut self, path: &Path, options: DecodeOptions) -> Result<MediaInfo, VideoError> {
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
        let mut context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|error| VideoError::Decode(error.to_string()))?;
        context.set_threading(ffmpeg::codec::threading::Config {
            kind: ffmpeg::codec::threading::Type::Frame,
            count: 0,
        });
        if options.hardware {
            match enable_cuda(&mut context) {
                Ok(()) => info!(
                    "FFmpeg CUDA hardware device created, waiting for decode format negotiation"
                ),
                Err(error) => {
                    warn!(%error, "NVIDIA CUDA decode device initialization failed, falling back to software decoding")
                }
            }
        }
        let decoder = context
            .decoder()
            .video()
            .map_err(|error| VideoError::Decode(error.to_string()))?;
        let width = decoder.width();
        let height = decoder.height();
        let rate = (frame_rate.denominator() != 0)
            .then(|| frame_rate.numerator() as f64 / frame_rate.denominator() as f64);

        self.input = Some(input);
        self.decoder = Some(decoder);
        self.scaler = None;
        self.scaler_source = None;
        self.stream_index = stream_index;
        self.time_base = time_base;
        self.draining = false;
        self.hardware_requested = options.hardware;
        self.max_height = options.max_height;
        self.hardware_active = None;
        info!(
            path = %path.display(), width, height, frame_rate = ?rate,
            time_base_num = time_base.numerator, time_base_den = time_base.denominator,
            "FFmpeg video stream opened"
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
                .ok_or_else(|| VideoError::Decode("open must be called first".to_owned()))?
                .packets()
                .find_map(|(stream, packet)| {
                    (stream.index() == self.stream_index).then_some(packet)
                });
            match packet {
                Some(packet) => self
                    .decoder
                    .as_mut()
                    .expect("decoder must exist after open")
                    .send_packet(&packet)
                    .map_err(|error| VideoError::Decode(error.to_string()))?,
                None => {
                    debug!("Input reached EOF, flushing FFmpeg decoder");
                    self.decoder
                        .as_mut()
                        .expect("decoder must exist after open")
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
            .ok_or_else(|| VideoError::Seek("open must be called first".to_owned()))?
            .seek(0, ..0)
            .map_err(|error| VideoError::Seek(error.to_string()))?;
        self.decoder
            .as_mut()
            .ok_or_else(|| VideoError::Seek("open must be called first".to_owned()))?
            .flush();
        self.draining = false;
        debug!("FFmpeg seeked to video start and reset decoder");
        Ok(())
    }
}
