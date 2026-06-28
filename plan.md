# Better Wallpaper 实施计划

## 当前进度（2026-06-27）

- M0：已完成工程骨架、基础日志、CI 和 SolidJS/Bun 构建；Rust 与 Web 检查均通过。
- M1：已完成配置创建/校验/原子写入、桌面检测、后端优先级及对应单元测试。
- M2：已完成。已完成 FFmpeg 软件解码、NVIDIA CUDA/NVDEC 硬件解码及自动软解降级、RGBA 转换、PTS 时钟、有界帧队列、循环 seek，以及 daemon 的 headless 播放闭环、暂停/取消控制和统计日志；已加入 H.264/VP9/AV1、可变帧率、损坏输入、循环 seek、取消退出和定时验收测试，并提供 `--run-for-seconds` 长测入口、已执行 30 分钟稳定性实测和 NVIDIA 真机性能验收。
- M3：已完成。已实现 niri 多输出 wlr layer-shell background 表面、共享解码帧分发、输出枚举、configure 生命周期、输出尺寸变化、wl_shm RGBA 提交及 cover/contain/stretch 缩放，并接入 FFmpeg PTS 播放；niri 已通过 Rodio/CPAL 接入系统默认音频设备，声音与首帧、暂停、恢复、循环和配置重载联动，失败时降级为无声视频；缩放模式已有自动化测试和无效尺寸保护；尚需真机验证多屏、分数缩放/旋转、热插拔、音画同步和 compositor 重连。
- M4：进行中。Plasma 6 壁纸已切换为 Rust/FFmpeg 解码、共享内存三缓冲发布和原生 Qt Quick 纹理显示，循环 seek 不再触发 Qt Multimedia EOS 清屏。插件继续按屏幕名称同步启用状态、缩放和心跳，daemon 负责暂停、声音、循环与配置重载。静态构建和自动测试已完成，仍需 Plasma 真机验证循环、双屏、活动切换和 plasmashell 重启恢复。M6 未开始（NVIDIA 能力探测不代表 M6 完成）。
- M5：已完成。daemon 已提供仅监听固定 loopback 地址 `127.0.0.1:43129` 的版本化状态/配置 API、播放暂停/恢复 API、WebSocket 状态推送、受限本地视频库扫描、支持 Range 的库内媒体预览和 Web 静态资源托管；SolidJS 前端已按 Waywallen 的信息架构加入 Router、响应式侧边栏以及壁纸、显示器、壁纸库、设置与诊断页面，可扫描、悬停预览并选择本地视频，也可编辑输出和运行设置，并在 WebSocket 断线后指数退避重连。配置更新由 Rust 校验并原子保存，保存后播放监督器会安全停止当前解码并用最新配置重建管线；请求、路由、连接、扫描、媒体读取、播放控制、配置重载与错误均有日志。安装包已包含 daemon、Web 资源、Plasma 6 壁纸插件和 systemd 用户服务，并提供 FFmpeg/Qt Multimedia 依赖说明；已加入并通过 release/Web 构建、staging 布局、systemd、动态依赖、QML 和卸载残留自动验收。

## 1. 项目目标

使用 Rust 实现 Linux 视频壁纸程序：启动时识别当前桌面环境，读取 `~/.better-wallpaper/config.toml`，通过 FFmpeg 解码视频，并由对应的桌面后端把画面显示在桌面背景层。首批支持 niri 和 KDE Plasma；管理界面沿用 `w/web` 的技术路线，使用 SolidJS，依赖和脚本统一由 Bun 管理。

首个可用版本只实现本地视频壁纸、循环播放、基础缩放、多显示器选择、启动恢复和 Web UI 管理。图片、在线壁纸、插件系统、音频播放、播放列表和复杂动画不进入首个里程碑。

## 2. 从 `w` 项目沿用的设计

