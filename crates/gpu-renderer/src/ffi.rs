use better_wallpaper_core::config::FillMode;
use better_wallpaper_core::{DecodeOptions, VideoDecoder, VideoError};
use better_wallpaper_ffmpeg::FfmpegDecoder;
use std::ffi::{CStr, c_char};

use crate::{GlLoaderFn, GpuRenderer};

pub struct KdeVideoDecoder {
    decoder: FfmpegDecoder,
    pixels: Vec<u8>,
}

#[repr(C)]
pub struct KdeVideoFrame {
    pub data: *const u8,
    pub len: usize,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pts_millis: u64,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kde_video_decoder_open(path: *const c_char) -> *mut KdeVideoDecoder {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    let path = unsafe { CStr::from_ptr(path) };
    let Ok(path) = path.to_str() else {
        return std::ptr::null_mut();
    };
    let mut decoder = FfmpegDecoder::new();
    if let Err(error) = decoder.open(
        std::path::Path::new(path),
        DecodeOptions { hardware: false, max_height: 0 },
    ) {
        tracing::error!(%error, path, "KDE direct video decoder open failed");
        return std::ptr::null_mut();
    }
    tracing::info!(path, "KDE direct video decoder opened");
    Box::into_raw(Box::new(KdeVideoDecoder { decoder, pixels: Vec::new() }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kde_video_decoder_next(
    decoder: *mut KdeVideoDecoder,
    output: *mut KdeVideoFrame,
) -> i32 {
    if decoder.is_null() || output.is_null() {
        return -1;
    }
    let decoder = unsafe { &mut *decoder };
    match decoder.decoder.next_frame() {
        Ok(frame) => {
            let pts_millis = frame.presentation_time().as_millis() as u64;
            decoder.pixels = frame.pixels;
            unsafe {
                *output = KdeVideoFrame {
                    data: decoder.pixels.as_ptr(),
                    len: decoder.pixels.len(),
                    width: frame.width,
                    height: frame.height,
                    stride: frame.stride as u32,
                    pts_millis,
                };
            }
            1
        }
        Err(VideoError::EndOfStream) => 0,
        Err(error) => {
            tracing::error!(%error, "KDE direct video decode failed");
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kde_video_decoder_seek_start(decoder: *mut KdeVideoDecoder) -> bool {
    !decoder.is_null() && unsafe { &mut *decoder }.decoder.seek_start().is_ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kde_video_decoder_destroy(decoder: *mut KdeVideoDecoder) {
    if !decoder.is_null() {
        unsafe { drop(Box::from_raw(decoder)); }
    }
}

fn fill_mode_from_int(raw: i32) -> FillMode {
    match raw {
        0 => FillMode::Cover,
        1 => FillMode::Contain,
        2 => FillMode::Stretch,
        _ => FillMode::Cover,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpu_renderer_create(
    loader: GlLoaderFn,
    output_width: u32,
    output_height: u32,
) -> *mut GpuRenderer {
    match GpuRenderer::from_loader(loader, output_width, output_height) {
        Ok(renderer) => {
            let boxed = Box::new(renderer);
            Box::into_raw(boxed)
        }
        Err(msg) => {
            tracing::error!(%msg, "gpu_renderer_create failed");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpu_renderer_destroy(renderer: *mut GpuRenderer) {
    if renderer.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(renderer));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpu_renderer_upload(
    renderer: *mut GpuRenderer,
    data: *const u8,
    data_len: u32,
    width: u32,
    height: u32,
    stride: u32,
) -> bool {
    if renderer.is_null() || data.is_null() {
        return false;
    }
    unsafe {
        let renderer = &mut *renderer;
        let slice = std::slice::from_raw_parts(data, data_len as usize);
        renderer.upload_frame(slice, width, height, stride).is_ok()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpu_renderer_draw(
    renderer: *const GpuRenderer,
    output_width: u32,
    output_height: u32,
    fill_mode: i32,
) {
    if renderer.is_null() {
        return;
    }
    unsafe {
        let renderer = &*renderer;
        renderer.draw(output_width, output_height, fill_mode_from_int(fill_mode));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gpu_renderer_resize(
    renderer: *mut GpuRenderer,
    width: u32,
    height: u32,
) {
    if renderer.is_null() {
        return;
    }
    unsafe {
        (*renderer).resize(width, height);
    }
}
