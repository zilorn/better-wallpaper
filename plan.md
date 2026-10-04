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

兼容报告中的等级按场景实际识别到的已支持功能取最高层级，支持项、不支持项与警告同时展示；混合场景不会因某个不支持项抹去已有的支持能力。当前报告可识别 L2 的透明度/相机缩放/背景淡入淡出标量时间线、引用纹理中的精灵动画和背景音频，以及 L3 的白名单 2D 特效与原生识别的音频频谱模式。L3 标记不表示整个 L3 阶段已完成，也不代表资源完整、当前后端可播放或视觉完全一致；粒子、任意脚本、自定义 shader 等限制仍单独列出。尚未使用已验证 L4 能力评级。

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
- Plasma 需新增场景提交路径，共享同一 Rust 场景渲染核心；当前 QML 通过 Qt Multimedia 播放 HTTP 视频，未实现原生场景提交。
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

### 已完成 — Library API 未知属性序列化修复 (2026-07-04)

- 修复 `PropertyType::Unknown(String)` 与 Serde 内部 tag 不兼容导致 `/api/v1/library` 返回 500 的问题，未知类型改为稳定对象 `{kind: "unknown", value: "..."}`。
- 新增未知项目属性序列化回归测试，并通过 `packaging/install.sh` 部署。
- 已对运行中的管理端点实测：HTTP 200、响应 208569 字节，4 个未知属性安全保留；近期 service 日志无 error/panic。

### 已完成 — Water Effect 遮罩多纹理提交 (2026-07-04)

- Scene IR 保留 waterwaves/waterripple/waterflow pass 的 mask、normal 与 phase 纹理逻辑名，路径继续执行逃逸校验。
- 共享场景资产层解析并校验水效果 TEX；GPU 缓存上传主纹理和 effect 纹理，fragment shader 使用独立纹理单元采样 water-wave mask/normal 与 water-flow mask/phase。
- waterwaves、waterripple 与 waterflow 的基础位移现在按局部遮罩强度生效，不再对整个图层统一形变；同一图层可组合 ripple 与 flow 遮罩。
- waterripple normal 与 waterflow phase 已参与逐像素方向场，并使用 repeat 采样；mask 保持 clamp 采样。参数映射基于当前合法样本，仍需 golden image 校准后才能宣称与官方效果等价。
- workspace 共 123 项测试通过，相关格式、renderer 与 GPU crate 严格 Clippy 通过。

### 已完成 — 当前实机壁纸 Waterwaves / Waterripple 动画 (2026-07-04)

- 针对当前实际加载的 Workshop 场景 `2919051696` 检查 `scene.json`，确认其主要动画不是精灵图或 scroll，而是背景 waterripple/waterflow 与前景 6 层 waterwaves。
- Scene IR 解析真实 waterwaves 的 direction、scale、speed、strength，并解析 waterripple 的 scroll direction、scale、animation/scroll speed 与 ripple strength。
- OpenGL ES fragment shader 使用共享场景时钟执行纹理位移；暂停会冻结动画，时间相位限制在一小时内避免长期精度下降。
- 已通过 `packaging/install.sh` 实机部署并重启服务；日志确认当前场景 `draw_count=9`、`animated_draw_count=4`，EGL OpenGL ES 3.2 初始化成功且无 shader/GL 错误。
- 将每帧 `scene frame rendered` 从 debug 降为 trace，避免 60 FPS 场景淹没 journal。
- 当前 waterwaves/waterripple 已采样遮罩与可用法线，waterflow 已使用参数、遮罩和 phase 方向场；粒子、composelayer 和音频条仍未实现，且视觉参数尚待 golden 校准，不能宣称已完整还原该壁纸。

### 已完成 — Waterflow 基础动态链路 (2026-07-04)

- 根据当前真实场景中的 `effects/waterflow/effect.json` 解析 `phasescale`、`speed` 与 `strength`，并对非有限值、非正 phase scale 和负 strength 执行拒绝校验。
- waterflow 与同图层 waterripple 保持为两个独立 IR 参数，GPU 在同一 draw 中顺序组合两种时间驱动形变；暂停与多输出继续共享场景时钟。
- fragment shader 使用有界流场近似基础动态，workspace 共 121 项测试通过。
- 当前已采样 waterflow mask 与 phase 方向场；仍需参考帧校准强度、相位速度和坐标变换。

### 已完成 — 真实 Scroll 动效与项目属性运行时接入 (2026-07-04)