- 桌面环境检测沿用 `w/src/display/spawner.rs` 的分层策略：依次读取 `XDG_CURRENT_DESKTOP`、`XDG_SESSION_DESKTOP`、`DESKTOP_SESSION`，按 `:` 或 `;` 拆分并统一转为小写；niri 额外用 `NIRI_SOCKET` 和 `$XDG_RUNTIME_DIR/niri.*.sock` 兜底。
- 显示实现按“核心服务 + 桌面后端”拆分。核心服务不直接写死 niri 或 KDE 行为，只负责配置、解码、播放状态和帧分发。
- niri 后端参考 `w/src/niri_display`：通过 Wayland `zwlr_layer_shell_v1` 创建 `background` layer surface，并为每个目标 `wl_output` 建立独立表面。这里的 “niri background” 指 layer-shell background 层，不依赖 `niri msg` 设置壁纸。
- KDE 采用专用 Plasma 后端，不把 layer-shell 当作可靠兜底。Rust 核心向 KDE 壁纸插件提供帧或视频源；插件负责把内容挂到 Plasma 桌面壁纸层。
- 视频管线参考 `w/plugins/org.waywallen.video`：FFmpeg 打开媒体、视频流选择、按 PTS 定时、循环 seek、软解码兜底；硬解码作为后续优化，不阻塞最小版本。
- UI 参考 `w/web`：SolidJS + TypeScript + Vite，前端通过本机 HTTP/WebSocket 与 Rust 服务通信，构建产物由 Rust 服务托管。
- 所有关键状态增加结构化日志：配置来源、桌面检测结果、后端选择、输出枚举、FFmpeg 解码模式、掉帧、后端重连和致命错误。

## 3. 建议目录结构

```text
better-wallpaper/
├── Cargo.toml
├── crates/
│   ├── better-wallpaper-core/       # 配置、桌面检测、播放控制、帧模型
│   ├── better-wallpaper-ffmpeg/     # FFmpeg 解码与 PTS 调度
│   ├── better-wallpaper-wayland/    # niri/layer-shell 输出后端
│   ├── better-wallpaper-kde/        # KDE IPC/帧共享适配
│   └── better-wallpaper-daemon/     # 主进程、HTTP/WS、生命周期管理
├── kde/                             # Plasma wallpaper plugin/package
├── web/                             # SolidJS 前端（Bun）
├── tests/
└── packaging/
```

如果初期拆分成本过高，可先使用单个 crate，并保持 `config`、`desktop`、`decoder`、`output`、`server` 模块边界；接口稳定后再拆 workspace。

## 4. 启动流程

1. 初始化日志和 panic hook，确保错误带模块、时间和上下文。
2. 解析少量 CLI 覆盖项，例如 `--config`、`--backend`、`--no-ui` 和 `--log-level`。
3. 确定配置路径。默认严格使用 `~/.better-wallpaper/config.toml`；目录或文件不存在时创建默认配置。无法解析时报告字段位置并停止启动，不静默覆盖用户文件。
4. 检测桌面环境和 Wayland 会话，输出检测依据；配置中的显式后端覆盖自动检测。
5. 初始化对应显示后端并枚举显示器。未知桌面环境进入 headless 状态，UI 可用但不启动解码，同时给出可操作错误。
6. 启动本机 HTTP/WebSocket 服务并托管 `web/dist`。
7. 根据配置恢复上次视频、目标显示器和播放状态；后端未 ready 前不开始产帧。
8. 监听配置/UI 操作、显示器热插拔、Wayland/KDE 后端断开和退出信号，执行可控重建与资源释放。

## 5. 配置文件设计

建议初始配置如下：

```toml
version = 1

[general]
backend = "auto"           # auto | niri | kde | headless
restore_on_start = true
log_level = "info"

[wallpaper]
path = "/home/user/Videos/wallpaper.mp4"
loop_playback = true
muted = true
fill_mode = "cover"        # cover | contain | stretch
fps_limit = 60

[decode]
hardware = "auto"          # 首版接受配置，但允许降级为 software

[[outputs]]
name = "DP-1"
enabled = true
```

实现要求：

- 使用 `serde` + `toml` 定义带默认值的强类型配置，并用 `version` 支持未来迁移。
- 路径展开只处理明确规则（例如开头的 `~/`），内部统一转为绝对路径。
- 配置写入采用“临时文件 + `fsync` + 原子替换”，避免崩溃产生半个 TOML。
- UI 修改配置后由 Rust 端校验和落盘；前端不得直接操作配置文件。
- 敏感运行时状态与持久配置分离，WebSocket 状态消息不能反向覆盖未知配置字段。

## 6. 核心 Rust 接口

定义稳定接口隔离解码和桌面输出：

