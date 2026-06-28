# 发布与安装

`install.sh` 构建并安装 daemon、Web 静态资源、Plasma 6 壁纸插件和 systemd 用户服务。默认安装到
`~/.local`，服务文件写入 `~/.config/systemd/user`，不需要 root 权限：

```bash
./packaging/install.sh
systemctl --user enable --now better-wallpaper.service
```

制作发行版包时应使用 staging 目录，避免启动服务或写入用户目录。非默认前缀下，服务文件
安装到 `$PREFIX/lib/systemd/user`：

```bash
DESTDIR="$PWD/pkg" PREFIX=/usr SKIP_BUILD=1 ./packaging/install.sh
```

`SKIP_BUILD=1` 要求预先生成 `target/release/better-wallpaper-daemon` 和 `web/dist`。
卸载时运行 `./packaging/uninstall.sh`；脚本先停止并禁用用户服务，不删除
`~/.better-wallpaper` 中的用户配置。

提交发行产物前运行完整 staging 验收。脚本会构建 daemon 和 Web、验证 systemd/动态依赖/QML，
并检查卸载后无包文件残留：

```bash
./packaging/verify.sh
```

## 构建依赖

- Rust stable、Cargo、Clang 和 `pkg-config`
- Bun（只用于构建 Web UI）
- FFmpeg 开发库：`libavformat`、`libavcodec`、`libavutil`、`libswscale`、`libswresample`
- ALSA 开发库（Rodio/CPAL 的 Linux 音频输出后端）
- Wayland 客户端开发文件

Debian/Ubuntu 的 FFmpeg 开发包通常为 `libavformat-dev libavcodec-dev libavutil-dev
libswscale-dev libswresample-dev`，ALSA 开发包为 `libasound2-dev`；Fedora 通常由启用的 FFmpeg 仓库提供对应
`ffmpeg-free-devel` 或 `ffmpeg-devel` 包，ALSA 开发包为 `alsa-lib-devel`。包名随发行版和仓库
变化，构建时以 `pkg-config` 检测结果为准。

## 运行时依赖

daemon 动态链接构建环境中的 FFmpeg、ALSA、Wayland、Vulkan loader 和系统 C/C++ 运行库。
发行包必须声明由 `ldd`/发行版依赖生成器得到的精确 ABI 依赖，不能只复制可执行文件。
Web UI 已编译为静态文件，运行时不需要 Bun。NVIDIA/Vulkan 不可用时会记录降级原因并使用
CPU `wl_shm` 路径。

niri 后端通过系统默认音频设备播放视频音轨；暂停、恢复、循环和配置重载会同步作用于音频。
媒体没有可解码音轨或音频设备不可用时，daemon 会记录英文警告并继续无声播放视频。

Plasma 插件构建依赖 CMake 以及 Qt 6 Core、Qml、Quick 开发包。安装后在桌面壁纸设置的
“壁纸类型”中选择“Better Wallpaper 视频壁纸”；Rust daemon 通过共享内存三缓冲发布
FFmpeg 解码帧，原生 Qt Quick 插件负责纹理显示。

服务日志写入 systemd journal：

```bash
journalctl --user -u better-wallpaper.service -f
```
