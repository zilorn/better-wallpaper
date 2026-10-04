# 发布与安装

`install.sh` 构建并安装支持 niri 与 KDE Plasma 6 的 daemon、Tauri 2 桌面客户端、应用菜单入口、Web 静态资源、Plasma 6
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
`scene-validate`、`better-wallpaper-desktop` 和 `web/dist`；Plasma QML 包直接从源码安装。
卸载时运行 `./packaging/uninstall.sh`；脚本先停止并禁用用户服务，不删除
`~/.better-wallpaper` 中的用户配置。

提交发行产物前运行完整 staging 验收。脚本会构建 daemon、桌面客户端和 Web、验证 systemd/动态依赖/QML，
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
- GTK 3 和 WebKitGTK 4.1 开发库（Tauri 2 桌面客户端）

Debian/Ubuntu 和 Fedora 的完整安装命令见项目根目录的
[`README.md`](../README.md#依赖)。包名随发行版和仓库变化，构建时以 Cargo 和
`pkg-config` 的检测结果为准。

## 运行时依赖

daemon 动态链接构建环境中的 FFmpeg、ALSA、Wayland、Vulkan loader 和系统 C/C++ 运行库。
发行包必须声明由 `ldd`/发行版依赖生成器得到的精确 ABI 依赖，不能只复制可执行文件。
Web UI 已编译为静态文件，运行时不需要 Bun。NVIDIA/Vulkan 不可用时会记录降级原因并使用
CPU `wl_shm` 路径。

桌面客户端运行时需要 GTK 3、WebKitGTK 4.1 和桌面会话 D-Bus；应用菜单入口位于
`$PREFIX/share/applications/org.betterwallpaper.desktop.desktop`。客户端通过 endpoint
发现文件连接 daemon，只允许 `http://127.0.0.1:<port>`，验证 API v1 和已构建的管理页面；
服务未就绪时尝试 `systemctl --user --no-block start better-wallpaper.service`，随后限时等待，
失败可在窗口内重试。`--url` 可指定本机管理地址，指定后不自动启动用户服务。
客户端使用单实例插件，重复启动聚焦现有窗口；关闭窗口退出客户端，daemon 和托盘继续运行。
检测到已加载的 NVIDIA 驱动时，客户端默认设置 `WEBKIT_DISABLE_DMABUF_RENDERER=1`，
规避 WebKitGTK 图形缓冲区分配失败；这会放弃 WebKit 的较快 DMA-BUF 呈现路径。
用户已设置该变量时会保留其值，其他 GPU 环境不变。

托盘“Open desktop client”需要 `systemd-run` 和正在运行的 systemd 用户管理器。
它优先启动 daemon 同目录的 `better-wallpaper-desktop --url <endpoint>`；未安装客户端时
回退到 `xdg-open`（xdg-utils）打开浏览器。启动器通过独立的临时用户服务使用桌面会话环境，
避免继承壁纸服务的只读文件系统、私有临时目录和图形驱动覆盖变量。启动失败记录在
壁纸服务日志中，客户端后续输出记录在临时服务的 journal 中。

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

## niri 网页壁纸渲染器

安装流程还通过 CMake 构建 `native/web-wallpaper`，将 `better-wallpaper-web` 原子安装到
`$PREFIX/bin`，卸载时一并删除。构建需要 CMake、Qt 6.6+ Quick / WebEngineQuick 和
匹配 Qt 6 的 LayerShellQt 开发包；运行需要 Qt WebEngine QML、Qt Wayland 和 LayerShellQt。
Chromium 沙箱保持启用。`SKIP_BUILD=1` 要求 `target/web-wallpaper/better-wallpaper-web`
已经存在。`verify.sh` 检查该程序的 staging 安装、动态依赖和卸载，并检查其 QML。

本地开发可设置 `BETTER_WALLPAPER_WEB_PLAYER` 指向该构建产物；安装后 daemon 默认查找
同目录程序。网页的私有资源服务不依赖管理服务，`--no-ui` 也可播放；
`--run-for-seconds` 可限制 niri 网页播放时长。默认属性和暂停回调兼容 Wallpaper Engine，
音频频谱/媒体信息回调暂未接入。