```rust
trait VideoDecoder {
    fn open(&mut self, path: &Path, options: DecodeOptions) -> Result<MediaInfo>;
    fn next_frame(&mut self) -> Result<DecodedFrame>;
    fn seek_start(&mut self) -> Result<()>;
}

trait DesktopBackend {
    fn kind(&self) -> DesktopKind;
    fn outputs(&self) -> Result<Vec<OutputInfo>>;
    fn configure(&mut self, config: &WallpaperConfig) -> Result<()>;
    fn present(&mut self, output: &OutputId, frame: &VideoFrame) -> Result<PresentResult>;
    fn shutdown(&mut self) -> Result<()>;
}
```

`DecodedFrame` 必须携带像素格式、尺寸、色彩信息、PTS、time base 和可选硬件帧句柄。不要让 FFmpeg 类型越过解码 crate 边界；否则 niri、KDE 和测试后端会被具体 FFmpeg ABI 绑定。

## 7. FFmpeg 解码与渲染管线

### 第一阶段：正确性优先

- 通过 `ffmpeg-next`/`ffmpeg-sys-next` 或受控的 FFI 封装链接系统 FFmpeg；构建时明确检查 `libavformat`、`libavcodec`、`libavutil`、`libswscale`。
- 打开输入并选择最佳视频流，读取旋转、像素宽高比、色彩空间和平均帧率。
- 解码帧先用 `libswscale` 转成统一的 BGRA/RGBA CPU 缓冲区。
- 使用 PTS 和单调时钟调度展示；落后超过阈值时丢弃旧帧，不能靠固定 `sleep(1/fps)` 累积漂移。
- 文件结束时 flush decoder；循环播放时 seek 到起点、flush codec buffers 并重置时钟基准。
- 使用有界通道连接解码与显示，实施背压，禁止视频播放时无限堆积帧。

### 第二阶段：性能优化

- 增加 VA-API/Vulkan 硬解，`auto` 失败后自动退回软解并记录原因。
- niri 优先走 DMA-BUF + 显式同步，减少 CPU 拷贝；协商失败时保留共享内存/CPU 上传兜底。
- 按显示器刷新率合并重复展示，记录 decode、convert、present 延迟和 dropped-frame 计数。
- 多显示器相同内容共享解码结果，不为每块屏幕重复解码；只在缩放/输出阶段分叉。

## 8. 桌面后端

### niri

- 使用 `wayland-client`、`wayland-protocols` 和 `wayland-protocols-wlr`。
- 绑定 `wl_compositor`、`wl_output`、`zwlr_layer_shell_v1`；性能阶段再绑定 `zwp_linux_dmabuf_v1` 和显式同步协议。
- 每个启用的输出创建一个 layer surface：layer 为 `background`，锚定四边，exclusive zone 为 `-1`，keyboard interactivity 为 `none`，namespace 使用 `better-wallpaper`。
- 等待 `configure` 后才能提交匹配尺寸的 buffer；正确处理 scale、transform、输出增删和 surface closed。
- 首版可用 `wl_shm` 提交 RGBA 帧验证完整链路；随后替换为 DMA-BUF。旧 buffer 必须等待 compositor release 后复用。
- niri IPC 只用于可选状态功能，不作为呈现壁纸的必要依赖。

### KDE Plasma

- 提供 Plasma wallpaper plugin/package，由 Plasma 创建真正的桌面壁纸实例，Rust daemon 继续负责解码和状态管理。
- daemon 与插件使用 Unix Domain Socket 传输控制消息；帧数据首版使用共享内存和 buffer id，插件消费完后回传 release。
- 插件需要支持 Plasma 提供的屏幕标识、尺寸变化、活动切换和壁纸实例重建，并将其映射到 daemon 的输出模型。
- KDE 后端不存在或版本不兼容时，daemon 保持 UI 可用并提示安装插件，不能偷偷改用普通顶层窗口覆盖桌面。

### 后端选择规则

- 配置 `general.backend != "auto"` 时验证并使用指定后端。
- 自动模式中，检测到 `niri` 选择 niri backend；检测到 `kde`/`plasma` 选择 KDE backend。
- 环境变量互相冲突时，记录全部候选和最终依据；CLI `--backend` 的优先级最高。
- 为测试提供 headless backend，只校验帧时序、尺寸和资源释放。

## 9. SolidJS Web UI

