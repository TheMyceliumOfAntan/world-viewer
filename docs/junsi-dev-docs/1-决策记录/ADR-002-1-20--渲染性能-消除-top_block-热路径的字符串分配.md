# ADR-002：1.20+ 渲染性能：消除 top_block 热路径的字符串分配

| 属性 | 内容 |
|---|---|
| 状态 | 已采纳 |
| 日期 | 2026-09-14 |
| 决策者 | AI Agent |

## 背景

1.20+ 存档（如 Aegis of the Frozen Sky）渲染显著慢于 1.7.10：同一瓦片 1099ms vs 69ms（15.9x），玩家附近 9 个瓦片 10347ms vs 1000ms（10.3x）。实测定位到根因不在解压或解析——单区块冷加载 1.20+ 反而更快（0.63ms vs 1.25ms），且解压耗时更低（0.104ms vs 1.146ms）。瓶颈在 `ChunkData::top_block`：1.13+ 分支对每个被检查的方块调用 `Section::block_name()`，该方法从调色板取 `&str` 后 `.to_string()` 返回 `String`，而 1.20+ 区块有 24 个 section（y 从 -64 到 320，其中 15 个是全空气），1.7.10 只有 5 个。每次 16x16 列扫描的方块查找数从 256 增至 4096（16x），且每次查找都堆分配一个 String。实测每列耗时 0.25μs → 15.73μs（63x）。

## 决策

三项修改：① 新增 `BlockRefRef<'a>` 借用枚举（`Legacy(u16,u16)` / `Named(&'a str)`）与 `Section::block_ref()` 方法，在热路径上完全不分配；`ChunkData::top_block` 改用借用版本，仅在真正命中顶层方块时才构造拥有所有权的 `BlockRef`（每列一次而非每方块一次）。② `top_block` 从最高 section 向下扫描时，跳过不含任何非空气方块的 section（1.20+ 实测 24 个 section 中 15 个全空气）。③ 为 `ChunkData` 预计算并缓存每个 section 的"最高非空气 y 坐标"，避免重复扫描已知为空的区域。

## 备选方案

### 方案 为 BlockRef 增加零分配的借用变体
- 优点：避免分配，保持 BlockRef 抽象；调用方可直接拿到 &str 做比较
- 缺点：需要维护两套代码路径，但本就已有 Legacy/Named 分支
- 为何不选：采纳：改动局部且直接消除热路径分配

### 方案 在 render 中改用索引比较
- 优点：完全避免字符串
- 缺点：每次比较都调用 palette 查询，逻辑更绕
- 为何不选：不采纳：调色板是按名字组织的，索引不稳定

### 方案 跳过全空气 section
- 优点：直接减少 62% 的无效扫描
- 缺点：Section 数量在 1.13+ 由数据决定，无法压缩
- 为何不选：采纳：与上一项组合使用

## 影响
- src-tauri/src/render.rs：新增 BlockRefRef、Section::block_ref、top_block 重写、section 空气跳过
- src-tauri/src/region.rs：Section 增加 block_ref 借用查询
- src-tauri/tests/diag_topblock.rs：性能回归基线
- 文档：7-调用规范/存档格式支持.md 补充 26.1+ 维度路径

## 修订记录
| 日期 | 版本 | 修改内容 | 修改人 |
|---|---|---|---|
| 2026-09-14 | v1.0 | 初版创建 | AI Agent |