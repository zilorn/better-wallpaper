use better_wallpaper_core::config::FillMode;

use crate::{GlLoaderFn, GpuRenderer};

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

