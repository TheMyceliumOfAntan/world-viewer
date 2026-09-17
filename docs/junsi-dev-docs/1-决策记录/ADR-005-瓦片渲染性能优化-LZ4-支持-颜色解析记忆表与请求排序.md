# ADR-005：瓦片渲染性能优化：LZ4 支持、颜色解析记忆表与请求排序

| 属性 | 内容 |
|---|---|
| 状态 | 已采纳 |
| 日期 | 2026-09-17 |
| 决策者 | AI Agent |

## 背景

对比 conic-apps/conic-worldmap 后确认本项目有三处可借鉴/缺失：① 1.20.5+ 服务端 region 文件可能用 LZ4 (compression type 4) 压缩，本项目只支持 1/2/3；② 单瓦片渲染中 Palette::color_ref 占约 60% 时间，每次调用分配 3 个 String（name_of 克隆、to_owned_ref 克隆、tint_kind 的 to_ascii_lowercase），而 GTNH z=0 瓦片实测 82944 列仅 102 种不同 id+meta；③ Leaflet 按行优先请求瓦片，新视口先取顶部行而非用户注视的视口中心。另外前端 CachedTileLayer 的 LRU 淘汰只从 Map 删除而不调用 ImageBitmap.close()，持续平移会泄漏 GPU 内存。

## 决策

分四项实施，每项前后打 git checkpoint tag 便于回滚，每项均以实测决定保留或回退。① region.rs 增加 comp=4 解压：按 Amulet-Core PR#283（Amulet issue #1027 的修复）实现 lz4-java LZ4Block 流解析——8 字节 ASCII "LZ4Block" magic + token + 三个小端 i32（compressed len, original len, xxhash）。不参照 conic-worldmap：它校验 0x184D2204（LZ4 Frame magic）并按大端读取，与 MC 实际写入的 LZ4Block 格式不符，其实现有误。因本地存档无 LZ4 样本，用构造的合法流做单元测试。② tileLayer.ts 淘汰/清空时调用 ImageBitmap.close()，关闭延后一个宏任务避免与已排队的 drawImage 竞争，并在关闭前复核位图是否已被重新命中。③ 新增渲染内局部 ColorMemo，按 BlockRefRef 记忆 (rgb, tint kind)，biome 在命中后套用。不放到共享 TileCache：共享需加锁，每列抢锁会让前端 6 路并发瓦片请求串行化。④ 队列项携带瓦片坐标，pump 时取距视口中心最近的一项；只对当前 _tileZoom 的项排序，避免缩放过渡时跨坐标系比较。

## 备选方案

### 方案 共享到 TileCache 的颜色缓存（加锁）
- 优点：跨瓦片复用，命中率更高
- 缺点：每列抢一次 Mutex，6 路并发瓦片请求会串行化，实测并发加速比会从 1.68x 退化
- 为何不选：并发收益远大于跨瓦片复用的收益，且渲染内局部记忆表已能覆盖单瓦片内 812:1 的去重比

### 方案 照抄 conic 的 LZ4 实现
- 优点：省去查证成本
- 缺点：其 magic 与字节序均与 MC 实际格式不符，会解压失败
- 为何不选：以 Amulet 的权威修复为准

### 方案 把 memo 提升为全局静态缓存
- 优点：无锁、跨瓦片复用
- 缺点：需处理调色板切换失效，且调色板随世界变化，静态缓存易出脏数据
- 为何不选：per-render 局部已足够，去重比 812:1

### 方案 对实现 4 以总耗时作验收指标
- 优点：指标直观
- 缺点：重排序不减少总工作量，只改变结果到达顺序，总耗时差异被噪声淹没（实测仅 -8%，方差大）
- 为何不选：改用「中心瓦片可见延迟」作为验收指标，实测 202ms→120ms

## 影响
- src-tauri/src/region.rs：新增 decompress_lz4 与 lz4_tests 单元测试
- src-tauri/Cargo.toml：新增 lz4_flex = "0.14" 依赖
- src-tauri/src/render.rs：新增 ColorMemo，Surface::fill 改用它；BlockRefRef 派生 Hash
- src/tileLayer.ts：closeSoon/remember/clearCache 关闭位图；队列项带坐标并按中心距离排序
- src-tauri/tests/memo_regression.rs、perf_palette.rs：新增一致性回归与性能探针
- scripts/measure-tiles-dev.mjs、measure-center-latency.mjs：新增 dev 模式 CDP 基准脚本

## 修订记录
| 日期 | 版本 | 修改内容 | 修改人 |
|---|---|---|---|
| 2026-09-17 | v1.0 | 初版创建 | AI Agent |