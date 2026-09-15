# API规范

> 生成时间：2026-09-14 03:57

# Tauri 命令与瓦片协议

## 1. 命令（invoke）

前端通过 `@tauri-apps/api/core` 的 `invoke()` 调用。全部定义在 `src-tauri/src/lib.rs`。

### 1.1 `open_world`

打开存档，成功时替换全局状态。

**入参**

| 字段 | 类型 | 说明 |
|------|------|------|
| `path` | `string` | 存档目录绝对路径（含 `level.dat` 的目录） |

**返回** `OpenResult`

```ts
{
  ok: boolean;
  info: WorldInfo | null;
  error: string | null;
}
```

失败时 `ok=false`，`error` 为中文错误信息（如 `"存档目录不存在: ..."`、`"找不到 level.dat: ..."`）。

**副作用**：替换 `AppState.world`，并以新的 Palette 重建 `TileCache`（容量 4096）。

### 1.2 `get_world_info`

返回当前已加载世界的 `WorldInfo`，未加载时返回 `null`。

无入参。

### 1.3 `invalidate_cache`

清空区块缓存，下次请求重新读盘。用于存档在外部被修改后刷新。

无入参，无返回。

### 1.4 `pick_save_folder`

占位命令，当前始终返回 `null`。目录选择实际由前端 `@tauri-apps/plugin-dialog` 的 `open({ directory: true })` 完成。

### 1.5 `probe_block`

查询一个世界列（x, z）在 `ymax` 过滤下的最高非空气方块，供底部状态栏显示 Y 坐标与方块信息。

**为什么需要后端查询**：地图是 `CRS.Simple` 二维平面，屏幕坐标只能推出 X/Z；Y 是垂直轴，不在平面内，必须由后端扫描列才能得到。

```ts
invoke<BlockInfo>("probe_block", { dim: 0, x: -415, z: -286, ymaxU: 4294967295 })
```

| 参数 | 类型 | 说明 |
|------|------|------|
| `dim` | i32 | 维度 id |
| `x` | i32 | 世界 X 坐标（方块） |
| `z` | i32 | 世界 Z 坐标（方块） |
| `ymaxU` | u32 | 同瓦片协议：`4294967295` = 全高，否则按 `min(255)` 截断 |

返回 `BlockInfo`：

```ts
{ y: number | null, name: string | null, id: string | null }
```

- `y`：该列最高非空气方块的 Y；列全为空气或区块未生成时为 `null`。
- `name`：可读名，优先 JourneyMap 显示名（GTNH 等 legacy 存档为中文，如「宏伟之木树叶」）；现代存档回落到原始 id。
- `id`：原始标识，legacy 为 `id:meta`（如 `31:1`），1.13+ 为 `minecraft:xxx`。

**与瓦片渲染同源**：复用 `render::TileCache` 的区块缓存（可见瓦片几乎总会把该区块读进缓存，因此通常是 HashMap 命中）；`ymax` 语义、`top_block_ref` 列扫描逻辑与渲染器完全一致，因此状态栏显示的方块就是该点渲染出的表面方块。

**调用频率**：前端在 `mousemove` 上以 150ms 节流调用，并用递增序号丢弃过期响应，避免慢回包覆盖新结果。

## 2. 数据结构

### 2.1 `WorldInfo`

```ts
{
  save_dir: string;             // 存档绝对路径
  save_name: string;            // 存档名（来自 level.dat LevelName）
  level_name: string;           // 同上（历史字段，保留兼容）
  world_seed: string;           // 世界种子，空串表示无法读取
  instance_root: string;        // 实例根目录（向上探测到的含 journeymap/ 或 mods/ 的目录）
  dimensions: DimensionInfo[];  // 有 region 数据的维度，按 id 升序
  player: PlayerInfo | null;    // 玩家最后位置
  waypoints: Waypoint[];        // 全部路径点（含所有维度与来源）
  palette_entries: number;      // JourneyMap 调色板条目数
  palette_mapped_blocks: number;// 有名称映射的方块数
}
```

#### `world_seed` 为什么是字符串

`world::read_world_seed()` 读取 `level.dat` 的 `Data.WorldGenSettings.seed`（1.16+），回退到旧版 `Data.RandomSeed`，两者都缺失时返回空串。

