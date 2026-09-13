use std::collections::HashMap;
use std::path::Path;

use crate::palette::Palette;
use crate::region;

pub const CHUNK: usize = 16;

/// A fully parsed chunk: block ids/metas as two 16x16x16-per-section layers.
pub struct ChunkData {
    pub sections: Vec<region::Section>,
}

impl ChunkData {
    /// Top-most non-air block at (x,z) with block-y <= ymax. Returns (id, meta, y).
    pub fn top_block(&self, x: usize, z: usize, ymax: i32) -> Option<(u16, u16, i32)> {
        for s in &self.sections {
            let base = s.y * 16;
            if base > ymax {
                continue;
            }
            for y in (0..16).rev() {
                let by = base + y as i32;
                if by > ymax {
                    continue;
                }
                let (id, meta) = s.block(x, y, z);
                if id != 0 {
                    return Some((id, meta, by));
                }
            }
        }
        None
    }

    /// Highest non-air block at (x,z) below ymax that is not fully transparent.
    pub fn top_visible(&self, x: usize, z: usize, ymax: i32) -> Option<(u16, u16, i32)> {
        self.top_block(x, z, ymax)
    }
}

pub fn load_chunk(region_dir: &Path, cx: i32, cz: i32) -> Result<Option<ChunkData>, String> {
    match region::read_chunk_nbt(region_dir, cx, cz)? {
        Some(nbt_bytes) => {
            let sections = region::parse_sections(&nbt_bytes)?;
            Ok(Some(ChunkData { sections }))
        }
        None => Ok(None),
    }
}

/// Render one chunk column into a 16x16 RGB buffer (top-down, ymax slice).
pub fn render_chunk(
    chunk: &ChunkData,
    palette: &Palette,
    ymax: i32,
    tint: bool,
) -> Vec<[u8; 3]> {
    let mut out = vec![[0u8, 0, 0]; CHUNK * CHUNK];
    for z in 0..CHUNK {
        for x in 0..CHUNK {
            let px = z * CHUNK + x;
            if let Some((id, meta, y)) = chunk.top_visible(x, z, ymax) {
                let (rgb, _src, _name) = palette.color(id, meta);
                let mut c = rgb;
                if tint {
                    // grass/leaves foliage tint approximation
                    let name = palette.name_of(id);
                    if name == "minecraft:grass" || name == "minecraft:leaves" {
                        c = [
                            (c[0] as f32 * 0.55 + 60.0) as u8,
                            (c[1] as f32 * 0.85 + 40.0) as u8,
                            (c[2] as f32 * 0.45) as u8,
                        ];
                    }
                }
                let shade = ((y + 64) as f32 / 192.0).clamp(0.35, 1.15);
                out[px] = [
                    (c[0] as f32 * shade).min(255.0) as u8,
                    (c[1] as f32 * shade).min(255.0) as u8,
                    (c[2] as f32 * shade).min(255.0) as u8,
                ];
            }
        }
    }
    out
}

pub type ChunkKey = (i32, i32);

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

    /// Load the chunk into the map if not present.
    pub fn ensure(&mut self, region_dir: &Path, cx: i32, cz: i32) {
        if self.chunks.contains_key(&(cx, cz)) {
            return;
        }
        if self.chunks.len() >= self.capacity {
            let keys: Vec<ChunkKey> = self.chunks.keys().take(self.capacity / 2).cloned().collect();
            for k in keys {
                self.chunks.remove(&k);
            }
        }
        let loaded = load_chunk(region_dir, cx, cz).ok().flatten();
        self.chunks.insert((cx, cz), loaded);
    }

    pub fn invalidate(&mut self) {
        self.chunks.clear();
    }
}
