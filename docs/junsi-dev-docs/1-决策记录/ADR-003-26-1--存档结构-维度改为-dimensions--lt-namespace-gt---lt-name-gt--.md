# ADR-003：26.1+ 存档结构：维度改为 dimensions/&lt;namespace&gt;/&lt;name&gt;/

| 属性 | 内容 |
|---|---|
| 状态 | 已采纳 |
| 日期 | 2026-09-14 |
| 决策者 | AI Agent |

## 背景

MC 26.1 snapshot-6 起，存档结构发生不兼容变更：所有维度改为按命名空间存储于 `dimensions/<namespace>/<name>/`，主世界/下界/末地不再使用根目录、`DIM-1/`、`DIM1/`。同时 `playerdata/` 移到 `players/data/`，`advancements/` 移到 `players/advancements/`，`stats/` 移到 `players/stats/`。实测本机 26.2 存档结构：`saves/新的世界/` 下只有 `data` `datapacks` `dimensions` `players` `icon.png` `level.dat`，无 `region/` 也无 `DIM*/`；区块在 `dimensions/minecraft/overworld/region/r.*.mca`。当前 `world::open_world` 只扫描 `region/` 与 `DIM*/region/`，因此 26.1+ 存档打开后维度列表为空，完全无法预览。

## 决策

在维度扫描中新增第三种布局分支：枚举 `<save>/dimensions/<namespace>/<name>/region`，并把 `<name>` 映射为内部数字维度 id（overworld→0、the_nether→-1、the_end→1，其余命名空间维度使用稳定的哈希派生 id 或按发现顺序编号）。同时保留现有 `region/` 与 `DIM*/region/` 分支，三者按存在性择一或合并。维度友好名优先使用命名空间 id 本身（如 `minecraft:overworld`），以便 UI 显示。

## 备选方案

### 方案 增加 dimensions/<ns>/<name>/region 扫描分支
- 优点：直接对应新格式；可把 <name> 映射回数字维度 id
- 缺点：26.1 之前的新版存档用 region/，需保留旧逻辑
- 为何不选：采纳：与现有 region/、DIM*/ 分支并列

### 方案 重写维度发现为插件式扫描器
- 优点：扩展性好
- 缺点：改动大
- 为何不选：不采纳：当前仅三种布局，重写超出需求

## 影响
- src-tauri/src/world.rs：维度扫描新增 dimensions/<ns>/<name>/region 分支
- src-tauri/src/palette.rs：dimension_name 支持命名空间形式
- src-tauri/tests/modern_format.rs 或新增测试：26.2 存档维度发现
- 文档：7-调用规范/存档格式支持.md 补充 26.1+ 布局

## 修订记录
| 日期 | 版本 | 修改内容 | 修改人 |
|---|---|---|---|
| 2026-09-14 | v1.0 | 初版创建 | AI Agent |