- 根据真实 Workshop `effects/scroll/effect.json` 实例实现首个官方 effect 动态路径：解析启用状态、`speedx`、`speedy` 与 `repeat`，GPU 使用共享场景时钟滚动 UV，并按 draw 切换 repeat/clamp 采样。
- 使用真实包复测确认 `3098540596` 的活动 scroll 图层进入 Scene IR；长时间运行时滚动相位取模，避免浮点精度随运行时间持续恶化。
- `scene.properties` 现覆盖场景中 `{"user":"key","value":fallback}` 绑定的 bool、number 与 string/vector fallback；保存配置后的热重载会重建场景并应用属性，不再只是持久化未消费字段。
- Wallpaper Engine 库 API 返回 `project.json/general.properties`，Web 场景配置根据 slider、bool、combo、color、file 与 textinput 生成项目属性控件。
- 场景质量档位已进入 niri 运行时：low/medium/high 分别限制为 30/45/60 FPS；日志记录加载的场景配置及属性覆盖数量。
- 真实样本证明 `cropoffset` 已烘入节点 `origin`，不得重复叠加；角度单位由 `1.57080/3.14159` 样本确认是弧度。仍有位置错误的图层主要依赖尚未实现的 transform effect、puppet/3D 或脚本求值。
- 当前 scroll 是完整动态实现；shake、waterwaves、pulse 等遮罩效果仍需离屏 render pass，不能用整图几何抖动冒充。

### 已完成 — 真实 Workshop 动态资源审计与解析纠偏 (2026-07-04)

- 使用本机 24 个合法 Workshop `scene.pkg` 做只读检测，而非以测试 fixture 推断真实格式；确认常见动态效果依次包含 waterwaves、shake、transform、foliagesway、pulse 等。
- 确认 `animationlayers` 在复杂样本中引用 `.mdl` puppet 动画 ID，并受用户属性与 SceneScript 控制；当前将其明确报告为不支持，不再把字段静默当成已渲染。
- 真实单通道动态遮罩的 LZ4 压缩比可超过 250:1；压缩比阈值按证据调整为 512:1，同时把单 mip 硬上限从 256 MiB 收紧为 64 MiB。抽查 5 个大型真实场景共 165 张纹理，解析失败数从多项降为 0。
- 修正绑定布尔值的初始可见性：`visible: {"user": ..., "value": false}` 不再错误地显示图层，避免真实场景出现隐藏层叠影。
- `scene-inspect` 新增受限 `--entry` 与 `--textures` 模式，用于继续审计真实包内 JSON、纹理尺寸、存储填充和动画元数据，不解包到文件系统。
- 真实样本显示主要动态来自官方 effect shader、粒子和 puppet，而非 TEX 精灵帧；下一步优先建立离屏 effect pass，不能用整图位移冒充遮罩驱动的局部 shake/waterwaves。

### 已完成 — RGB 修复与首个实机动态场景 (2026-07-04)

- 修正格式 0 裸纹理的实际 RGBA 字节序，不再错误交换红蓝通道；内嵌 PNG 解码结果也显式标记为 RGBA8。
- 支持逻辑尺寸与 GPU 对齐存储尺寸不同的纹理，仅在字节数精确匹配头部存储尺寸时接收，并通过 UV 排除填充区域。
- 新增完全自构造的两帧动态场景 fixture/generator，按水平图集正确排列不对称青色与橙色帧；动画 UV 收进半个 texel，避免线性采样串入相邻帧。
- 实机 niri 日志确认 `draw_count=1`、`animated_draw_count=1`，动态场景以 60 FPS 持续渲染且无 OpenGL 错误。
- Workshop 场景仍有大量粒子、合成层、puppet 和脚本动画未支持；本项只证明基础动画纹理端到端可运行。

### 已完成 — 场景配置 API 往返与 Web 控制面 (2026-07-04)

- Web 配置模型补齐 `scene`，避免保存其它设置时丢失场景配置。
- 当前壁纸为场景时显示质量、粒子上限、鼠标、视差、音频响应和实验脚本开关；脚本开关明确标注尚未执行。
- `SceneConfig` 新增确定性 `[scene.properties]` 字符串覆盖表，为后续按项目元数据生成用户属性表单提供持久化基础。
- daemon 保存前限制粒子上限、属性数量、键和值长度，防止配置绕过运行时资源边界。
- workspace 共 112 项测试和 Web TypeScript 检查通过。
- 当前开关仅完成配置与热重载控制面；视差、音频响应、粒子和脚本运行时仍未实现。

