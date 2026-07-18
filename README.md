# Better Wallpaper

Linux 视频壁纸程序（开发中），支持 niri 和 KDE Plasma 6。当前 niri 后端的播放性能和
成熟度优于 KDE 后端。项目已实现配置管理、桌面环境检测、后端选择，以及基于 FFmpeg 的
软件解码、RGBA 帧转换、PTS 调度和有界帧队列。

```bash
cargo run -p better-wallpaper-daemon -- --backend headless --no-ui
```

默认配置位于 `~/.better-wallpaper/config.toml`。程序会输出配置来源、桌面检测依据和最终后端等结构化日志。
配置 `wallpaper.path` 后，headless 后端会启动软件解码线程，通过容量为 3 的有界队列按 PTS 消费帧，并记录循环、丢帧和播放统计。

先构建 Web 页面，再启动默认管理服务：

```bash
cd web && bun run build
cd .. && cargo run -p better-wallpaper-daemon -- --backend headless
```

管理服务固定绑定到 `127.0.0.1:43129`，并将管理地址写入
`$XDG_RUNTIME_DIR/better-wallpaper/endpoint`；可通过托盘菜单打开管理页面。服务提供 `/api/v1/status`、
`/api/v1/config`、`/api/v1/library`、`/api/v1/library/media` 和 `/api/v1/ws`；壁纸库会扫描
`~/Videos` 及当前视频目录，媒体接口只允许读取扫描结果并支持范围请求。WebSocket 会推送播放
状态变化，断线后前端指数退避重连。

Plasma 6 用户安装后可在桌面壁纸设置中选择“Better Wallpaper 视频壁纸”。插件通过
`/api/v1/plasma/config` 按 Plasma 屏幕名同步启用状态和播放参数，再通过
静态链接的 Rust/FFmpeg C ABI 直接打开媒体并解码视频帧，音频由插件内 Qt Multimedia
直接播放原媒体。daemon 不传输音视频数据，仅负责配置、控制和状态查询；daemon 或
plasmashell 重启后插件会重新同步配置并恢复。
插件实例会发送本机心跳，管理界面的诊断页可查看当前在线的 Plasma 屏幕实例。
Plasma 在活动切换时隐藏壁纸实例后，视频会立即暂停并停止请求；实例重新可见时会同步最新配置并恢复。
配置更新由 Rust 端校验并原子写入；保存成功后会安全停止当前解码并用新配置重建播放管线。

可用有限运行时间执行稳定性验收，到期后会走正常取消和资源释放路径：

```bash
cargo run --release -p better-wallpaper-daemon -- \
  --backend headless --no-ui --run-for-seconds 1800
```

构建解码模块需要系统提供 `libavdevice`、`libavformat`、`libavcodec`、`libavutil`、
`libavfilter`、`libswscale`、`libswresample` 及 Clang。可使用解码探针验证本地视频、音频和循环 seek：

```bash
cargo run -p better-wallpaper-ffmpeg --example decode_probe -- /path/to/video.mp4
cargo run -p better-wallpaper-ffmpeg --example audio_probe -- /path/to/video.mp4
```

在 niri 会话中会自动探测 NVIDIA Vulkan 设备，并校验 DMA-BUF 导入/导出与外部同步扩展。需要禁止 CPU 路径降级时使用：

```bash
cargo run -p better-wallpaper-daemon -- --backend niri --require-nvidia --no-ui
```

niri 后端现已支持多输出 wlr layer-shell background 表面和 `wl_shm` 软件帧提交。配置
`wallpaper.path` 后可直接运行：

```bash
cargo run --release -p better-wallpaper-daemon -- --backend niri --no-ui
```

若配置中存在启用的 `[[outputs]]`，会为每个匹配的 Wayland 输出创建背景表面；否则自动选择
compositor 输出。当前阶段仍需继续验证热插拔和 compositor 断线重连。

## 安装

项目支持 niri 和 KDE Plasma 6。niri 后端不需要 Plasma 插件；如需构建 KDE Plasma 插件，
还需要 CMake、FFmpeg 开发库以及 Qt 6 Core、Qml、Quick、Multimedia 开发包。

### 依赖

完整构建需要 Rust stable、Cargo、Bun、C/C++ 工具链、Clang、CMake、`pkg-config`，以及
ALSA、FFmpeg、Wayland 和 Qt 6 开发库。运行时还需要 FFmpeg 命令行程序（用于生成缩略图）
以及 `parec`（通过 PipeWire/PulseAudio 默认输出 monitor 提供桌面音频响应）；Plasma 6 插件要求
Qt 6.6 或更高版本。

