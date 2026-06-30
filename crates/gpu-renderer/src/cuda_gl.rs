use better_wallpaper_core::CudaFrame;
use glow::HasContext;
use std::{ffi::c_void, ptr};

type CuContext = *mut c_void;
type CuResource = *mut c_void;
type CuArray = *mut c_void;
const CUDA_SUCCESS: i32 = 0;
const CU_MEMORYTYPE_DEVICE: u32 = 2;
const CU_MEMORYTYPE_ARRAY: u32 = 3;
const CU_POINTER_ATTRIBUTE_CONTEXT: i32 = 1;
const CU_GRAPHICS_REGISTER_FLAGS_WRITE_DISCARD: u32 = 2;

#[repr(C)]
struct Memcpy2d {
    src_x: usize,
    src_y: usize,
    src_type: u32,
    src_host: *const c_void,
    src_device: u64,
    src_array: CuArray,
    src_pitch: usize,
    dst_x: usize,
    dst_y: usize,
    dst_type: u32,
    dst_host: *mut c_void,
    dst_device: u64,
    dst_array: CuArray,
    dst_pitch: usize,
    width: usize,
    height: usize,
}

#[link(name = "cuda")]
unsafe extern "C" {
    fn cuInit(flags: u32) -> i32;
    fn cuPointerGetAttribute(data: *mut c_void, attribute: i32, ptr: u64) -> i32;
    fn cuCtxPushCurrent_v2(context: CuContext) -> i32;
    fn cuCtxPopCurrent_v2(context: *mut CuContext) -> i32;
    fn cuGraphicsGLRegisterImage(
        resource: *mut CuResource,
        image: u32,
        target: u32,
        flags: u32,
    ) -> i32;
    fn cuGraphicsUnregisterResource(resource: CuResource) -> i32;
    fn cuGraphicsMapResources(count: u32, resources: *mut CuResource, stream: *mut c_void) -> i32;
    fn cuGraphicsUnmapResources(count: u32, resources: *mut CuResource, stream: *mut c_void)
    -> i32;
    fn cuGraphicsSubResourceGetMappedArray(
        array: *mut CuArray,
        resource: CuResource,
        layer: u32,
        level: u32,
    ) -> i32;
    fn cuMemcpy2D_v2(copy: *const Memcpy2d) -> i32;
}

pub struct CudaGl {
    y: glow::Texture,
    uv: glow::Texture,
    resources: [CuResource; 2],
    size: (u32, u32),
    context: CuContext,
}

impl CudaGl {
    pub unsafe fn new(gl: &glow::Context) -> Result<Self, String> {
        if unsafe { cuInit(0) } != CUDA_SUCCESS {
            return Err("cuInit failed".into());
        }
        let y = unsafe { gl.create_texture()? };
        let uv = unsafe { gl.create_texture()? };
        Ok(Self {
            y,
            uv,
            resources: [ptr::null_mut(); 2],
            size: (0, 0),
            context: ptr::null_mut(),
        })
    }

    pub unsafe fn upload(
        &mut self,
        gl: &glow::Context,
        frame: &CudaFrame,
        width: u32,
        height: u32,
    ) -> Result<(glow::Texture, glow::Texture), String> {
        let mut context = ptr::null_mut();
        check(
            unsafe {
                cuPointerGetAttribute(
                    (&mut context as *mut CuContext).cast(),
                    CU_POINTER_ATTRIBUTE_CONTEXT,
                    frame.planes[0] as u64,
                )
            },
            "CUDA pointer context",
        )?;
        check(unsafe { cuCtxPushCurrent_v2(context) }, "push CUDA context")?;
        self.context = context;
        let result = (|| {
            if self.size != (width, height) {
                unsafe { self.allocate(gl, width, height)? };
            }
            check(
                unsafe { cuGraphicsMapResources(2, self.resources.as_mut_ptr(), ptr::null_mut()) },
                "map CUDA GL textures",
            )?;
            let mut arrays = [ptr::null_mut(); 2];
            for (index, array) in arrays.iter_mut().enumerate() {
                check(
                    unsafe {
                        cuGraphicsSubResourceGetMappedArray(array, self.resources[index], 0, 0)
                    },
                    "get CUDA GL array",
                )?;
            }
            for (index, (width_bytes, rows)) in [
                (width as usize, height as usize),
                (width as usize, height as usize / 2),
            ]
            .into_iter()
            .enumerate()
            {
                let copy = Memcpy2d {
                    src_x: 0,
                    src_y: 0,
                    src_type: CU_MEMORYTYPE_DEVICE,
                    src_host: ptr::null(),
                    src_device: frame.planes[index] as u64,
                    src_array: ptr::null_mut(),
                    src_pitch: frame.strides[index],
                    dst_x: 0,
                    dst_y: 0,
                    dst_type: CU_MEMORYTYPE_ARRAY,
                    dst_host: ptr::null_mut(),
                    dst_device: 0,
                    dst_array: arrays[index],
                    dst_pitch: 0,
                    width: width_bytes,
                    height: rows,
                };
                check(
                    unsafe { cuMemcpy2D_v2(&copy) },
                    "copy CUDA plane to GL texture",
                )?;
            }
            check(
                unsafe {
                    cuGraphicsUnmapResources(2, self.resources.as_mut_ptr(), ptr::null_mut())
                },
                "unmap CUDA GL textures",
            )?;
            Ok((self.y, self.uv))
        })();
        let mut previous = ptr::null_mut();
        unsafe {
            cuCtxPopCurrent_v2(&mut previous);
        }
        result
    }

    pub unsafe fn destroy(&mut self, gl: &glow::Context) {
        if !self.context.is_null() {
            unsafe { cuCtxPushCurrent_v2(self.context) };
        }
        for resource in &mut self.resources {
            if !resource.is_null() {
                unsafe { cuGraphicsUnregisterResource(*resource) };
                *resource = ptr::null_mut();
            }
        }
        if !self.context.is_null() {
            let mut previous = ptr::null_mut();
            unsafe { cuCtxPopCurrent_v2(&mut previous) };
        }
        unsafe {
            gl.delete_texture(self.y);
            gl.delete_texture(self.uv);
        }
    }

    unsafe fn allocate(
        &mut self,
        gl: &glow::Context,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        for resource in &mut self.resources {
            if !resource.is_null() {
                unsafe { cuGraphicsUnregisterResource(*resource) };
                *resource = ptr::null_mut();
            }
        }
        for (texture, internal, format, w, h) in [
            (self.y, glow::R8, glow::RED, width, height),
            (self.uv, glow::RG8, glow::RG, width / 2, height / 2),
        ] {
            unsafe {
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    internal as i32,
                    w as i32,
                    h as i32,
                    0,
                    format,
                    glow::UNSIGNED_BYTE,
                    None,
                );
            }
        }
        for (index, texture) in [self.y, self.uv].into_iter().enumerate() {
            check(
                unsafe {
                    cuGraphicsGLRegisterImage(
                        &mut self.resources[index],
                        texture.0.get(),
                        glow::TEXTURE_2D,
                        CU_GRAPHICS_REGISTER_FLAGS_WRITE_DISCARD,
                    )
                },
                "register CUDA GL texture",
            )?;
        }
        self.size = (width, height);
        Ok(())
    }
}

fn check(code: i32, operation: &str) -> Result<(), String> {
    if code == CUDA_SUCCESS {
        Ok(())
    } else {
        Err(format!("{operation} failed with CUDA error {code}"))
    }
}