### 已完成 — niri 动画纹理播放与共享场景时钟 (2026-07-04)

- 场景资产解析不再丢弃 `.tex` 的 `TEXS` 动画帧，新增经过边界与有限数校验的精灵帧 UV/时长 IR。
- GPU 每帧按单调场景时间选择精灵帧并更新 UV；纹理只上传一次，不产生逐帧 CPU 解码或 GPU 重传。
- 动画按各帧实际时长循环，非法/越界帧安全降级为静态纹理，不影响同场景其它图层。
- niri 多输出共享同一个场景时间，播放控制暂停时冻结时间，恢复后不会产生时间跳跃。
- 新增帧时长循环和子矩形 UV 回归测试；相关 renderer、GPU、Wayland 与 daemon 测试共 34 项通过。
- 当前动态范围为动画纹理；对象时间线、视频纹理、视差和粒子仍待后续实现。

### 已完成 — 热重载资源回收与场景图层级降级 (2026-07-03)

- 配置保存、更换壁纸和静音继续通过播放控制信号热重载，无需重启 daemon。
- 场景播放结束时先释放每个输出的 EGL/GPU 对象，再释放共享 CPU 纹理；新配置只在旧管线返回后构建。
- 修复场景渲染结束后错误落入视频解码路径的问题。
- 单个图片节点遇到缺失依赖、未知混合、多纹理或无效纹理时仅跳过该节点并记录英语警告，其余已支持节点继续渲染。

### 已完成 — niri 最小 2D 纹理坐标、透明度与 resize 修正 (2026-07-03)

- 场景 GPU 提交改为每个四边形携带独立 UV，不再使用固定 1920×1080 屏幕坐标采样纹理。
- shader 应用场景图继承后的图层透明度，并继续由已验证的 opaque、translucent、additive 模式控制混合。
- niri 输出尺寸变化时同步更新 EGL window、GL viewport 状态，避免热插拔或模式切换后沿用旧尺寸。
- 新增 CPU 侧顶点/UV 交错布局回归测试；workspace 共 105 项测试通过。

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
- 场景对象与特效资源路径统一执行逃逸校验，并对照包索引输出确定性引用完整性报告；缺失资源会使校验结果标记为无效。
- 新增统一纹理上传 IR，严格校验原始与 BCn mipmap 字节数，并显式携带色彩空间和 alpha 语义；无格式证据时标为 unknown，块压缩数据保持原样供 GPU 上传。
- scene-format 单元测试增至 33 项，格式化、测试和该 crate 严格 Clippy 通过。
- 新增 CPU 资源缓存：规范路径与内容哈希组成缓存键，预算硬上限为 512 MiB，支持精确字节记账、最近最少使用驱逐和超大单资源拒绝。
- 新增 renderer 侧 GPU 资源缓存：显存预算硬上限为 2 GiB，按规范路径与内容哈希标识资源，支持精确记账、确定性 LRU、替换/移除/清空，并通过所有权保证驱逐时释放后端句柄。
- Phase 2 尚未完成：模型 JSON 解析仍待合法样本证据，避免在未知格式上猜测字段语义。

### 已完成 — 格式发现与解析器增强 (2026-07-01)

基于真实 Workshop 场景 `431960`（60+ 子项目）的深入分析，发现并适配了以下格式细节：

#### Wallpaper Engine 场景项目结构

- 每个 Workshop 子项目是独立目录，含 `project.json` + 入口文件
- 壁纸类型三种：`video`（MP4）、`Web`/`web`（index.html）、`scene`（scene.pkg）
- `scene.pkg` 是 PKGVxxxx 二进制包，内部包含完整的文件树

#### 场景对象字段发现

通过实际 137 对象的复杂场景（`3492627662`），确认了以下对象字段：
- 基础字段：`id`, `name`, `parent`, `visible`, `origin`, `size`, `scale`, `angles`, `alpha`
- 图层/渲染：`image`, `effects`, `castshadow`, `color`, `colorBlendMode`, `parallaxDepth`
- 动画/模型：`animationlayers`, `attachment`, `bones`, `bone_animations`, `transform`
- 其它：`container`, `collision`, `particle`, `sound`, `text`, `instanceoverride`

#### 属性绑定与脚本驱动值

