# 发布与安装

`install.sh` 构建并安装支持 niri 与 KDE Plasma 6 的 daemon、Web 静态资源、Plasma 6
壁纸插件和 systemd 用户服务。当前 niri 后端的播放性能优于 KDE 后端。默认安装到
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

`SKIP_BUILD=1` 要求预先生成 `target/release/better-wallpaper-daemon`、`scene-inspect`、
`scene-validate` 和 `web/dist`；Plasma QML 包直接从源码安装。
卸载时运行 `./packaging/uninstall.sh`；脚本先停止并禁用用户服务，不删除
`~/.better-wallpaper` 中的用户配置。

提交发行产物前运行完整 staging 验收。脚本会构建 daemon 和 Web、验证 systemd/动态依赖/QML，
检查升级时清理旧原生帧模块，并检查卸载后无包文件残留：

```bash
./packaging/verify.sh
```

## 构建依赖

- Rust stable、Cargo、C/C++ 工具链、Clang 和 `pkg-config`
- Bun（只用于构建 Web UI）
- FFmpeg 命令行程序，以及开发库：`libavdevice`、`libavfilter`、`libavformat`、
  `libavcodec`、`libavutil`、`libswscale`、`libswresample`
- ALSA 开发库（Rodio/CPAL 的 Linux 音频输出后端）
- Wayland 客户端开发文件

Debian/Ubuntu 和 Fedora 的完整安装命令见项目根目录的
[`README.md`](../README.md#依赖)。包名随发行版和仓库变化，构建时以 Cargo 和
`pkg-config` 的检测结果为准。

## 运行时依赖

daemon 动态链接构建环境中的 FFmpeg、ALSA、Wayland、Vulkan loader 和系统 C/C++ 运行库。
发行包必须声明由 `ldd`/发行版依赖生成器得到的精确 ABI 依赖，不能只复制可执行文件。
Web UI 已编译为静态文件，运行时不需要 Bun。NVIDIA/Vulkan 不可用时会记录降级原因并使用
CPU `wl_shm` 路径。

托盘“Open UI”需要 `systemd-run`、正在运行的 systemd 用户管理器和 `xdg-open`
（xdg-utils）。浏览器通过独立的临时用户服务启动，使用桌面会话环境，避免继承壁纸
服务的只读文件系统、私有临时目录等限制而导致浏览器用户配置错误。启动失败原因
记录在壁纸服务日志中；浏览器后续输出记录在该临时用户服务的 journal 中。

niri 后端通过系统默认音频设备播放视频音轨；暂停、恢复、循环和配置重载会同步作用于音频。
媒体没有可解码音轨或音频设备不可用时，daemon 会记录英文警告并继续无声播放视频。

Plasma 插件是无需原生编译的 QML 包，运行时需要 Qt 6.6+ Qt Quick、Qt Multimedia
和 Qt WebEngine QML 模块。安装后在桌面壁纸设置的“壁纸类型”中选择“Better Wallpaper
视频壁纸”；插件从 daemon 的 HTTP 媒体接口读取视频文件，由 Qt Multimedia 解码并播放
视频和音频。安装脚本会清理旧版安装的未使用原生帧插件及其 `qmldir`。

服务日志写入 systemd journal：

```bash
journalctl --user -u better-wallpaper.service -f
```
