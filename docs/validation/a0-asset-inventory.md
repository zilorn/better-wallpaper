# A0 有界资源清单

日期：2026-10-18。工具提交 `ca62639`；合成场景修复 `7918b37`。
本次完成 A0 的“对散装资源与选定 PKG 建立有界审计清单”子项。
记录覆盖、失败与未知，不要求所有样本均被解析器接受，也不等同于完成整个 A0。

完整机器清单：[a0-assets.json](a0-assets.json)，1,887,869 字节，SHA-256：
`6fd93241c3c5d8f3f9cd949b1ccc3f2dd2b76932a55854e4c970273807439bb5`。
两次独立运行的输出逐字节相同。文件仅含路径、哈希、格式元数据、引用与诊断；
没有商业壁纸资源字节、完整 JSON 文档或反编译输出。

## 输入边界与复现

本机 Steam 根目录为 `/home/zilorn/.local/share/Steam/steamapps`，以下简称
`$audit_steam`。原程序安装目录 `common/wallpaper_engine` 的已记录版本为
2.8.42 / Steam build 23967692，版本身份见[纹理调查](../reverse-engineering/wallpaper-engine-textures.md)。
本次只读取资源，没有运行原程序，也没有新增 REA 原生语义证据。

| 清单来源标签 | 相对 Steam 根目录的输入 | 选择说明 |
| --- | --- | --- |
| `engine-assets` | `common/wallpaper_engine/assets` | 递归散装 TEX/JSON；显式共享引用候选根 |
| `engine-projects` | `common/wallpaper_engine/projects` | 递归散装 TEX/JSON；按最近项目描述文件区分项目根 |
| `workshop-1731973973` | `workshop/content/431960/1731973973/scene.pkg` | 本机已安装场景包中按数字 ID 升序选择的前三项之一 |
| `workshop-2760105203` | `workshop/content/431960/2760105203/scene.pkg` | 同上 |
| `workshop-2801215039` | `workshop/content/431960/2801215039/scene.pkg` | 同上 |
| `synthetic` | 仓库 `tests/fixtures/dynamic-scene/scene.pkg` | 自主合成、确定性两帧图集 |

没有扫描所有 Workshop 项目，也未按失败数挑选包。这三个包只是有界起点，
不是随机样本、流行度统计或整体兼容率样本。

从仓库根目录运行（已有文件可能随 Steam 更新变化，以各条目哈希核对身份）：

```sh
audit_steam=/home/zilorn/.local/share/Steam/steamapps
cargo run --locked -p better-wallpaper-scene-format --bin asset-audit -- \
  --shared "engine-assets=$audit_steam/common/wallpaper_engine/assets" \
  --root "engine-projects=$audit_steam/common/wallpaper_engine/projects" \
  --pkg "workshop-1731973973=$audit_steam/workshop/content/431960/1731973973/scene.pkg" \
  --pkg "workshop-2760105203=$audit_steam/workshop/content/431960/2760105203/scene.pkg" \
  --pkg "workshop-2801215039=$audit_steam/workshop/content/431960/2801215039/scene.pkg" \
  --pkg synthetic=tests/fixtures/dynamic-scene/scene.pkg > inventory.json
```

本次预期退出码为 **2**：清单正常输出，但 32 个严格 JSON 解析失败使引用覆盖
不完整，`complete` 为 `false`。没有触及配置上限，没有读取或目录遍历错误，
`notices` 为空。不要将退出码 2 当作成功覆盖所有引用，也不要因此丢弃清单。
限额、退出码、路径规则及引用子集详见[工具契约](../asset-audit.md)。

默认上限为目录项 20,000、记录 10,000、深度 16、单文件 128 MiB、总读取 1 GiB、
单 TEX 累计保留 mip 128 MiB、引用 100,000。实际遍历 5,619 个目录项，读取
154,287,219 字节。1,537 个非 TEX/JSON 散装文件按选择规则排除；显式 PKG 的
全部成员均被记录。包成员与容器共享输入字节，不重复计入读取总量。

## 文件与格式结果

| 来源 | 记录数（PKG 含容器本身） | TEX 二维载荷通过 | TEX 被拒绝 | JSON 语法通过 / 失败 |
| --- | ---: | ---: | ---: | ---: |
| `engine-assets` | 2,009 | 283 | 28 | 1,667 / 31 |
| `engine-projects` | 388 | 129 | 0 | 258 / 1 |
| `workshop-1731973973` | 37 | 5 | 0 | 19 / 0 |
| `workshop-2760105203` | 99 | 28 | 0 | 49 / 0 |
| `workshop-2801215039` | 55 | 13 | 0 | 29 / 0 |
| `synthetic` | 5 | 1 | 0 | 3 / 0 |
| 合计 | 2,593 | 459 | 28 | 2,025 / 32 |

