# World Viewer

Minecraft 存档可视化工具。直接读取存档的 Anvil region 文件，按需渲染成地图瓦片，在桌面窗口中平移、缩放浏览。**只读**——绝不写入存档目录。

技术栈：Tauri 2（Rust 后端 + React 19 前端 + WebView2 渲染 + Leaflet 地图）。

## 特点

### 全版本存档支持

不依赖 `DataVersion`，而是**按 NBT 结构特征探测**区块格式。这让被模组改过版本号的存档（如 GTNH）也能正确解析。

| MC 版本 | 区块格式 | 方块标识 |
|---------|---------|---------|
| 1.6.4 及更早 | `Blocks` + `Data`（8 位 id） | 数值 id + meta |
| 1.7.10 | `Blocks16` + `Data16`（NotEnoughIDs 16 位扩展） | 数值 id + meta |
| 1.12.2 | `Blocks` + `Add`（半字节扩展）+ `Data` | 数值 id + meta |
| 1.13 – 1.17 | `Level.Sections[]` 平级 `Palette` + `BlockStates` | 命名空间名 |
| 1.18+ | 根级 `sections[].block_states`（调色板 + 位压缩） | 命名空间名 |

位压缩的两种 long 打包布局（1.13–1.15 连续跨 long、1.16+ 每项对齐 long 边界）由 long 数组长度自动判定，不需要版本号。

压缩支持 gzip、zlib、无压缩与 **LZ4**（1.20.5+ 服务端 `region-file-compression=lz4`）。

已实测的存档版本（`tests/all_saves_smoke.rs`）：1.6.4、1.8.9、1.12.2（原版与 Forge）、1.16.5、1.20+、26.2，以及 GTNH 2.8.4（1.7.10 + NotEnoughIDs + 4038 个模组方块）。

### 模组方块支持

- **方块名映射**，按优先级回退：`level.dat` 的 `FML.ItemData`（1.7.10 全量注册表，GTNH 实测 4038 个）→ `FML.Registries.minecraft:blocks.ids`（1.12.2 模组注册）→ 内置原版表。
- **方块配色**：JourneyMap `colorpalette.json` 优先，缺失时用内置原版表 + 名称后缀规则推断（`_leaves` → 叶色、`_ore` → 石色等）。
- **1.13 扁平化别名**：旧存档的 `minecraft:grass`、`leaves`、`log` 等映射到扁平化后的名字，覆盖 1.0–1.12.2 全区间。

### 生物群系染色

草与树叶在调色板里存的是**未染色灰度**，需按生物群系施加 tint 才是正确的绿色。区分两类方块：

- 灰色（未染色的草/树叶）→ 乘算 `(base * tint) >> 8`
- 已是最终色（水、芦苇）→ 按平原基准做比例缩放，避免二次压暗成近黑

生物群系数据源：1.18+ 逐 section 的 `biomes` 调色板；1.15–1.17 为 chunk 级 4×4×4 网格；1.7–1.14 为 chunk 级逐列。

### 其他

- **多维度**：自动扫描 `region/`、`DIM*/region/` 与 26.1+ 的 `dimensions/<namespace>/<name>/region/`，只列出有数据的维度。维度命名支持 Galacticraft / GalaxySpace / 暮色森林 / Witchery 等。
- **高度切层**：滑块选择 Y 上限查看地下结构，高度范围按维度实际上下限动态适配（1.18+ 世界 Y 从 -64 起）。
- **路径点**：合并 JourneyMap、Xaero、VoxelMap 三种来源，标注来源。
- **浮雕着色**：坡向光照（光源西北）呈现地形起伏，纯平地保持中性不变暗。
- **水面透视**：列扫描遇水继续下钻取水底，按深度混合水色与水底色。

## 架构

```
前端 (React 19 + Leaflet)
  App.tsx          维度列表 / 高度切层 / 标记 / 状态栏
  tileLayer.ts     CachedTileLayer：缓存 + 限流 + 占位
        │
        ├─ invoke(命令) ─────► Tauri 层 (src-tauri/src/lib.rs)
        │                        AppState { world, cache }  全局单例，Mutex 保护
        │                        5 个 #[tauri::command]     加载 / 信息 / 失效 / 探针 / 选择
        │
        └─ tile:// 协议 ─────► 异步 URI 协议处理器（独立线程）
                                 │
                                 ▼
                              领域层 (Rust)
                                world.rs       存档打开、维度扫描、实例根探测
                                region.rs      Anvil 读取 + 四种区块格式解析
                                render.rs      Surface 缓冲、水底双缓冲、浮雕、缓存
                                biome.rs       生物群系 tint 解析与两条染色公式
                                palette.rs     方块配色、路径点、玩家位置
                                nbt.rs         最小 NBT 解析器（含定向跳过）
                                legacy_ids.rs  内置原版 ID→名称表
```