Debian/Ubuntu：

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential clang cmake pkg-config ffmpeg pulseaudio-utils \
  libasound2-dev libwayland-dev \
  libavcodec-dev libavdevice-dev libavfilter-dev libavformat-dev \
  libavutil-dev libswresample-dev libswscale-dev \
  qt6-base-dev qt6-declarative-dev qt6-multimedia-dev
```

Arch Linux（所有依赖均在官方仓库中）：

```bash
sudo pacman -S --needed \
  base-devel clang cmake pkgconf ffmpeg libpulse \
  alsa-lib wayland \
  qt6-base qt6-declarative qt6-multimedia
```

注意：Arch 的 `ffmpeg` 包同时提供运行时命令行和开发头文件，无需单独安装 `*-devel` 包。若使用 NVIDIA 专有驱动且需要 Vulkan DMA-BUF 支持，确保已安装 `nvidia-utils` 和 `vulkan-loader`。

Fedora（FFmpeg 开发包要求系统已启用提供完整 FFmpeg 的仓库）：

```bash
sudo dnf install \
  @development-tools clang cmake pkgconf-pkg-config ffmpeg ffmpeg-devel pulseaudio-utils \
  alsa-lib-devel wayland-devel \
  qt6-qtbase-devel qt6-qtdeclarative-devel qt6-qtmultimedia-devel
```

Rust 和 Bun 建议使用各自的官方安装方式；完成安装后确认 `cargo`、`rustc` 和 `bun` 位于
`PATH` 中。仅构建 daemon/Web UI 时可以省略 CMake 和 Qt 6 开发包；执行 `./install.sh`
会同时构建 Plasma 插件，因此需要上面的完整依赖。Ubuntu 24.04 仓库中的 Qt 6.4 不满足
版本要求；请使用提供 Qt 6.6+ 的发行版，或单独安装较新 Qt 并设置 `CMAKE_PREFIX_PATH`。

### 完整安装（推荐）

在项目根目录运行：

```bash
./install.sh
systemctl --user enable --now better-wallpaper.service
```

该脚本会构建并安装 daemon、Web UI、systemd 用户服务以及 Plasma 壁纸插件。
默认不需要 root 权限，插件安装到：

```text
~/.local/share/plasma/wallpapers/org.better-wallpaper
```

安装脚本会检查 `web/dist/index.html` 和 Plasma `main.qml`，并在复制后再次验证目标文件。
不要使用 `SKIP_BUILD=1`，除非已手动完成 Rust 发布构建和 `bun run build`。

如果日志仍显示 `/usr/share/better-wallpaper`，但本次安装输出是 `~/.local`，说明旧的系统级服务仍在运行。
可检查当前用户服务实际使用的路径：

```bash
systemctl --user cat better-wallpaper.service
systemctl --user restart better-wallpaper.service
```

daemon 不依赖 systemd 才能找到 Web UI。未显式设置 `BETTER_WALLPAPER_WEB_ROOT` 时，默认路径为
`$XDG_DATA_HOME/better-wallpaper/web`；如果 `XDG_DATA_HOME` 未设置，则使用
`~/.local/share/better-wallpaper/web`。

### 仅安装 Plasma 插件

已安装 `kpackagetool6` 时，可直接安装源码树中的 Plasma 包：

```bash
kpackagetool6 --type Plasma/Wallpaper --install kde/org.better-wallpaper
```

更新已安装的插件：

```bash
kpackagetool6 --type Plasma/Wallpaper --upgrade kde/org.better-wallpaper
```

仅安装插件不会安装或启动 daemon。需另行运行
`better-wallpaper-daemon`，否则插件无法获取配置和视频。

### 在 Plasma 中启用

1. 右键单击桌面，选择“桌面和壁纸”或“配置桌面和壁纸”。
2. 在“壁纸类型”中选择“Better Wallpaper 视频壁纸”。
3. 单击“应用”，然后在 Better Wallpaper Web UI 中选择视频和目标显示器。

如果列表中没有出现插件，请先注销并重新登录 Plasma 会话。可用以下命令确认插件文件已安装：

```bash
test -f ~/.local/share/plasma/wallpapers/org.better-wallpaper/metadata.json && echo installed
```

### 卸载

整套安装的内容使用项目脚本卸载：

```bash
./packaging/uninstall.sh
```

仅通过 `kpackagetool6` 安装的插件可单独卸载：

```bash
kpackagetool6 --type Plasma/Wallpaper --remove org.better-wallpaper
```

默认安装前缀为 `~/.local`，可通过 `PREFIX` 和 `DESTDIR` 覆盖。发行版依赖、离线打包和更详细的卸载说明见
[`packaging/README.md`](packaging/README.md)。