另外包含 4 个成功读取索引的容器及 45 个只作哈希记录的其他包成员。
440 个散装 TEX 仍为 412 个二维载荷通过、28 个体积/LUT 拒绝，与既有调查一致。
选定真实 PKG 新增 46 个通过的 TEX，自主合成包新增 1 个。

| PKG | 版本 / 成员数 | SHA-256 |
| --- | --- | --- |
| `1731973973/scene.pkg` | PKGV0002 / 36 | `4891b57732db7b27e2f2a5b91270cb2b0f911b3dab0c127e10fab7c0c9759ab9` |
| `2760105203/scene.pkg` | PKGV0018 / 98 | `b1e94def17a1c25b06c19a514b5d20d28f0ff508610afb2403d442e2f30b302b` |
| `2801215039/scene.pkg` | PKGV0016 / 54 | `d6a6df1da4e6979685524e9acf5d2da6bad28968517d6bbaec24e0eb22c24312` |
| `dynamic-scene/scene.pkg` | PKGV0001 / 4 | `2408281477093c829fa10ceeeadf590c4aa40e867bdf9e0f2e54ec1d558d3aae` |

每个成功读取的散装文件及包成员都有 SHA-256。包成员还记录在其父容器中的绝对
字节偏移；无需把商业资源解包到仓库。解析失败仍保留身份，例如 28 个体积 TEX
标为 `unsupported_volume`，不混入错误像素长度或缺少文件分类。

## 引用覆盖及发现

3,858 条是带 JSON pointer 的候选边，不是去重后的资源数：

- 2,149 条在同一来源或显式共享来源找到候选路径，标为 `candidate_found`。
- 1,657 条标为 `not_found_in_scope`。例如被排除的散装 shader/media 也可能被
  `dependencies` 引用；这个数字不能解释为 1,657 个安装资源缺失。
- 52 条为非文件或不安全路径值，包含空值、运行时 `_rt_` 目标等，标为
  `non_file_or_unsafe`，不触发文件访问。

候选匹配保留多个结果，不猜测原程序的覆盖顺序。独立 PKG 之间不会互相满足引用。
脚本/属性动态路径、未列出的字段、对象式纹理映射、TEX 编辑器内嵌 JSON，以及
严格 JSON 无法读取的文档都不在本轮引用发现范围内。

32 个 JSON 文件严格解析失败。已检查的例子：
`engine-assets/effects/blend/preview/effects/blend/effect.json` 包含行注释，
`engine-assets/effects/fluidsimulation/effect.json` 包含尾逗号，
`engine-projects/defaultprojects/fantasticcar/materials/car/glass.json` 包含行注释。
清单保留各自哈希及首个语法错误位置。本轮没有去掉这些语法后冒充完整解析，
也不据此断言原程序不接受这些资源。后续扩展语法/引用范围须有针对性样本回归。

另一个可修复发现是仓库合成包过期：旧 SHA-256
`4b45b05cfa39909e0e24802cdec56a723bb876a4a216546691d22e014ba5b51f`
中的动画记录仍使用旧坐标顺序，当前解析器将其报告为不支持的旋转/倾斜帧。
现有生成器已正确，但落盘包未同步。`7918b37` 重新生成该包，并加入读取实际
已提交字节的回归，先复现失败再验证正确的图像索引、矩形及每帧 0.5 秒时间。
没有放宽解析器来接受旧错误布局。样本输入详见[fixtures 说明](../../tests/fixtures/README.md)。

## 验证与剩余工作

- `cargo clippy`、`cargo fmt --all -- --check`、`git diff --check` 通过。
- 格式库 85 项与审计命令 8 项测试通过；最终全量 Rust 测试 **218 通过、0 失败**。
  普通套件仍按既有设计忽略 1 个显式离屏 GPU 用例，本轮没有修改 GPU 提交代码。
- CLI 的 `--max-records 2` 检查输出两条记录、`complete: false`、限额诊断和退出码 2。
- 合成包独立审计退出 0；本地完整选择范围的两次扫描均退出 2、输出逐字节相同。
- 额外的 `cargo clippy --workspace --all-targets` 退出 0；既有 daemon 测试模块位置
  产生 `items_after_test_module` 警告，未作为本次资源审计的额外改动。

全量测试运行需要本机 GPU/本地套接字访问，详见[解码基线](a0-decoder-baseline.md)。
审计本身无需 GPU。本次没有原程序截图、真实 niri/Plasma 播放验证或长时间 soak/fuzz。
下一步仍需按拟适配行为选取最小样本，固定时间/属性/随机种子/指针输入，并补原程序
参照或明确的运行环境阻塞；不能用本清单的二维载荷计数当作视觉兼容率。