**必须以字符串跨 IPC 传递**：Minecraft 种子是 `i64`，范围可达 ±2^63，超过 JS `Number.MAX_SAFE_INTEGER`（2^53-1）。若以数字序列化，前端会静默丢失低位精度，显示的种子与真实值不符。后端在 Rust 侧 `to_string()`，前端按字符串原样展示。

### 2.2 `DimensionInfo`

```ts
{
  id: number;          // 维度 id（0 主世界 / -1 下界 / 1 末地 / 其余为模组维度）
  name: string;        // 友好名，未知则为 "DIMxx"
  region_dir: string;  // region 目录绝对路径
  chunk_count: number; // 该维度已生成的区块总数
  has_data: boolean;   // 恒为 true（无数据的维度不会出现在列表里）
  min_y: number;       // 该维度最低方块 Y（含）
  max_y: number;       // 该维度最高方块 Y（含）
}
```

#### 高度范围如何确定

世界高度**不在 level.dat 里**（`min_y`/`height` 属于数据包定义，不写入存档）。唯一可靠来源是区块本身：

| 存档格式 | 区块 section 列表 | 探测结果 |
|---|---|---|
| 1.13+（扁平化） | **完整**，含空 section | 取极值即真实范围。1.20 主世界为 section -4..19 → **block Y -64..319** |
| 1.12-（legacy） | 仅非空 section（GTNH 区块只有 Y=0..4） | 无法推导，回退 **0..255** |

判据用 `block_states` 是否存在来区分两种格式（该字段仅 1.13+ 有），而不是靠 section 数量——legacy 区块的 section 数同样可能大于 1。

探测成本：只读每个 region 目录里前 2 个 `.mca` 文件的第一个非空区块，与既有的 `count_regions` 扫描同量级。

**为什么必须动态**：1.20+ 世界最高到 Y=319，旧实现把 `ymax` 硬编码截断在 255，会静默隐藏 Y>255 的全部方块；同时 1.18+ 世界最低为 Y=-64，固定从 0 起会丢失整个深板岩层。

### 2.3 `PlayerInfo`

```ts
{ x: number; y: number; z: number; dimension: number; name: string }
```

来自 `level.dat` 的 `Data.Player.Pos` / `Dimension` / `name`。

### 2.4 `Waypoint`

```ts
{
  name: string;
  x: number; y: number; z: number;
  dimension: number;
  color: string;   // "#rrggbb"
  kind: string;    // "Normal" | "Death" | "OldDeath"
  source: string;  // "journeymap" | "xaero" | "voxelmap"
}
```

## 3. 瓦片协议

自定义 URI scheme `tile`，在 `register_asynchronous_uri_scheme_protocol` 注册。

### 3.1 URL 格式

```
http://tile.localhost/{worldKey}/{dim}/{z}/{x}/{y}.png?ymax=N
```

（Tauri 2 在 Windows 上将 `tile://` 映射为 `http://tile.localhost/`）

| 段 | 类型 | 说明 |
|----|------|------|
| `worldKey` | string | 存档路径哈希（`w` + base36）。**后端忽略**，仅用于隔离缓存 |
| `dim` | i32 | 维度 id，可为负 |
| `z` | i32 | 缩放级别，0..=4 |
| `x` | i32 | 瓦片列号，可为负 |
| `y` | i32 | 瓦片行号，可为负 |
| `ymax` | i64 | 高度切层上限，**可为负**（1.18+ 世界最低 Y=-64）；`4294967295` 表示全高 |

**路径段数必须为 5**，否则返回 404 与错误文本。

**`z` 必须是整数字面量**：后端用 `parse::<i32>()` 解析，`/0/0.5/-3/-2.png` 这类小数 zoom 会解析失败并返回 404。前端若把小数缩放（`zoomSnap: 0.25` 下的 `mapZoom=2.5`）直接当瓦片 zoom 传下来，所有瓦片都会 404、地图全黑。前端在 `CachedTileLayer._clampZoom` 唯一入口对 zoom 取整（详见 `6-UI组件设计/前端组件.md` §3.6）。

**`ymax` 是有符号的**：必须能表达负高度（1.18+ 世界最低 Y=-64），若按 `u32` 解析则 `ymax=-64` 会解析失败并静默回退到全高，负 Y 过滤形同失效。全高哨兵 `4294967295` 会被解析为该维度自身的 `max_y`（不是固定 255），超出范围的值 clamp 到 `[min_y, max_y]`。

### 3.2 缩放与覆盖范围

