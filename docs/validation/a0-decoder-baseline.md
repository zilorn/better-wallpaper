# A0 解码与测试基线

验证日期：2026-10-18。提交：`39a9faa`（帧契约与高度限制）、
`f4b0295`（资源服务关闭回归）。本记录完成适配计划 A0 的测试基线子项，
不代表 A0 的资源审计和原程序参照捕获已完成。

## 环境与样本

- Linux，本机 NVIDIA GeForce RTX 3060 Ti，驱动 615.78.08。
- Rust 1.97.1（8bab26f4f）、FFmpeg n9.0.2；Rust `ffmpeg-next` 9.0.0。
- 离屏 GLES：llvmpipe，LLVM 23.1.1，256 bits。
- 样本均为仓库现有文件，未修改二进制资源或 GPU 参照图。
  [fixtures 说明](../../tests/fixtures/README.md)记录 SHA-256、尺寸、时间和输入设置。
- 全量测试及真实 CUDA 验证在允许 GPU、符号链接和本地套接字访问的环境运行。
  默认沙箱内 CUDA 初始化报告 `CUDA_ERROR_NO_DEVICE`；该环境中的硬件请求通过
  软件回退，因此不能单凭沙箱内测试成功宣称验证了 CUDA。

## 已观察到的失败与修复

### CPU/CUDA 存储契约

旧 `hardware_request_decodes_or_falls_back_to_software` 在真实 CUDA 下可复现：
CPU `pixels.len()` 为 0，测试却要求 9216（64×36×4）。
`decoder.rs::receive_frame` 为 NV12 硬件帧保存 FFmpeg 帧引用，返回两个 CUDA
设备平面；`EglRenderer::render` 根据 `DecodedFrame.cuda` 选择专用上传入口。
因此空 CPU 缓冲是有效行为，`format` 仅描述 CPU 存储。

更新后的测试分别断言 CPU RGBA 的尺寸、步长、字节数和不透明 alpha，或 CUDA
NV12 的非空独立平面、足够的字节 pitch、空 CPU 缓冲和可保留的 owner。完整读取
五帧，检查递增 PTS/time base、seek 回首帧，以及 seek/decoder 销毁后保留帧的存储。
硬件请求不强制要求 CUDA，以允许无设备机器运行；独立子进程设置
`CUDA_VISIBLE_DEVICES=-1` 并强制要求 CPU 输出，覆盖有 GPU 主机上的回退路径。

这验证 CUDA 存储元数据及引用生命周期，不读取 CUDA 像素，也不证明 CUDA/GL
互操作最终画面的颜色或实际 niri 呈现正确。

### 高度限制

新增 `max_height=18` 的回归在修复前分别得到：

- CPU：`Pixel format conversion returned 9/18 rows`。
- CUDA：仍输出 64×36，而预期为 32×18。

修复 CPU 缩放调用的源 slice 高度为完整源帧高度；需要缩小时，硬件帧走已有的
回传及 CPU RGBA 转换路径。未缩小的 NV12 硬件帧继续保留 CUDA 平面。
真实 GPU 上 `max_height=0/72` 输出 CUDA NV12，`max_height=18` 输出 32×18 CPU
RGBA；CPU 路径相同尺寸检查通过。72 的限制不放大原片。
硬件缩小时会产生 GPU→CPU 传输开销，不称为零拷贝缩放。

### 资源服务关闭测试竞争

首次全量运行在 `web_projects_keep_nested_paths_defaults_and_asset_boundaries`
的“销毁后立即连接失败”断言处失败，随后完整重跑通过。检查本地 tiny_http 0.12.0
源码可见 `Server::drop` 设置关闭标记并连接自身唤醒独立 accept 线程，不 join 该线程。
应用自己的 asset worker 已被 join，底层 listener 仍可能短暂存在。

测试现以 50 ms 单次连接超时、10 ms 重试间隔、2 秒总期限确认拒绝连接；
永久不关闭或出现其他连接错误仍失败。实际服务实现和资源权限边界未改变。
修复后此用例在 30 个独立测试进程中连续通过。

## 验证命令与结果

从仓库根目录执行：

```sh
cargo clippy
cargo fmt --all -- --check
cargo test -p better-wallpaper-ffmpeg --test fixture_decode -- --nocapture
cargo test --workspace
LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture
```

| 检查 | 结果 |
| --- | --- |
| Clippy / 格式 / `git diff --check` | 通过 |
| 解码 fixture 集，沙箱软件回退 | 9 通过、0 失败 |
| 解码 fixture 集，真实 CUDA 可访问 | 9 通过、0 失败；输出日志区分 CUDA NV12 与 CPU RGBA |
| 最终全量 Rust 测试 | 209 通过、0 失败；普通套件按既有设置忽略 1 个需显式运行的 GPU 用例 |
| 显式离屏 GPU 用例 | 1 通过；20 次比较全部 SSIM=1、MSE=0、最大通道误差=0 |
| 资源服务关闭回归重复运行 | 30/30 通过 |

重复关闭回归可使用以下单次命令，在独立进程中执行 30 次；每次必须成功：

```sh
cargo test -p better-wallpaper-daemon --lib web_wallpaper::tests::web_projects_keep_nested_paths_defaults_and_asset_boundaries -- --exact
```

## 尚未验证

本次没有捕获 Wallpaper Engine 原程序输出，也没有执行 niri/Plasma 桌面生命周期、
真实 CUDA/GL 呈现、厂商 GPU golden、8 小时播放或 24 小时 fuzz。现有合成 golden
只能证明测试覆盖的绘制行为未回归。散装 TEX 和选定 PKG 的有界审计清单、引用关系、
适配行为样本矩阵及原程序参照仍需按 A0 后续子项推进。
