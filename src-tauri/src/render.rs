use std::collections::HashMap;
use std::path::Path;

use crate::biome::{self, BiomeDef, TintKind};
use crate::palette::Palette;
use crate::region;

pub const TILE_SIZE: usize = 256;
const EMPTY: i32 = i32::MIN;

/// A block reference, either a legacy numeric id+meta or a 1.13+ namespaced
/// block name. Both resolve through `Palette`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BlockRef {
    Legacy(u16, u16),
    Named(String),
}

impl BlockRef {
    pub fn is_air(&self) -> bool {
        match self {
            BlockRef::Legacy(id, _) => *id == 0,
            BlockRef::Named(name) => is_air_name(name),
        }
    }
}

/// Air block ids used by 1.13+ chunks.
#[inline]
pub fn is_air_name(name: &str) -> bool {
    name == "minecraft:air" || name == "minecraft:cave_air" || name == "minecraft:void_air"
}

/// Water block names. Used to decide whether a column's surface is a water
/// surface, in which case the column scan continues down to the sea floor.
#[inline]
pub fn is_water_name(name: &str) -> bool {
    let base = name.split('[').next().unwrap_or(name);
    let short = base.split(':').next_back().unwrap_or(base);
    short == "water" || short == "flowing_water"
}

/// Toggles for the renderer. All default to `true`.
#[derive(Debug, Clone, Copy)]
pub struct RenderOpts {
    /// See through water: scan down to the sea floor and blend the surface
    /// water colour toward it with depth. When off, water renders as a flat
    /// water-coloured block.
    pub water: bool,
    /// Slope shading (directional relief).
    pub shading: bool,
    /// Part of shading: lighten terrain by altitude.
    pub altitude: bool,
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts {
            water: true,
            shading: true,
            altitude: true,
        }
    }
}

impl RenderOpts {
    /// Parse the `water`/`shade`/`alt` tile query flags. Absent flags default
    /// to on, so existing URLs keep their behaviour.
    pub fn from_query(get: impl Fn(&str) -> Option<String>) -> Self {
        let flag = |k: &str| get(k).map(|v| v != "0").unwrap_or(true);
        RenderOpts {
            water: flag("water"),
            shading: flag("shade"),
            altitude: flag("alt"),
        }
    }
}

/// A borrowed block reference, used on hot paths to avoid allocating a
/// `String` for every block inspected while scanning a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockRefRef<'a> {
    Legacy(u16, u16),
    Named(&'a str),
}

impl<'a> BlockRefRef<'a> {
    #[inline]
    pub fn is_air(&self) -> bool {
        match self {
            BlockRefRef::Legacy(id, _) => *id == 0,
            BlockRefRef::Named(name) => is_air_name(name),
        }
    }

    #[inline]
    pub fn is_water(&self) -> bool {
        match self {
            // 8 = flowing water, 9 = still water (both eras of the numeric table).
            BlockRefRef::Legacy(id, _) => *id == 8 || *id == 9,
            BlockRefRef::Named(name) => is_water_name(name),
        }
    }

    /// Take ownership, allocating only for the named variant.
    pub fn to_owned_ref(self) -> BlockRef {
        match self {
            BlockRefRef::Legacy(id, meta) => BlockRef::Legacy(id, meta),
            BlockRefRef::Named(name) => BlockRef::Named(name.to_string()),
        }
    }
}

/// A fully parsed chunk. Supports both the legacy (1.12-) and flattened
/// (1.13+) section layouts.
pub struct ChunkData {
    pub sections: Vec<region::Section>,
    /// Chunk-level biomes for the 1.7–1.17 formats (1.18+ carries them per
    /// section instead).
    pub legacy_biomes: Option<region::LegacyBiomes>,
    /// Per section, the highest local y (0..16) that holds a non-air block,
    /// or `None` when the section is entirely air. Computed once at load time
    /// so column scans can skip empty sections immediately.
    ///
    /// This matters a lot for 1.13+ chunks: a 1.20 overworld chunk has 24
    /// sections spanning y=-64..320, of which ~15 are empty air.
    section_top: Vec<Option<u8>>,
}

impl ChunkData {
    pub fn new(sections: Vec<region::Section>) -> Self {
        Self::with_biomes(sections, None)
    }

    pub fn with_biomes(
        sections: Vec<region::Section>,
        legacy_biomes: Option<region::LegacyBiomes>,
    ) -> Self {
        let section_top = sections.iter().map(section_highest_non_air).collect();
        ChunkData {
            sections,
            legacy_biomes,
            section_top,
        }
    }