- 在 `web/` 使用 `bun init`/`bun install` 管理依赖，保留 `solid-js`、`@solidjs/router`、`typescript`、`vite`、`vite-plugin-solid`，提交 `bun.lock`。
- 页面至少包括：当前壁纸、视频选择/应用、显示器选择、播放/暂停、循环和缩放方式、解码与后端诊断。
- Rust 提供版本化 JSON API；命令使用 HTTP 或 WebSocket request/response，状态和错误通过 WebSocket 推送。
- 前端连接断开后指数退避重连；操作按钮显示 pending/error，不能假设命令发送即成功。
- 开发模式运行 `bun run dev` 并代理 daemon API；发布模式执行 `bun run build`，Rust 只监听 `127.0.0.1` 并托管静态资源。
- 首版不引入额外状态管理库，使用 Solid signal/store 即可。

## 10. 里程碑与验收标准

### M0：工程骨架

- 建立 Rust workspace、基础日志、错误类型、CI 格式检查和 Web 工程。
- `cargo test`、`cargo clippy -- -D warnings`、`cargo fmt --check`、`bun run build` 均可执行。

### M1：配置与桌面检测

- 能创建、读取、校验并原子更新 `~/.better-wallpaper/config.toml`。
- 单元测试覆盖环境变量优先级、大小写、复合 desktop token、niri socket 兜底和未知桌面。
- UI/日志可看到检测到的桌面、依据和所选后端。

### M2：FFmpeg 播放核心

- headless backend 能连续播放 H.264/VP9/AV1 测试样本，正确处理可变帧率、循环和损坏文件。
- 运行 30 分钟内存稳定，有界队列不增长，暂停与退出能释放 decoder 和 frame buffer。

### M3：niri 最小可用版本

- 在真实 niri 会话中，单屏视频稳定显示在 background layer，不遮挡窗口和桌面交互。
- 支持暂停、恢复、循环、cover/contain/stretch 和输出尺寸变化。
- 随后验证双屏、缩放、旋转、热插拔和 compositor 重启恢复。

### M4：KDE Plasma 最小可用版本

- Plasma 插件可安装、枚举并与 daemon 建立 IPC。
- 单屏和双屏能独立启用/禁用视频；plasmashell 重启后可重连并恢复配置。
- 锁屏、活动切换和屏幕配置变化不遗留进程或共享内存。

### M5：UI 与发布

- UI 完成视频选择、应用、显示器设置、播放控制和诊断。
- 打包包含 daemon、Web 静态资源、KDE 插件及 FFmpeg 运行时依赖说明。
- 提供 systemd user service；退出、升级和卸载不会遗留后台进程。

### M6：零拷贝和硬解

- 在 Mesa 与 NVIDIA 的至少一个受支持驱动组合上验证硬解、DMA-BUF modifier、acquire/release 同步和软解降级。
- 与 CPU RGBA 路径对比 CPU、GPU、内存占用和掉帧率，以数据决定默认启用策略。

## 11. 测试策略

- 单元测试：配置迁移、桌面检测、PTS 换算、循环时钟重置、缩放矩形、后端选择。
- 集成测试：用短视频 fixture 驱动 FFmpeg 到 headless backend，验证帧顺序、EOF、seek、背压和取消。
- 协议测试：模拟 WebSocket、KDE 插件和 Wayland buffer 生命周期中的重复消息、断连和超时。
- 真机矩阵：niri/KDE，单屏/双屏，1x/分数缩放，Mesa/NVIDIA，软解/VA-API/Vulkan。
- 稳定性测试：连续播放、反复切换视频、显示器热插拔、compositor 重启、daemon 强制退出后重启。

## 12. 关键风险与约束

- KDE 与 niri 的“背景层”机制不同，必须维持独立后端；强行统一为 layer-shell 会产生兼容性问题。
- FFmpeg Rust 封装仍受系统 FFmpeg ABI 和发行版开发包影响；CI 和打包必须固定最低支持版本，并测试缺失库时的错误信息。
- DMA-BUF 不是首版前提。先用 CPU RGBA 路径验证行为，再优化零拷贝，能显著降低调试范围。
- Wayland buffer 生命周期、显式同步和多 GPU 设备选择是最主要的资源安全风险；每种 buffer 必须具有清晰的所有权和 release 状态。
- Web 服务默认只能绑定 loopback；若未来允许远程访问，必须另行设计认证、CSRF 和文件访问边界。

## 13. 首轮开发顺序

按 `M0 → M1 → M2(headless) → M3(niri wl_shm) → UI → M4(KDE) → M6` 推进。最先建立“配置 → FFmpeg → 有界帧队列 → headless sink”的可测试闭环，再接 niri；不要在桌面输出链路尚未正确前投入硬解和 DMA-BUF 优化。