瓦片 URL：`http://tile.localhost/{worldKey}/{dim}/{z}/{x}/{y}.png?ymax=N&water=1&shade=1&alt=1`

`worldKey` 由存档路径哈希得到，后端忽略该段，仅用于隔离浏览器缓存与前端缓存。渲染开关（水/阴影/高度）也是 URL 的一部分，切换即触发重新请求。

## 缓存方案

采用**四层缓存**，从前端到磁盘逐层回退。

### 1. 前端位图缓存（`CachedTileLayer`，`src/tileLayer.ts`）

- 缓存已解码的 `ImageBitmap`，容量 **512 张**，真 LRU（`Map` 插入序即最近使用序，命中时重新插入）。
- 命中时同步返回，不发起任何请求。
- **在途请求去重**：同一瓦片 key 的重复请求不会重复发出，而是把回调追加到等待列表，全部在图片到达时一起触发。Leaflet 在缩放/平移时会重建瓦片元素并重复请求同一瓦片，若丢弃回调会导致瓦片永久空白。
- **父瓦片降级**：请求新瓦片时若其父瓦片（上一 zoom 级的 1/4 区域）已在缓存，立即放大裁剪绘制到当前瓦片，等清晰瓦片到达后替换。这样缩放和平移时不会出现空洞。
- **位图生命周期**：淘汰与切换世界时调用 `ImageBitmap.close()` 释放 GPU 内存。关闭延后一个宏任务（避免与 `createTile` 中已排队的 `drawImage` 竞争抛 `InvalidStateError`），并在关闭前复核位图是否已被重新命中。

### 2. 后端区块缓存（`TileCache`，`src-tauri/src/render.rs`）

- 缓存解析后的 `ChunkData`，容量 **8192 区块**（约 78 MB）。
- **真 LRU**：每个 key 记录使用时间戳，淘汰时按时间戳排序一次，批量降到容量的 7/8 水位。早期版本在容量满时移除 HashMap 迭代序的前 25%，那是近似随机的——热点区块与已划走的区块被淘汰概率相同，来回平移会反复重读磁盘。
- **容量依据**：zoom 0 一屏 5×5 瓦片、每瓦片需 18×18 区块，工作集约 6500 区块。容量 4096 时只能容纳一屏的 63%，实测命中率 17%；提到 8192 后一屏装得下，命中率升到约 53%，整屏渲染耗时约减半。
- 区块以 `Arc<ChunkData>` 形式交出，锁只保护 map。批量取区块在**一次加锁**内完成，随后在**不持锁**的情况下做扫描——否则前端并发的 6 个瓦片请求会串行执行（实测加速比 0.94x，即完全无并行）。
- 缓存"该区块不存在"（`None`），避免对空白区域反复读盘。

### 3. 区域文件句柄缓存（`region.rs`）

- 缓存打开的 `.mca` 文件句柄及其 8 KiB 位置表（`OnceLock<Mutex<HashMap<PathBuf, Arc<RegionFile>>>>`），进程内常驻。
- 没有这层时，每个区块都要重新打开文件并重读位置表。单瓦片 324 个区块就是 324 次多余的 open + 8 KiB 读取。
- 打开动作在锁外执行，避免并行加载时 8 个工作线程在首次访问同一文件时排队。
- 用 Windows 的**定位读**（`seek_read`）而非 `seek` + `read`：定位读只需 `&File`，多个工作线程可同时读同一句柄；`seek` + `read` 需要 `&mut File`，会强制所有并发读取排队。

### 4. 浏览器 HTTP 缓存隔离

瓦片 URL 携带 `worldKey`，让浏览器自身的缓存也按存档隔离。没有这个字段时，切换存档会继续复用上一个世界的瓦片。

## 性能优化方案

### 渲染路径