Wallpaper Engine 允许属性值为对象格式以支持动态绑定：

```json
// SceneScript 驱动：运行时求值
{"script": "// update(value) { ... }", "value": "1920 1080 0"}

// 用户属性绑定：连到 UI 控件
{"user": "propertyKey", "value": 1.0}
```

解析器现在对所有向量/标量字段（origin, size, scale, angles, alpha）自动解包 `.value`。

#### 资源引用格式

- 标准格式：`"image": "models/bg.json"`（字符串）
- 数组格式：`"sound": ["sounds/bgm.mp3"]`（单元素数组，已适配）

#### 代码改进

- `SceneNodeKind` 新增 `Model`、`Container` 变体
- `KNOWN_FIELDS` 从 15 扩展到 25 个字段（animationlayers, attachment, castshadow, collision, color, colorBlendMode, parallaxDepth, transform 等）
- `parse_vec3_lenient` 支持 2 分量 origin（2D 场景常见，z 补 0）
- `unwrap_script_value` 统一处理脚本/属性绑定对象
- `string_resource` 支持单元素数组格式
- `parse_project_properties` 新增 project.json 用户属性解析（slider/bool/combo/color/file/textinput/text/group）
- 单元测试从 33 增至 43 项（含 2D origin、脚本驱动值、容器/模型检测）
- `scene-validate` 成功通过 137 对象 / 144 文件 / 38MB 真实场景完整解析

### 已完成 — Phase 2：模型资源 IR 与递归引用校验 (2026-07-01)

- 基于合法本地样本确认并实现 `models/*.json` 解析，覆盖 `autosize`、`cropoffset`、`material` 与可选 `puppet` 字段。
- 模型中的材质和 puppet 路径执行与场景资源一致的逃逸校验；非有限偏移、错误类型和缺失材质会被拒绝。
- 未知模型字段进入确定性排序的 `UnsupportedFeature`，不静默采用默认语义。
- `ModelManifest` 按规范路径稳定排序，`scene-validate --json` 输出模型 IR，并递归检查模型到材质/puppet 的资源依赖。
- 修正 JSON 资源分类：`models/*.json` 与 `materials/*.json` 分开标识，其它未知 JSON 不再误报为模型。
- scene-format 单元测试增至 47 项，严格 Clippy 通过；137 对象真实样本中的 23 个模型 JSON 全部解析成功。
- Phase 2 的格式与资源 IR 范围完成；下一步进入 Phase 3 的最小 2D 渲染路径。内置引擎虚拟资源（如 `models/util/solidlayer.json`）仍需在渲染阶段提供受控实现。

### 已完成 — Phase 3 起步：共享 2D 绘制计划 (2026-07-01)

- `better-wallpaper-renderer` 新增后端无关 `Scene2dPlan`，把场景 IR 确定性转换为按源顺序排列的 NDC 四边形；niri 与 Plasma 后续共享该语义。
- 实现正交相机中心/投影尺寸、节点平移、Z 旋转、XY 缩放、父子变换、继承可见性与透明度。
- 严格拒绝重复节点 ID、缺失父节点、父子环、非法投影与越界透明度；非图片节点和缺少尺寸的图片显式计入跳过统计。
- 当前 IR 尚无经过样本验证的锚点字段，因此暂用“尺寸以节点局部原点为中心”的明确语义，后续按合法样本补充，不猜测字段。
- Web 库类型补齐 `scene`，场景卡片显示兼容等级、不支持功能数与解析警告数，且不启用视频悬停预览。
- renderer 单元测试增至 10 项，严格 Clippy 通过；workspace 共 99 项测试通过。Web 构建因当前环境未安装 `bun` 未执行。
- Phase 3 尚未完成：材质/纹理解码到 GPU 纹理、混合与实际 draw submission、离屏 golden、niri/Plasma surface 接入仍待实现。

### 已完成 — Phase 3：基础材质 IR 与纹理依赖 (2026-07-01)

- 基于合法本地样本实现图片模型实际引用的材质 IR，覆盖 pass、opaque/translucent/additive 混合、shader 标识和纹理逻辑名。
- 未知混合模式与材质/pass 字段被显式保留为不支持语义；纹理引用继续拒绝绝对路径、盘符、NUL 与 `..`。
- 材质解析范围严格限定为模型实际引用的 JSON，避免把 `materials/` 下的特效和粒子预设误判为图片材质。
- 递归资源校验现覆盖 scene → model → material → texture，并按样本确认纹理逻辑名相对 `materials/` 资源根解析。
- `scene-validate --json` 增加确定性材质 IR；137 对象真实样本的图片材质与纹理引用全部解析成功，剩余缺失项仅为待实现的内置虚拟模型。
- scene-format 单元测试增至 50 项，workspace 共 102 项测试通过；Phase 3 下一步为纹理解码上传与实际 GPU draw submission。