    /// Biome tints for the block at local `(x, z)` and absolute block-Y `y`.
    ///
    /// 1.18+ resolves a namespaced name from the section's biome palette;
    /// older formats use the chunk-level numeric biome id.
    fn biome_at(
        &self,
        section: &region::Section,
        x: usize,
        y: usize,
        z: usize,
        by: i32,
    ) -> BiomeDef {
        if section.biomes.is_some() {
            return section
                .biome_name(x, y, z)
                .map(BiomeDef::modern)
                .unwrap_or_default();
        }
        match &self.legacy_biomes {
            Some(b) => BiomeDef::legacy(b.biome_id(x, z, by)),
            None => BiomeDef::default(),
        }
    }

    /// Top-most non-air block at (x,z) with block-y <= ymax.
    ///
    /// Hot path: no allocation until a block is actually found. Callers that
    /// need an owned `BlockRef` pay for exactly one allocation per column
    /// instead of one per inspected block.
    pub fn top_block_ref(&self, x: usize, z: usize, ymax: i32) -> Option<(BlockRefRef<'_>, i32)> {
        self.scan_column(x, z, ymax, &ScanOpts::default()).block
    }

    /// Owning convenience wrapper around [`top_block_ref`].
    pub fn top_block(&self, x: usize, z: usize, ymax: i32) -> Option<(BlockRef, i32)> {
        self.top_block_ref(x, z, ymax)
            .map(|(b, y)| (b.to_owned_ref(), y))
    }

    /// Highest non-air block at (x,z) below ymax that is not fully transparent.
    pub fn top_visible(&self, x: usize, z: usize, ymax: i32) -> Option<(BlockRef, i32)> {
        self.top_block(x, z, ymax)
    }

    /// Scan one block column, resolving its surface and (when `water` is on)
    /// the sea floor beneath a water surface.
    pub fn scan_column(
        &self,
        x: usize,
        z: usize,
        ymax: i32,
        opts: &ScanOpts,
    ) -> ColumnScan<'_> {
        let mut out = ColumnScan::default();
        let mut in_water = false;

        for (s, top) in self.sections.iter().zip(self.section_top.iter()) {
            let base = s.y * 16;
            if base > ymax {
                continue;
            }
            // Skip sections with nothing in them (the common case above ground).
            let Some(top_local) = *top else { continue };
            let start = (top_local as i32).min(15);
            for y in (0..=start).rev() {
                let by = base + y;
                if by > ymax {
                    continue;
                }
                let Some(b) = s.block_ref(x, y as usize, z) else {
                    continue;
                };
                if b.is_air() {
                    continue;
                }

                let is_water = b.is_water();
                if !in_water {
                    // First solid thing from the top: the visible surface.
                    out.block = Some((b, by));
                    out.tint = self.biome_at(s, x, y as usize, z, by);
                    if !(opts.water && is_water) {
                        return out;
                    }
                    // It is water: keep going to find the floor.
                    in_water = true;
                    out.water_top = Some(by);
                    continue;
                }
                if is_water {
                    // Still submerged; the floor is deeper.
                    continue;
                }
                // First non-water block below the surface: the sea floor.
                out.floor_block = Some((b, by));
                return out;
            }
        }
        out
    }
}

/// Which parts of a column to resolve.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScanOpts {
    /// Look past a water surface for the sea floor.
    pub water: bool,
}