| 优化 | 效果 |
|------|------|
| **定向 NBT 解析** | 4.4x |
| **颜色解析记忆表** | 2.8x（单瓦片） |
| **瓦片级 Surface 缓冲** | 消除区块接缝 |
| **并行区块加载** | 8 线程 |

**定向 NBT 解析**：区块 NBT 中 `Entities` / `TileEntities` 占大部分体积（GTNH 单区块可达 747 个 tile entity），但渲染完全用不到。解析器在遇到这些子树时直接按长度跳过而不构建 `Tag`。实测单区块 1.023 ms → 0.231 ms。正确性由 `tests/fast_parser.rs` 保证：对真实区块逐字节比对两种解析器的输出。

**颜色解析记忆表（`ColorMemo`）**：单瓦片渲染中 `Palette::color_ref` 曾是最大热点，占约 60% 渲染时间。根因是每次调用分配 3 个 `String`：块名克隆、`to_owned_ref` 克隆、`tint_kind` 内的小写转换。而实际去重空间极小——GTNH zoom 0 瓦片实测 **82944 个地表列只有 102 种不同 `(id, meta)`**，去重比约 812:1。

`ColorMemo` 按方块记忆 `(rgb, tint kind)`，生物群系在命中后再套用。它是**渲染内局部**的，不放到共享缓存——共享需要加锁，而每列抢锁会让前端 6 路并发瓦片请求串行化。

一致性由 `tests/memo_regression.rs` 保证：96912 组 `(block, meta, biome)` 组合下与直接解析路径结果完全一致；跨缓存历史的渲染字节稳定。

**瓦片级 Surface 缓冲**：渲染以整个瓦片为单位构建 `Surface`（带 1 格边距 + 1 区块边距），再统一做浮雕着色。跨区块的坡度比较因此正确，且区块之间没有接缝。若按区块单独着色，区块边缘的坡向会算错。

**并行区块加载**：`std::thread::scope` 手写分片，线程数为 `available_parallelism()` 上限 8。未引入 rayon。结果按原顺序返回，保证渲染结果不依赖线程调度。

### 传输与请求

| 优化 | 效果 |
|------|------|
| **二进制 PNG 响应** | 无 base64 膨胀 |
| **瓦片请求按视口中心排序** | 中心瓦片可见延迟 -41% |
| **异步协议响应** | 主线程 0 个 long task |
| **前端并发限流** | 上限 6，其余排队 |
| **空瓦片标记** | 与"加载中"区分 |
| **父瓦片降级** | 无空洞 |

**瓦片请求按视口中心排序**：Leaflet 按行优先顺序请求瓦片，新视口会把顶部几行先取完，而用户注视的视口中心反而靠后。队列项携带瓦片坐标，取出时优先取距视口中心最近的一项。实测中心瓦片可见延迟中位数 **202 ms → 120 ms**。

> 注意验收指标是"中心瓦片可见延迟"而非总耗时：重排序不减少总工作量，只改变结果到达顺序，总耗时差异会被噪声淹没。

**异步协议响应**：`tile://` 协议处理器内必须先 `std::thread::spawn` 再渲染。同步渲染会阻塞 Tauri 事件循环，拖拽/缩放时窗口明显卡顿（实测 700 ms 级 long task）。

**空瓦片标记**：响应头 `X-Tile-Empty` 区分"此处从未生成区块"与"仍在加载"。前端对前者绘制淡棋盘格，对后者保持透明。仅按 PNG 内容判断不够——瓦片带 1 区块边距，边距内可能有数据而瓦片自身全空。

### 依赖克制

并行加载用标准库 `thread::scope` 手写，未引入 rayon；NBT 解析自研（1.7.10 + NotEnoughIDs 的变体格式既有库支持不佳）。Rust 侧依赖仅 7 个：`tauri`、`tauri-plugin-dialog`、`serde`、`serde_json`、`flate2`、`lz4_flex`、`png`。

## 性能基线

实测环境：GTNH 2.8.4 存档（主世界 8 个 region，18 MB），14 核。

| 操作 | 数值 |
|------|------|
| 单区块加载 + 解析 | 0.56 ms |
| 单瓦片渲染（zoom 0，324 区块，warm） | 5.2 ms |
| 一屏 16 瓦片（zoom 0） | 7.4 ms/瓦片 |
| 并发加速比（6 瓦片） | 1.68x |
| 主线程 long task | 0 |

## 构建与运行

环境要求：Node.js 24+、**pnpm**（不要用 npm，见下）、Rust 1.97+（MSVC 工具链）、WebView2。

