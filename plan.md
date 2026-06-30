# Wallpaper Engine 场景壁纸独立实现计划

## 1. 目标与原则

目标是在 Better Wallpaper 内部独立实现 Wallpaper Engine `scene` 类型壁纸的解析、资源加载、动画、交互与渲染，支持 niri 和 KDE Plasma 6，同时保持现有视频、网页壁纸功能不回退。

硬性原则：

- 不依赖、不调用、不复制任何其他 Wallpaper Engine 兼容项目的代码、库、可执行文件与格式实现。
- 仅依据合法取得的本地 Wallpaper Engine 项目、公开文档、黑盒行为观察和自行编写的测试工具研究格式。
- 不分发 Wallpaper Engine 或 Workshop 资产；用户必须自行合法安装和订阅内容。
- 所有不可信输入均按敌对数据处理；解析器不得执行系统命令或暴露文件系统。
- Rust/daemon 日志与 Web 控制台日志使用英语，不记录 Web 路由访问日志。
- 安装相关修改统一放在 `packaging/install.sh`。

## 2. 支持范围

Wallpaper Engine 场景包含图层、特效、时间线、视差、粒子、音频响应、3D 模型、SceneScript 和自定义 shader，完整复刻属于渲染引擎级工程，不应一次性承诺。官方能力参考：[场景概览](https://docs.wallpaperengine.io/en/scene/overview.html)、[粒子系统](https://docs.wallpaperengine.io/en/scene/particles/introduction.html)、[SceneScript](https://docs.wallpaperengine.io/en/scene/scenescript/introduction.html)、[Shader](https://docs.wallpaperengine.io/en/scene/shader/overview.html)。

按兼容等级逐步交付：

| 等级 | 支持内容 | 明确不支持 |
|---|---|---|
| L0 | 发现项目、解析元数据、预览图、兼容性报告 | 场景播放 |
| L1 | 2D 图片图层、变换、颜色、透明度、混合、基础相机、静态合成 | 粒子、脚本、3D、自定义特效 |
| L2 | 时间线、精灵图、视频纹理、音频、基础视差、常用用户属性 | SceneScript、复杂粒子、3D |
| L3 | 基础粒子、音频频谱响应、常用官方 2D 特效、鼠标交互 | 任意 SceneScript、3D、自定义 shader |
| L4 | 受限 SceneScript、3D 模型、受控 shader 子集 | 未验证或危险能力 |

首个可发布版本以 L2 为目标。L3/L4 只有在格式、稳定性、安全和测试样本满足阶段门槛后再进入开发。

## 3. 仓库现状与需要调整的模型

当前已有 Steam Workshop 根目录扫描、`project.json` 解析、预览图、Web UI、视频解码、niri layer-shell、Plasma 插件和配置热更新。当前 `scan_wallpaper_engine_library` 会跳过 `type: "scene"`，`WallpaperType` 也只有 `Video`、`Web`。

配置升级到版本 2，并用类型代替 `engine_mode` 布尔值：

```toml
[wallpaper]
kind = "scene"                 # video | web | scene
source = "/.../431960/123456" # scene 保存项目目录

[scene]
quality = "high"              # low | medium | high
mouse = true
parallax = true
audio_processing = true
particle_limit = 10000
script_enabled = false

[scene.properties]
rain = "1"
schemecolor = "0.15 0.23 0.40"
```

提供显式 v1 -> v2 迁移并测试 video/web 无损迁移。库 API 增加 `compatibility_level`、`unsupported_features`、`parse_warnings` 和场景版本信息。

## 4. 内部架构

新增独立 crate，避免格式解析、场景求值和桌面呈现互相耦合：

```text
better-wallpaper-scene-format   不可信文件解析、资源索引、兼容性分析
            |
better-wallpaper-scene-runtime  场景树、时间线、属性、粒子、脚本调度
            |
better-wallpaper-renderer       GPU 资源、渲染图、shader、后处理
            |
better-wallpaper-wayland / kde  niri 与 Plasma 呈现
```

关键领域类型：

- `ScenePackage`：包头、版本、条目索引和受限资源读取接口。
- `SceneManifest`：项目元数据、入口、依赖资源和兼容性结果。
- `SceneGraph`：相机、节点、图层、材质、效果及父子关系。
- `SceneClock`：单调时钟、暂停、恢复、循环和固定步长模拟。
- `RenderGraph`：离屏 pass、纹理依赖、混合和最终合成。
- `SceneSession`：加载、播放、暂停、停止、热重载和统计。

格式 crate 不创建 GPU 对象、不解码媒体、不访问项目目录之外的路径；运行时只消费经过校验的中间表示。

## 5. 分阶段实施

### 阶段 0：合规研究与样本体系（3～5 天）

- 建立研究记录，区分公开文档事实、自有样本观察和推断字段；每个字段记录样本证据与置信度。
- 准备至少 30 个合法本地样本，覆盖纯图片、混合图层、时间线、视频纹理、粒子、音频、属性、脚本、2D 特效和 3D。
- 仓库只提交自行构造的最小 fixture、字段摘要、哈希与预期截图，不提交 Workshop 原始资产。
- 编写只读 `scene-inspect` 工具，输出 `project.json`、包头、条目边界、文件类型猜测、未知字段和十六进制区段；任何猜测必须标注为 unknown/inferred。
- 记录 Wallpaper Engine 的参考截图或帧哈希作为黑盒基准，不进行进程注入、内存抓取或绕过保护。

退出条件：至少能稳定区分已知/未知包版本，样本不会导致探针崩溃或越界读取。

### 阶段 1：安全包解析器与 L0（1～2 周）

- 新建 `better-wallpaper-scene-format`，实现包头、版本、索引、路径、偏移、长度、压缩标志和校验信息解析。
- 先实现零拷贝索引和按需读取，再按样本证据实现必要的解压算法；所有长度计算使用 checked arithmetic。
- 限制单条目、总解压大小、压缩比、嵌套深度、文件数和字符串长度，防止 zip bomb 类资源耗尽。
- 拒绝绝对路径、`..`、NUL、符号链接逃逸、条目重叠、越界偏移和重复冲突路径。
- 识别 `type: "scene"` 并加入库；即使不兼容也显示预览和原因，不再静默跳过。
- 实现 `scene-inspect --json` 和 `scene-validate`，输出稳定错误码与英语结构化日志。
- 对解析入口加入 `cargo-fuzz`：包头、索引、字符串、解压和场景描述各一个 target。

退出条件：所有 L0 样本可扫描；损坏/恶意 fixture 无 panic、OOM、路径逃逸；模糊测试累计运行至少 24 小时无崩溃。

### 阶段 2：纹理与场景中间表示（1～2 周）

- 从包中定位场景描述、纹理、视频、音频和 shader 资源，未知资源保留原始类型标识但不加载。
- 将场景描述转换为版本无关 IR；未知节点或字段进入 `UnsupportedFeature`，不得用错误默认值悄悄渲染。
- 实现常见纹理格式、色彩空间、alpha 模式、mipmap、采样与寻址；优先复用安全、许可证兼容的通用图像解码 crate，但不引入其他 Wallpaper Engine 项目。
- 构建资源缓存：以规范路径和内容哈希为键，设置 CPU/GPU 内存预算和 LRU 驱逐。
- 为每个样本输出确定性的场景树快照，纳入 snapshot test。

退出条件：L1 样本的场景树与资源引用可完整解析，未知功能能被准确报告。

### 阶段 3：GPU 2D 渲染器与 L1（2～3 周）

- 扩展 `better-wallpaper-renderer`，实现正交相机、节点层级、位置/旋转/缩放、锚点、裁剪、颜色和透明度。
- 实现常用混合模式、纹理采样、sRGB/线性空间转换与离屏 framebuffer。
- 用 render graph 表示多 pass，避免将效果硬编码进桌面后端。
- niri 复用现有 layer-shell surface；场景渲染直接输出到每个 Wayland 输出，不经过 RGBA CPU 回读。
- Plasma 扩展当前 C ABI/QML 帧插件，共享同一 Rust 场景渲染核心；QML 只负责承载纹理和配置同步。
- 支持不同分辨率、DPR、旋转、cover/contain/stretch 和输出热插拔。
- 使用离屏 headless 渲染生成 golden image；比较时允许明确的 GPU 浮点误差阈值。

退出条件：L1 样本与参考截图达到既定 SSIM/像素误差门槛；niri 与 Plasma 不维护两套场景语义。

### 阶段 4：时间线、媒体与 L2（2～3 周）

- 实现单调 `SceneClock`、关键帧、插值、循环、播放速率和固定步长求值；暂停后恢复不得产生时间跳跃。
- 支持精灵图序列和动画纹理。
- 视频纹理复用 `better-wallpaper-ffmpeg` 解码、PTS 调度和有界队列；上传 GPU 后立即释放过期帧。
- 音频复用现有音频能力，统一静音、暂停、循环和设备错误恢复。
- 实现基础鼠标视差；坐标从 compositor/QML 正确映射到场景空间。
- 实现 color、slider、bool、combo、textinput、texture 用户属性；官方类型参考[用户属性文档](https://docs.wallpaperengine.io/en/scene/userproperties/overview.html)。
- `usershortcut` 永久禁止执行；texture 属性只允许选择已批准库根内的文件。

退出条件：L2 样本连续播放 8 小时，无音画时钟漂移增长、GPU 资源泄漏或暂停后突跳。

### 阶段 5：常用特效、粒子、音频响应与 L3（3～5 周）

- 根据样本频率排序实现官方常用 2D 特效；每个特效必须有字段说明、CPU 参考实现或离线 golden、GPU 实现和边界测试。
- shader 由项目内受审计模板生成，只暴露白名单参数；未知 shader 显示不支持，不直接编译执行包内任意源码。
- 粒子采用结构化 emitter/initializer/operator IR，使用固定步长和可复现随机种子。
- 设置全局与单系统粒子上限、生成速率、模拟步数和 GPU buffer 预算；超限时降级并记录限频英语日志。
- 用音频帧生成 FFT 频带和平滑包络，驱动白名单属性；静音与无输入时输出确定性零值。
- 鼠标交互只注入位置和受控按键事件，不提供系统 API。

退出条件：目标 L3 样本兼容率达到 80% 以上；4K/30 和 1080p/60 性能达到阶段 8 确定的预算。

### 阶段 6：受限 SceneScript 与 3D 可行性门（先研究，暂不承诺发布）

- 从公开语言定义和自有样本建立 SceneScript 语法/运行时需求表。
- 首选自行实现受限解释器；若采用通用 ECMAScript crate，必须是通用语言实现而非 Wallpaper Engine 兼容项目，并完成许可证与沙箱审查。
- 禁止网络、文件、进程、FFI、动态模块和 wall-clock 系统 API；只提供显式场景对象接口。
- 每帧设置指令、调用深度、对象数、内存和执行时间预算；超限终止当前脚本而不终止 daemon。
- 3D 先支持静态网格、相机、基本材质和骨骼动画，再评估灯光、阴影与高级材质。
- 自定义 shader 仅在完成独立验证、资源限制和 GPU 挂死隔离方案后开放受限子集。

退出条件：安全审查和 fuzzing 通过前，`script_enabled` 始终默认关闭，UI 清晰显示功能为实验性。

### 阶段 7：API 与 SolidJS UI（与阶段 1～6 同步）

- 库卡片显示“场景壁纸”、兼容等级、已支持/不支持功能和解析警告；场景不使用 HTML video 悬停预览。
- 增加场景详情接口，返回规范化元数据和兼容报告，不直接暴露内部文件路径。
- 增加用户属性表单、质量档位、粒子上限、鼠标、视差、音频响应和实验脚本开关。
- 状态页显示场景加载阶段、当前 FPS、帧时间、CPU/GPU 资源、活动节点/粒子数、掉帧和最近错误。
- 配置保存沿用现有原子写入与热重载；加载失败时保留旧场景并返回明确错误。

### 阶段 8：性能、稳定性与发布（1～2 周）

- 以现有视频路径为基线，在 Intel/AMD/NVIDIA 上测试 1080p/60、1440p/60、4K/30。
- 记录首帧时间、平均/99 分位帧时间、CPU、GPU、RSS、显存、掉帧、音画漂移和输出热插拔恢复时间。
- 增加低/中/高质量策略：渲染分辨率、特效 pass、粒子上限、视频纹理分辨率和 FPS。
- 稳定性场景覆盖 8 小时播放、100 次热切换、锁屏/解锁、plasmashell 重启、Wayland 断线和显示器热插拔。
- `packaging/install.sh` 安装新增 crate 产物或资源；`packaging/verify.sh` 校验 shader、配置迁移和场景探针；卸载不删除用户 Workshop 内容。
- README 明确兼容等级、已知限制、合法资产要求及非官方实现身份。

## 6. 测试体系

- **单元测试**：所有字段解析、checked arithmetic、路径校验、IR 转换、动画插值、属性校验和资源预算。
- **fixture 测试**：只用自行构造的最小包和损坏包，覆盖截断、越界、条目重叠、压缩炸弹、路径穿越和版本未知。
- **模糊测试**：格式解析、解压、纹理头、场景描述、属性和脚本入口。
- **快照测试**：场景 IR、兼容报告、渲染图与 shader 参数。
- **Golden 测试**：离屏固定时间点渲染，与合法参考截图比较。
- **集成测试**：场景加载/切换/失败回滚、暂停恢复、视频纹理、音频、配置热更新和多输出。
- **真实样本验收**：本地私有样本矩阵记录结果，CI 无资产时跳过，不伪造成功。
- **回归测试**：`cargo test --workspace`、Web 类型检查/构建、安装验证，以及现有 video/web 测试。

## 7. 安全边界

- 包和场景文件只通过受限 reader 读取，禁止任意路径访问。
- 所有数量、大小、时间步和递归均有硬上限；配置只能在更严格范围内调整，不能取消安全上限。
- 不执行 `usershortcut`，不加载原生库，不启动项目附带程序，不访问网络。
- 未知 shader 不执行；实验脚本运行在无系统能力、带预算的沙箱中。
- GPU 资源在提交前验证尺寸、格式、buffer 范围和 pass 数；异常场景可被取消并回收全部资源。
- 错误日志记录项目 ID、资源逻辑名、格式版本和错误类别，不泄露无关本地路径或内容。

## 8. 主要风险

| 风险 | 影响 | 控制措施 |
|---|---|---|
| 私有格式变化且缺少规范 | 新场景解析失败 | 版本化解析器、证据记录、未知字段保留、兼容报告 |
| 工作量被低估 | 长期无法交付 | L0～L4 分级，首版锁定 L2，不把实验能力算作完成 |
| 恶意或损坏包 | OOM、越界、GPU 挂死 | 严格预算、checked arithmetic、fuzzing、白名单 shader |
| niri/Plasma 渲染分叉 | 双倍维护和行为不一致 | 共享场景 runtime 与 renderer，后端只负责 surface 呈现 |
| 不同 GPU 结果不一致 | 视觉差异/崩溃 | 通用 shader 子集、离屏测试、多 GPU 矩阵、明确容差 |
| SceneScript 能力过大 | 任意代码执行或卡死 | 默认关闭、受限解释器、无系统 API、逐帧预算与熔断 |
| 版权与商标误解 | 分发风险 | 不提交/分发资产，文档标明非官方实现和合法安装要求 |

## 9. 首版完成定义（L2）

- `scene` 项目能被发现、预览、选择并显示准确兼容报告。
- 安全解析器通过恶意 fixture、24 小时 fuzzing 和资源上限测试，无 panic、OOM 或路径逃逸。
- 2D 图层、变换、混合、时间线、精灵图、视频纹理、音频、基础视差及常用用户属性工作。
- niri 与 Plasma 共享同一渲染语义；多屏、DPR、旋转、热插拔和 plasmashell 重启通过测试。
- 加载失败能够回滚到旧壁纸；daemon 退出后无资源残留。
- video/web 回归测试全部通过，v1 配置可无损迁移。
- 程序与 Web 控制台使用英语诊断日志，且没有 Web 路由访问日志。
- 文档不宣称 100% 兼容，并明确列出粒子、SceneScript、3D 和自定义 shader 的实际状态。

## 10. 预估

L0～L2 预计需要约 8～12 个工程周；L3 额外约 3～5 周；L4 无法在格式研究前可靠估算。该估算按一名熟悉 Rust、GPU 与 Wayland 的开发者计算，不包含跨 GPU/发行版的大规模兼容修复。

---

## 实施进度 (2026-06-30)

### 已完成 — Phase 1 核心：安全包解析器

#### 新 crate: `better-wallpaper-scene-format`
- **安全 .pkg 解析器** (`src/pkg.rs`):
  - 零拷贝 `PkgReader`：加载一次，按需切片读取
  - checked arithmetic 保护所有长度/偏移计算
  - 安全硬上限：条目数≤4096、文件名≤512B、单条目≤64MB、总量≤128MB
  - 路径安全：拒绝绝对路径、`..`、NUL 字节、Windows 盘符、重复文件名；反斜杠自动归一
  - 输出 `PkgInspectOutput` 支持 JSON 序列化
  - 14 项安全检查 + 读取测试

- **.tex 纹理解析器** (`src/texture.rs`):
  - TEXV0005 + TEXB0001~0004 容器
  - 全部 15 种像素格式（ARGB8888 ~ RGB161616f）
  - LZ4 解压缩 + 压缩比上限
  - TEXS0001~0003 动画帧 + 精灵图网格

- **场景元数据与兼容性** (`src/scene.rs`):
  - scene.json 字段提取（相机、投影、物体类型）
  - L0~L4 兼容等级计算
  - 未知功能报告（粒子、音效、文本、脚本、自定义 shader）

- **测试**: 25 个单元测试全部通过；scene-format crate 严格 Clippy 通过

- **CLI 工具**:
  - `scene-inspect [--json] <scene.pkg>` 输出包版本、条目边界及文件类型提示
  - `scene-validate [--json] <project-dir|scene.pkg>` 校验包与 scene.json，并输出兼容性报告
  - `packaging/install.sh` 安装两个工具

- **本轮审计修复**:
  - 包版本不再返回占位值，保留并报告完整 `PKGVxxxx`
  - 拒绝非空条目数据范围重叠
  - 修正 LZ4 压缩比限制方向，避免高扩张比纹理绕过资源限制
  - 纹理二进制读取改用 checked arithmetic，精灵图网格计算使用饱和乘法
  - 建立包头、索引、字符串、纹理/解压和场景描述共 5 个 cargo-fuzz target

### 已完成 — 核心库与扫描更新

#### better-wallpaper-core
- `WallpaperType` 新增 `Scene` 变体
- 新增 `SceneConfig`（quality, mouse, parallax, audio_processing, particle_limit, script_enabled）
- 配置版本 v1 → v2 自动迁移（engine_mode → wallpaper_type）
- 场景 TOML 序列化/反序列化测试

#### better-wallpaper-daemon 库扫描
- `scan_wallpaper_engine_library` 识别 `type: "scene"` 项目
- 场景条目路径设为项目目录，而非 scene.pkg
- `LibraryEntry` 新增 `scene_compatibility: Option<CompatibilityPayload>`
- 兼容报告包含等级（L0-L4）、不支持功能列表、警告
- 场景项目在 Web UI 中展示兼容性信息
- 全部 70 个 workspace 测试通过（含 25 个 scene-format 测试）

### 已知未完成项

- workspace 严格 Clippy 仍被 daemon 既有警告阻塞；scene-format crate 自身无警告。
- cargo-fuzz 目标已建立；当前环境未安装 `cargo-fuzz`，24 小时 fuzzing 尚未执行，阶段 1 不能据此判定完整退出。
- 当前只完成 L0 扫描与格式探针；尚无场景播放/runtime/renderer，不能宣称 L1 可播放。

### 已完成 — Phase 2 起步：场景 IR 与资源清单

- 新增版本无关 `SceneGraph`，包含相机、节点、基础变换、父子引用、资源引用和效果标识。
- 场景资源引用执行路径逃逸校验；非有限数值和畸形向量被拒绝。
- 未知对象字段及未支持节点能力进入确定性排序的 `UnsupportedFeature`，不会静默渲染。
- 新增确定性 `ResourceManifest`，按规范路径排序并记录资源类型、大小和内容哈希；未知扩展被保留。
- `scene-validate --json` 现输出场景 IR 与资源清单，支持私有样本 snapshot。
- scene-format 单元测试增至 30 项，格式化、测试和该 crate 严格 Clippy 通过。
- Phase 2 尚未完成：模型 JSON、资源引用完整性、统一纹理像素 IR、资源预算与 LRU 仍待实现。