/// Result of [`ChunkData::scan_column`].
#[derive(Default)]
pub struct ColumnScan<'a> {
    /// Top-most non-air block, and its block-Y.
    pub block: Option<(BlockRefRef<'a>, i32)>,
    /// Biome tints at the surface block.
    pub tint: BiomeDef,
    /// When the surface is water: its block-Y (same as the surface height).
    pub water_top: Option<i32>,
    /// First non-water block below a water surface, and its block-Y.
    pub floor_block: Option<(BlockRefRef<'a>, i32)>,
}

/// Highest local y (0..16) containing a non-air block, or `None` if the
/// section is entirely air.
fn section_highest_non_air(s: &region::Section) -> Option<u8> {
    for y in (0..16usize).rev() {
        for z in 0..16usize {
            for x in 0..16usize {
                let is_air = match s.block_ref(x, y, z) {
                    Some(b) => b.is_air(),
                    None => true,
                };
                if !is_air {
                    return Some(y as u8);
                }
            }
        }
    }
    None
}

pub fn load_chunk(region_dir: &Path, cx: i32, cz: i32) -> Result<Option<ChunkData>, String> {
    match region::read_chunk_nbt(region_dir, cx, cz)? {
        Some(nbt_bytes) => {
            // 1.13+ chunks keep sections at the root; older ones nest them
            // under `Level`. Detect by peeking at the section parser output.
            match region::parse_sections_fast_full(&nbt_bytes) {
                Ok((s, biomes)) if !s.is_empty() => Ok(Some(ChunkData::with_biomes(s, biomes))),
                _ => {
                    let sections = region::parse_sections_modern(&nbt_bytes)?;
                    Ok(Some(ChunkData::new(sections)))
                }
            }
        }
        None => Ok(None),
    }
}

/// Which biome tint a block takes, by block name.
///
/// The JourneyMap palette stores the *untinted* grey texture colour for the
/// blocks vanilla tints (grass, leaves, ...), so without this they render as
/// flat grey. Membership follows the vanilla `grass` / `foliage` / `water` /
/// `dry_foliage` tint categories; flowers, crops and mushrooms are excluded
/// because they carry their own colours.
///
/// The decision to tint is made by [`biome::needs_tint`], which checks whether
/// the palette colour is grey. That is what keeps modded leaves (already
/// coloured in the palette) from being tinted twice; this function only says
/// *which* tint to use when a tint is due.
pub fn tint_kind(name: &str) -> TintKind {
    let base = name.split('[').next().unwrap_or(name);
    // Strip any namespace, not just "minecraft:" — modded ids like
    // "BiomesOPlenty:foliage" must classify the same way.
    let short = base.split(':').next_back().unwrap_or(base);
    let lower = short.to_ascii_lowercase();

    if is_water_name(&lower) {
        return TintKind::Water;
    }
    if lower == "leaf_litter" {
        return TintKind::DryFoliage;
    }

    // --- vanilla `grass` tint category ---
    if matches!(
        lower.as_str(),
        "grass" | "grass_block" | "tallgrass" | "short_grass" | "tall_grass"
            | "fern" | "large_fern" | "reeds" | "sugar_cane" | "double_plant"
            | "grass_path" | "wildflowers" | "bush"
    ) {
        return TintKind::Grass;
    }

    // --- vanilla `foliage` tint category ---
    if lower == "leaves" || lower == "leaves2" || lower == "vine" || lower == "vines" {
        return TintKind::Foliage;
    }
    if lower.ends_with("_leaves") || lower.ends_with("leaves") {
        return TintKind::Foliage;
    }
    if lower.ends_with("_vine") || lower.ends_with("_vines") {
        return TintKind::Foliage;
    }

    // --- modded ground cover that behaves like grass/foliage ---
    // BiomesOPlenty:foliage, Botania grass, Thaumcraft magical leaves, etc.
    if matches!(lower.as_str(), "foliage" | "shrub") {
        return TintKind::Grass;
    }
    if lower.ends_with("_foliage") || lower.contains("tallgrass") {
        return TintKind::Grass;
    }
    if lower.ends_with("_grass") && !lower.contains("nether") && !lower.contains("warped") {
        return TintKind::Grass;
    }

    TintKind::None
}

/// Resolve a palette colour into its final colour for a given biome.
///
/// Grey palette colours (the untinted vanilla textures) are multiplied by the
/// biome tint; colours the palette already stores tinted are shifted relative
/// to the plains baseline so they take the biome's hue without going dark.
pub fn resolve_color(c: [u8; 3], name: &str, biome: BiomeDef) -> [u8; 3] {
    biome::tint_color(c, tint_kind(name), biome)
}

/// Directional relief shading, in the spirit of VoxelMap's `applyHeight()`.
/// Light comes from the north-west, so slopes rising toward NW are lit.
/// `h` is a square height buffer of width `w` with a 1-block margin already applied.
fn hillshade_at(h: &[i32], w: usize, idx: usize, altitude: bool) -> f32 {
    let hc = h[idx];
    if hc == EMPTY {
        return 1.0;
    }
    let pick = |i: usize| -> i32 {
        let v = h[i];
        if v == EMPTY {
            hc
        } else {
            v
        }
    };
    // central differences; margin guarantees idx-1/idx+1/idx±w are in bounds
    let dx = (pick(idx + 1) - pick(idx - 1)) as f32 * 0.5;
    let dz = (pick(idx + w) - pick(idx - w)) as f32 * 0.5;

    // slope term: light from NW -> +dx (rising east) and +dz (rising south) are lit
    let slope = (dx + dz) * 0.18;

    // elevation term: subtle large-scale relief relative to sea level (y=64)
    let elev_term = if altitude {
        let elev = hc - 64;
        (elev as f32).signum() * ((elev.abs() as f32) / 8.0 + 1.0).log10() / 5.0
    } else {
        0.0
    };

    (1.0 + slope + elev_term).clamp(0.45, 1.55)
}

/// Surface buffer for a whole tile: color + height per block, with a 1-block
/// margin so hillshading can compare against neighbouring chunks (no seams).
///
/// Columns whose surface is water also carry the sea floor, so the shade pass
/// can blend the water colour toward it with depth (see [`Surface::shade`]).
pub struct Surface {
    pub blocks: usize,
    pub side: usize,
    pub color: Vec<[u8; 3]>,
    pub height: Vec<i32>,
    /// Water-surface block-Y for water columns, `EMPTY` otherwise.
    pub water_top: Vec<i32>,
    /// Sea-floor colour, valid where `water_top != EMPTY`.
    pub floor_color: Vec<[u8; 3]>,
    /// Sea-floor block-Y, valid where `water_top != EMPTY`.
    pub floor_height: Vec<i32>,
}

impl Surface {
    fn new(blocks: usize) -> Self {
        let side = blocks + 2;
        Surface {
            blocks,
            side,
            color: vec![[0u8, 0, 0]; side * side],
            height: vec![EMPTY; side * side],
            water_top: vec![EMPTY; side * side],
            floor_color: vec![[0u8, 0, 0]; side * side],
            floor_height: vec![EMPTY; side * side],
        }
    }

    #[inline]
    fn idx(&self, x: i32, z: i32) -> usize {
        ((z + 1) as usize) * self.side + (x + 1) as usize
    }

    /// Fill the surface from chunks covering `chunks_per_tile`, including a
    /// 1-chunk margin on every side so edge pixels shade correctly.
    /// Chunks are read/parsed in parallel, then written in a deterministic
    /// order so the result does not depend on thread scheduling.
    #[allow(clippy::too_many_arguments)]
    pub fn fill(
        &mut self,
        cache: &mut TileCache,
        region_dir: &Path,
        chunk_x0: i32,
        chunk_z0: i32,
        chunks_per_tile: i32,
        ymax: i32,
        opts: RenderOpts,
    ) {
        let lim = self.blocks as i32;
        let scan_opts = ScanOpts {
            water: opts.water,
        };

        // 1. work out which chunks are not cached yet
        let mut coords: Vec<(i32, i32)> = Vec::new();
        for cz in -1..=chunks_per_tile {
            for cx in -1..=chunks_per_tile {
                let key = (chunk_x0 + cx, chunk_z0 + cz);
                if !cache.chunks.contains_key(&key) {
                    coords.push(key);
                }
            }
        }

        // 2. read + parse the missing ones in parallel
        if !coords.is_empty() {
            let loaded = load_chunks_parallel(region_dir, &coords);
            for (key, data) in loaded {
                cache.insert_loaded(key, data);
            }
        }

        // 3. copy the visible surface out of the cache (single-threaded, ordered)
        for cz in -1..=chunks_per_tile {
            for cx in -1..=chunks_per_tile {
                let ox = cx * 16;
                let oz = cz * 16;
                if let Some(Some(chunk)) = cache.chunks.get(&(chunk_x0 + cx, chunk_z0 + cz)) {
                    for lz in 0..16i32 {
                        for lx in 0..16i32 {
                            let gx = ox + lx;
                            let gz = oz + lz;
                            if gx < -1 || gz < -1 || gx > lim || gz > lim {
                                continue;
                            }
                            let scan =
                                chunk.scan_column(lx as usize, lz as usize, ymax, &scan_opts);
                            let Some((block, y)) = scan.block else { continue };

                            let (rgb, _src, name) = cache.palette.color_ref(&block.to_owned_ref());
                            let c = resolve_color(rgb, &name, scan.tint);
                            let i = self.idx(gx, gz);
                            self.color[i] = c;
                            self.height[i] = y;

                            if scan.water_top.is_some() {
                                self.water_top[i] = y;
                                if let Some((fb, fy)) = scan.floor_block {
                                    let (frgb, _s, fname) =
                                        cache.palette.color_ref(&fb.to_owned_ref());
                                    self.floor_color[i] = resolve_color(frgb, &fname, scan.tint);
                                    self.floor_height[i] = fy;
                                } else {
                                    // Water column with no floor (void world).
                                    self.floor_color[i] = c;
                                    self.floor_height[i] = y;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Blend water toward the sea floor by depth, then apply relief shading.
    pub fn shade(&mut self, opts: RenderOpts) {
        let w = self.side;
        for z in 0..self.blocks {
            for x in 0..self.blocks {
                let i = self.idx(x as i32, z as i32);
                if self.height[i] == EMPTY {
                    continue;
                }

                if opts.water && self.water_top[i] != EMPTY {
                    // Underwater: blend the surface toward the floor by depth.
                    // Shallow water shows the floor (ratio ~0.5); by 40 blocks
                    // deep the blend fades out and deep water is pure water
                    // colour. This mirrors MCA Selector's `TileImage.shade`.
                    let depth = (self.water_top[i] - self.floor_height[i]).max(1) as f32;
                    let ratio = (0.5 - 0.5 / 40.0 * depth).clamp(0.0, 0.5);
                    self.color[i] = lerp(self.color[i], self.floor_color[i], ratio);
                }

                if opts.shading {
                    let s = hillshade_at(&self.height, w, i, opts.altitude);
                    let c = self.color[i];
                    self.color[i] = [
                        ((c[0] as f32 * s).min(255.0)) as u8,
                        ((c[1] as f32 * s).min(255.0)) as u8,
                        ((c[2] as f32 * s).min(255.0)) as u8,
                    ];
                }
            }
        }
    }

    /// Downsample the shaded surface into a TILE_SIZE RGBA image.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut img = vec![0u8; TILE_SIZE * TILE_SIZE * 4];
        let scale = TILE_SIZE / self.blocks;
        for z in 0..self.blocks {
            for x in 0..self.blocks {
                let i = self.idx(x as i32, z as i32);
                if self.height[i] == EMPTY {
                    continue;
                }
                let c = self.color[i];
                let px0 = x * scale;
                let py0 = z * scale;
                for py in py0..(py0 + scale).min(TILE_SIZE) {
                    for px in px0..(px0 + scale).min(TILE_SIZE) {
                        let idx = (py * TILE_SIZE + px) * 4;
                        img[idx] = c[0];
                        img[idx + 1] = c[1];
                        img[idx + 2] = c[2];
                        img[idx + 3] = 255;
                    }
                }
            }
        }
        img
    }
}

/// Linear interpolation between two colours, `ratio` in 0..=1.
fn lerp(a: [u8; 3], b: [u8; 3], ratio: f32) -> [u8; 3] {
    let r = ratio.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 * (1.0 - r) + y as f32 * r).round() as u8;
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])]
}

/// Render one tile (TILE_SIZE x TILE_SIZE) with relief shading.
/// Returns (png_bytes, has_any_data). `has_any_data == false` means the tile
/// covers no generated chunks at all, which the frontend renders differently
/// from a tile that simply has not loaded yet.
pub fn render_tile(
    cache: &mut TileCache,
    region_dir: &Path,
    zoom: i32,
    tile_x: i32,
    tile_row: i32,
    ymax: i32,
    opts: RenderOpts,
) -> Result<(Vec<u8>, bool), String> {
    if !(0..=4).contains(&zoom) {
        return Err(format!("zoom {} out of range 0..=4", zoom));
    }
    // z=0 -> 16 chunks/tile (1 px per block), z=4 -> 1 chunk/tile (16 px per block)
    let chunks_per_tile = 16i32 >> zoom;
    let blocks = (chunks_per_tile * 16) as usize;
    let chunk_x0 = tile_x * chunks_per_tile;
    let chunk_z0 = tile_row * chunks_per_tile;

    let mut surface = Surface::new(blocks);
    surface.fill(
        cache,
        region_dir,
        chunk_x0,
        chunk_z0,
        chunks_per_tile,
        ymax,
        opts,
    );
    // Only the tile's own area counts. The surface carries a 1-block margin
    // for hillshading, and those margin pixels are not rendered, so including
    // them would report "has data" for a tile that renders fully transparent.
    let has_data = (0..blocks as i32).any(|z| {
        (0..blocks as i32).any(|x| surface.height[surface.idx(x, z)] != EMPTY)
    });
    surface.shade(opts);
    Ok((
        encode_png(&surface.to_rgba(), TILE_SIZE as u32, TILE_SIZE as u32),
        has_data,
    ))
}

pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(rgba).expect("png data");
    }
    out
}

pub type ChunkKey = (i32, i32);

/// Read and parse a batch of chunks across worker threads.
/// The OS page cache and per-file seeks make this IO-light, so a simple
/// chunked work split is enough; no new dependency needed.
fn load_chunks_parallel(region_dir: &Path, coords: &[ChunkKey]) -> Vec<(ChunkKey, Option<ChunkData>)> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8)
        .min(coords.len().max(1));
    if threads <= 1 || coords.len() < 4 {
        return coords
            .iter()
            .map(|&k| (k, load_chunk(region_dir, k.0, k.1).ok().flatten()))
            .collect();
    }

    let per = coords.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for batch in coords.chunks(per) {
            handles.push(scope.spawn(move || {
                batch
                    .iter()
                    .map(|&k| (k, load_chunk(region_dir, k.0, k.1).ok().flatten()))
                    .collect::<Vec<_>>()
            }));
        }
        let mut out = Vec::with_capacity(coords.len());
        for h in handles {
            if let Ok(v) = h.join() {
                out.extend(v);
            }
        }
        out
    })
}