| zoom | 每瓦片区块数 | 每方块像素 |
|------|-------------|-----------|
| 0 | 16 × 16 | 1 |
| 1 | 8 × 8 | 2 |
| 2 | 4 × 4 | 4 |
| 3 | 2 × 2 | 8 |
| 4 | 1 × 1 | 16 |

### 3.3 响应

**成功**

| 头 | 值 |
|----|-----|
| `Content-Type` | `image/png` |
| `Access-Control-Allow-Origin` | `*` |
| `Cache-Control` | `no-store` |
| `X-Tile-Empty` | `1` = 该瓦片覆盖范围内无任何已生成区块；`0` = 有数据 |
| `Access-Control-Expose-Headers` | `X-Tile-Empty` |

响应体为 256×256 RGBA PNG。无数据区域为全透明。

**失败**：404，`Content-Type: text/plain; charset=utf-8`，响应体为错误原因（如 `"未加载世界"`、`"维度不存在"`、`"bad tile path: ..."`）。

### 3.4 `X-Tile-Empty` 的用途

前端据此区分两种"看起来空白"的情况：

| 情况 | 表现 |
|------|------|
| 无数据（`X-Tile-Empty: 1`） | 绘制浅色棋盘格，表示"该区域从未生成" |
| 仍在加载 | 显示父级瓦片的模糊放大图（若有），否则透明 |

没有这个头时，用户无法区分"没数据"与"加载失败"。

### 3.5 并发与线程

协议处理器在**独立线程**中执行渲染：

```rust
.register_asynchronous_uri_scheme_protocol("tile", |ctx, request, responder| {
    let app = ctx.app_handle().clone();
    let uri = request.uri().clone();
    std::thread::spawn(move || { /* 渲染并 respond */ });
})
```

**不得改回同步渲染**：同步执行会阻塞 Tauri 事件循环，导致窗口在拖拽/缩放时卡顿。

## 4. 前端瓦片层契约

`src/tileLayer.ts` 的 `CachedTileLayer` 继承 `L.GridLayer`。

| 方法 | 说明 |
|------|------|
| `setUrlTemplate(template)` | 设置 URL 模板；变化时清空失败记录、空标记、待执行队列与等待回调，然后 `redraw()` |
| `getUrlTemplate()` | 返回当前模板（用于判断是否需要清缓存） |
| `clearCache()` | 清空图片缓存、等待队列、失败记录、空标记 |
| `stats()` | 返回 `{cached, waiting, queued, active, failed, empty}`，供诊断 |

**关键不变量**：

1. 同一瓦片的并发请求必须**共享回调**而非丢弃。Leaflet 在缩放/平移时会重建瓦片元素并重复请求同一坐标；若第二次请求的回调被丢弃，该瓦片将永久空白。
2. 切换存档时必须清空缓存。模板含 `worldKey`，因此比较模板即可检出世界变化；`loadWorld` 另有显式 `clearCache()` 兜底。
3. **模板变化时必须同时清空 `queue` 与 `waiting`**。`queue` 中存的是旧模板 URL 的闭包，失效后继续执行会浪费带宽并阻塞新请求；`waiting` 是这些任务的回调登记表，只清队列会让回调成为孤儿——若用户拖回同一 ymax，该 key 命中 `waiting` 分支被追加而非重新请求，瓦片永久空白。二者必须成对清理。高度滑块每次 `onChange` 都触发模板变化，是这条不变量最常被踩到的入口。

## 5. 相关文档

- `2-架构设计/整体架构.md`
- `2-架构设计/渲染管线.md`
- `7-调用规范/存档格式支持.md`
- `6-UI组件设计/前端组件.md`


## 修订记录
| 日期 | 版本 | 修改内容 | 修改人 |
| 2026-09-14 | v1.0 | 初版创建 | AI Agent |
| 2026-09-15 | v1.1 | WorldInfo 新增 world_seed（字符串，防 i64 精度丢失）；前端瓦片层契约补充模板切换须成对清空 queue/waiting | AI Agent |
| 2026-09-15 | v1.2 | 瓦片协议补充 z 必须为整数字面量；新增 probe_block 命令与 BlockInfo 结构（状态栏 Y 坐标与方块信息） | AI Agent |
| 2026-09-15 | v1.3 | DimensionInfo 新增 min_y/max_y（按区块 section 动态探测高度范围）；ymax 改为有符号以支持 1.18+ 负 Y | AI Agent |