### 已完成 — niri / Plasma 场景能力边界接入 (2026-07-01)

- daemon 在两种桌面后端启动场景前统一执行安全包解析、Scene IR 解析和共享 `Scene2dPlan` 构建，不再把场景项目目录错误交给 FFmpeg。
- niri 在实际纹理提交完成前保持现有桌面 surface 不变，并输出英语能力日志；Plasma 控制平面保持在线，插件显示明确的场景未就绪状态。
- Plasma 配置 API 增加 `scene_rendering_available` 能力字段，QML 仅对 `video` 设置媒体源，避免将场景目录作为视频播放。
- Web 当前壁纸类型标识补齐 `SCENE`。下一步仍是纹理解码、实际 draw submission 以及将能力字段切换为可用。

### 已完成 — Phase 3：共享场景资产提交 IR (2026-07-01)

- 新增后端无关 `Scene2dAssets` / `Scene2dDraw`，将绘制四边形沿 `scene → model → material → texture` 完整解析为同一份 CPU 提交数据。
- 每个 draw 携带 NDC 顶点、透明度、规范纹理路径、混合模式及已通过尺寸/格式校验的 mipmap；niri EGL 与 Plasma QSG 后续不再各自解释私有格式。
- 材质纹理路径解析规则从格式 crate 统一导出，避免桌面后端出现路径语义分叉。
- 当前最小提交范围严格限定为单 pass、单纹理和已知混合模式；未知混合、多纹理、缺失模型/材质/纹理均返回稳定错误，不静默错误渲染。
- daemon 的 niri / Plasma 场景预检现同时执行共享资产解析，并以英语日志记录可提交 draw 数或精确降级原因。
- 新增完整最小包测试，覆盖模型、材质、半透明混合、TEX 解析到提交 IR；workspace 共 103 项测试通过。
- 仍未完成实际 GL 多纹理上传与 draw submission，因此 Plasma 能力字段继续保持 false，niri 也不会覆盖当前 surface。

### L2 media follow-up

Package video textures now resolve from direct media layers/material references
and TEX video payloads into shared RGBA frames. The niri daemon uses bounded FFmpeg
queues and the shared scene clock for pause, PTS scheduling and loops. Package
media decoding excludes playlists/network access and external MOV references.
Original generated video tests cover clock freeze, loop boundaries, shared outputs
and cancellation. Eight-hour playback and real GPU/hotplug validation are pending;
these unit checks do not complete the phase 4 exit criterion.

### L2 basic parallax follow-up

Camera amount/delay/mouse influence and explicit per-layer `parallaxDepth` now
feed shared scene-clock smoothing and GPU translation. The niri backend consumes
standard Wayland pointer events on the wallpaper surface; the saved mouse/parallax
switches gate this behavior and the Web UI enables them. Tests cover axes, pause,
frame-rate independence, input loss and stale events after surface replacement.
The approximation is bounded; desktop-wide tracking over other windows and
reference-sample motion parity remain unverified. Particle systems, SceneScript,
Spine/Spriter model nodes, 3D/bones, container child groups, unlisted effects,
model animation layers and attachments remain outside this L2 follow-up.

### Golden/SSIM verification follow-up

The repository now has explicit surfaceless GLES golden regressions in
`crates/gpu-renderer/tests/scene_golden.rs`, ten independently generated synthetic
PNG references and local SSIM/RGBA error gates (SSIM ≥0.995, MSE ≤1, max error ≤2).
They cover transforms, rotation, blend alpha/order, cover/resize, UV padding and
orientation, scalar timelines, sprites, basic parallax, embedded video texture
updates and context replacement. All fourteen comparisons pass exactly on
llvmpipe. They exposed and fixed incorrect translucent framebuffer alpha.
Stage 3 reference-screenshot parity still needs legal authored Wallpaper Engine
captures and vendor GPU runs; the synthetic regression harness alone does not
fulfill that exit condition. Stage 4 eight-hour playback remains unperformed.