pub struct TileCache {
    pub chunks: HashMap<ChunkKey, Option<ChunkData>>,
    pub palette: Palette,
    capacity: usize,
}

impl TileCache {
    pub fn new(palette: Palette, capacity: usize) -> Self {
        TileCache {
            chunks: HashMap::new(),
            palette,
            capacity,
        }
    }

    /// Insert an already-loaded chunk, evicting if over capacity.
    pub fn insert_loaded(&mut self, key: ChunkKey, data: Option<ChunkData>) {
        if !self.chunks.contains_key(&key) && self.chunks.len() >= self.capacity {
            let keys: Vec<ChunkKey> = self
                .chunks
                .keys()
                .take(self.capacity / 4)
                .cloned()
                .collect();
            for k in keys {
                self.chunks.remove(&k);
            }
        }
        self.chunks.insert(key, data);
    }

    pub fn invalidate(&mut self) {
        self.chunks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat terrain must stay unshaded.
    #[test]
    fn hillshade_flat_is_neutral() {
        let w = 5;
        let h = vec![64i32; w * w];
        let idx = 2 * w + 2;
        assert!((hillshade_at(&h, w, idx, true) - 1.0).abs() < 1e-6);
    }

    /// Light comes from the north-west, so slopes whose surface faces NW
    /// (i.e. terrain rising toward the SE) must be lit, and the opposite
    /// slope must be darkened. This is standard cartographic hillshading.
    #[test]
    fn hillshade_lights_nw_facing_slope_and_darkens_se_facing_slope() {
        let w = 5;
        let mut rises_to_se = vec![64i32; w * w]; // faces NW -> lit
        let mut rises_to_nw = vec![64i32; w * w]; // faces SE -> dark
        for z in 0..w {
            for x in 0..w {
                rises_to_se[z * w + x] = 64 + x as i32 + z as i32;
                rises_to_nw[z * w + x] = 64 + (4 - x) as i32 + (4 - z) as i32;
            }
        }
        let idx = 2 * w + 2;
        let lit = hillshade_at(&rises_to_se, w, idx, true);
        let dark = hillshade_at(&rises_to_nw, w, idx, true);
        assert!(lit > 1.0, "NW-facing slope should be lit, got {}", lit);
        assert!(dark < 1.0, "SE-facing slope should be dark, got {}", dark);
    }

    /// A cliff must produce a much stronger response than gentle terrain.
    #[test]
    fn hillshade_cliff_is_stronger_than_gentle_slope() {
        let w = 5;
        let mut gentle = vec![64i32; w * w];
        let mut cliff = vec![64i32; w * w];
        for z in 0..w {
            for x in 0..w {
                gentle[z * w + x] = 64 + x as i32;
                cliff[z * w + x] = 64 + if x >= 2 { 12 } else { 0 };
            }
        }
        let idx = 2 * w + 2;
        let g = (hillshade_at(&gentle, w, idx, true) - 1.0).abs();
        let c = (hillshade_at(&cliff, w, idx, true) - 1.0).abs();
        assert!(c > g * 2.0, "cliff {} should dwarf gentle {}", c, g);
    }

    /// Empty columns must not blow up the shading of their neighbours.
    #[test]
    fn hillshade_handles_empty_columns() {
        let w = 5;
        let mut h = vec![EMPTY; w * w];
        let idx = 2 * w + 2;
        h[idx] = 70;
        let s = hillshade_at(&h, w, idx, true);
        assert!(s.is_finite() && s > 0.0);
    }
}
