#ifndef GPU_RENDERER_H
#define GPU_RENDERER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* GL function loader: caller provides a function that resolves GL proc addresses. */
typedef void *(*GlLoaderFn)(const char *name);

/* Opaque renderer handle. */
typedef struct GpuRenderer GpuRenderer;
typedef struct KdeVideoDecoder KdeVideoDecoder;
typedef struct KdeVideoFrame {
    const uint8_t *data;
    size_t len;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint64_t pts_millis;
} KdeVideoFrame;

/* Create a renderer. Returns NULL on failure. */
GpuRenderer *gpu_renderer_create(GlLoaderFn loader,
                                 uint32_t output_width,
                                 uint32_t output_height);

/* Destroy the renderer and free all GPU resources. */
void gpu_renderer_destroy(GpuRenderer *renderer);

/* Upload RGBA frame data to GPU via PBO (async, non-blocking pipelined).
   stride must equal width * 4 (compact RGBA). Returns false on error. */
bool gpu_renderer_upload(GpuRenderer *renderer,
                         const uint8_t *data,
                         uint32_t data_len,
                         uint32_t width,
                         uint32_t height,
                         uint32_t stride);

/* Draw the previously uploaded texture as a full-screen quad.
   fill_mode: 0 = Cover, 1 = Contain, 2 = Stretch. */
void gpu_renderer_draw(GpuRenderer *renderer,
                       uint32_t output_width,
                       uint32_t output_height,
                       int fill_mode);

/* Notify the renderer that the output surface was resized. */
void gpu_renderer_resize(GpuRenderer *renderer,
                         uint32_t width,
                         uint32_t height);

KdeVideoDecoder *kde_video_decoder_open(const char *path);
int32_t kde_video_decoder_next(KdeVideoDecoder *decoder, KdeVideoFrame *output);
bool kde_video_decoder_seek_start(KdeVideoDecoder *decoder);
void kde_video_decoder_destroy(KdeVideoDecoder *decoder);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* GPU_RENDERER_H */