```bash
# 依赖安装（必须用 pnpm）
pnpm install

# 开发（Vite + Tauri，热重载）
pnpm tauri dev

# 仅前端构建（类型检查 + 打包）
pnpm build

# Rust 检查与测试
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --release -- --nocapture

# 打包（生成 .msi / .exe）
pnpm tauri build
```

**为什么用 pnpm**：本机实测 `npm install` 安装 devDependencies 时连续两次超时（600 s 无输出），但 `npm view` 2 秒即返回、缓存与认证均正常。同网络下 `pnpm install` 23 秒完成。现象特征是 `package-lock.json` 写入记录但 `node_modules` 未落盘。故 `package-lock.json` 已删除，改用 `pnpm-lock.yaml`。

### 调试

```powershell
# 用 CDP 连真实 WebView2（9222 常被其他 WebView2 应用占用，故用 9223）
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9223'
pnpm tauri dev
```

```
# 深链直接打开存档，刷新页面即可恢复世界
http://localhost:1420/?save=<URL 编码的存档路径>
```

基准脚本（需应用已带 CDP 端口运行）：

```bash
node scripts/measure-tiles-dev.mjs 9223 <label>
node scripts/measure-center-latency.mjs 9223 <label>
```

## 测试

21 个集成测试文件，分三类：

- **正确性**：`fast_parser`（解析器逐字节比对）、`colors` / `legacy_colors` / `tint_regression`（配色与染色回归）、`palette_packing`（位压缩布局）、`modern_format` / `height_range`（版本格式）、`water_and_biomes`、`waypoints*`
- **性能**：`perf_profile` / `perf_stages` / `perf_region_io` / `perf_concurrency` / `perf_palette` / `perf_chunk_memory` / `perf_sweep`
- **一致性**：`memo_regression`（颜色记忆表与直接路径等价、渲染可重复）

测试存档不存在时自动 SKIP，不阻塞 CI。

## 项目约束

违反会导致功能静默失效：

| 约束 | 原因 |
|------|------|
| 协议处理器必须 `std::thread::spawn` | 同步渲染阻塞事件循环，窗口卡顿 |
| `color_ref()` 只返回方块 id 名 | 返回显示名会让染色分类恒为 false |
| 瓦片 URL 必须含 `worldKey` | 否则切换存档复用旧瓦片 |
| 存档目录只读 | 红线，不得写入 |
| `is_foliage` 不得包含花朵/作物 | 会把红花染成绿色 |

## 文档

详细设计文档在 [`docs/junsi-dev-docs/`](docs/junsi-dev-docs/)：

| 主题 | 文档 |
|------|------|
| 分层与数据流 | `2-架构设计/整体架构.md` |
| 渲染细节（Surface、浮雕、缓存、并行） | `2-架构设计/渲染管线.md` |
| 接口契约 | `3-API规范/Tauri命令与瓦片协议.md` |
| 存档格式、配色、路径点 | `7-调用规范/存档格式支持.md` |
| 前端组件 | `6-UI组件设计/前端组件.md` |
| 测试与验证 | `4-编码规范/测试与验证.md` |
| 环境与构建 | `8-部署运维/开发环境与构建.md` |
| 需求边界 | `9-系统要求/功能与非功能需求.md` |
| 架构决策记录 | `1-决策记录/ADR-00*.md` |

## 许可

GNU General Public License v3.0（仅此版本，SPDX: `GPL-3.0-only`）。全文见 [`LICENSE`](LICENSE)。

```
Copyright (C) 2026 JunSi_233 <tmoaminecraft@gmail.com>
```

这意味着：你可以自由使用、修改、分发本软件，但分发（含修改版）时必须提供完整源码并以同样的 GPLv3 授权，且不提供任何担保。本程序按"现状"提供，无任何明示或暗示的保证。

第三方依赖均使用与 GPLv3 兼容的宽松许可（MIT / Apache-2.0 / BSD-2-Clause），详见各自仓库。

## 范围外

- 写入存档（红线：只读）
- 从 jar 提取纹理（颜色用调色板 + 内置表）
- 磁盘瓦片缓存（每次启动重新渲染）
- 实体 / 生物 / 光照 / 洞穴 / 结构渲染（仅渲染方块地表）
- 生物群系精确着色（需读 colormap 温度/湿度，当前用 tint 表）
