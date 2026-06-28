# Better Wallpaper

Linux 视频壁纸程序（开发中）。当前已实现配置管理、桌面环境检测、后端选择，以及
基于 FFmpeg 的软件解码、RGBA 帧转换、PTS 调度和有界帧队列。

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

管理服务会在 `127.0.0.1` 动态分配空闲端口，并将本次地址写入
`$XDG_RUNTIME_DIR/better-wallpaper/endpoint`；可通过托盘菜单打开管理页面。服务提供 `/api/v1/status`、
`/api/v1/config`、`/api/v1/library`、`/api/v1/library/media` 和 `/api/v1/ws`；壁纸库会扫描
`~/Videos` 及当前视频目录，媒体接口只允许读取扫描结果并支持范围请求。WebSocket 会推送播放
状态变化，断线后前端指数退避重连。

Plasma 6 用户安装后可在桌面壁纸设置中选择“Better Wallpaper 视频壁纸”。插件通过
`/api/v1/plasma/config` 按 Plasma 屏幕名同步启用状态和播放参数，再通过
`/api/v1/wallpaper/media` 播放当前配置的视频；daemon 或 plasmashell 重启后会自动重连恢复。
插件实例会发送本机心跳，管理界面的诊断页可查看当前在线的 Plasma 屏幕实例。
Plasma 在活动切换时隐藏壁纸实例后，视频会立即暂停并停止请求；实例重新可见时会同步最新配置并恢复。
配置更新由 Rust 端校验并原子写入；保存成功后会安全停止当前解码并用新配置重建播放管线。

可用有限运行时间执行稳定性验收，到期后会走正常取消和资源释放路径：

```bash
cargo run --release -p better-wallpaper-daemon -- \
  --backend headless --no-ui --run-for-seconds 1800
```

构建解码模块需要系统提供 `libavformat`、`libavcodec`、`libavutil`、`libswscale`
及 Clang。可使用解码探针验证本地视频和循环 seek：

```bash
cargo run -p better-wallpaper-ffmpeg --example decode_probe -- /path/to/video.mp4
```

在 niri 会话中会自动探测 NVIDIA Vulkan 设备，并校验 DMA-BUF 导入/导出与外部同步扩展。需要禁止 CPU 路径降级时使用：

```bash
cargo run -p better-wallpaper-daemon -- --backend niri --require-nvidia --no-ui
```

niri 后端现已支持单输出 wlr layer-shell background 表面和 `wl_shm` 软件帧提交。配置
`wallpaper.path` 后可直接运行：

```bash
cargo run --release -p better-wallpaper-daemon -- --backend niri --no-ui
```

若配置中存在启用的 `[[outputs]]`，使用第一项匹配 Wayland 输出名称；否则使用 compositor
报告的首个输出。当前阶段尚未完成多输出、热插拔和 compositor 断线重连。

## 安装

发布构建、Web 资源和 systemd 用户服务可通过打包脚本安装：

```bash
./packaging/install.sh
systemctl --user enable --now better-wallpaper.service
```

默认安装到 `~/.local`，可通过 `PREFIX` 和 `DESTDIR` 覆盖。发行版依赖、离线打包和卸载方式见
[`packaging/README.md`](packaging/README.md)。
