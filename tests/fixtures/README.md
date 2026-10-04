# Test fixtures

`scene-video.mp4` is an original, generated 16×16, 2 FPS, two-second lossless
H.264 RGB clip: red, green, blue, yellow. It contains no audio. Regenerate from
four RGBA frames using `ffmpeg -f rawvideo -pixel_format rgba -video_size 16x16
-framerate 2 -i colors.rgba -c:v libx264rgb -crf 0 -preset ultrafast scene-video.mp4`.
It verifies scene-clock scheduling, loop boundaries, pause, shared CPU frames,
and cancellation while the bounded queue is full.
