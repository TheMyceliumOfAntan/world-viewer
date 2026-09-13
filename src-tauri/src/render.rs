use std::collections::HashMap;
use std::path::Path;

use crate::palette::Palette;
use crate::region;

pub const TILE_SIZE: usize = 256;
const EMPTY: i32 = i32::MIN;

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

/// Directional relief shading, in the spirit of VoxelMap's `applyHeight()`.
/// Light comes from the north-west, so slopes rising toward NW are lit.
/// `h` is a square height buffer of width `w` with a 1-block margin already applied.
fn hillshade_at(h: &[i32], w: usize, idx: usize) -> f32 {
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
    let elev = hc - 64;
    let elev_term = (elev as f32).signum() * ((elev.abs() as f32) / 8.0 + 1.0).log10() / 5.0;

    (1.0 + slope + elev_term).clamp(0.45, 1.55)
}

/// Surface buffer for a whole tile: color + height per block, with a 1-block
/// margin so hillshading can compare against neighbouring chunks (no seams).
pub struct Surface {
    pub blocks: usize,
    pub side: usize,
    pub color: Vec<[u8; 3]>,
    pub height: Vec<i32>,
}

impl Surface {
    fn new(blocks: usize) -> Self {
        let side = blocks + 2;
        Surface {
            blocks,
            side,
            color: vec![[0u8, 0, 0]; side * side],
            height: vec![EMPTY; side * side],
        }
    }

    #[inline]
    fn idx(&self, x: i32, z: i32) -> usize {
        ((z + 1) as usize) * self.side + (x + 1) as usize
    }

    /// Fill the surface from chunks covering `chunks_per_tile`, including a
    /// 1-chunk margin on every side so edge pixels shade correctly.
    pub fn fill(
        &mut self,
        cache: &mut TileCache,
        region_dir: &Path,
        chunk_x0: i32,
        chunk_z0: i32,
        chunks_per_tile: i32,
        ymax: i32,
        tint: bool,
    ) {
        for cz in -1..=chunks_per_tile {
            for cx in -1..=chunks_per_tile {
                cache.ensure(region_dir, chunk_x0 + cx, chunk_z0 + cz);
                let ox = cx * 16;
                let oz = cz * 16;
                let lim = self.blocks as i32;
                // Copy the visible surface out of the cache before touching
                // `self`, so the cache borrow ends here.
                let mut hits: Vec<(i32, i32, u16, u16, i32)> = Vec::new();
                if let Some(Some(chunk)) = cache.chunks.get(&(chunk_x0 + cx, chunk_z0 + cz)) {
                    for lz in 0..16i32 {
                        for lx in 0..16i32 {
                            let gx = ox + lx;
                            let gz = oz + lz;
                            if gx < -1 || gz < -1 || gx > lim || gz > lim {
                                continue;
                            }
                            if let Some((id, meta, y)) =
                                chunk.top_visible(lx as usize, lz as usize, ymax)
                            {
                                hits.push((gx, gz, id, meta, y));
                            }
                        }
                    }
                }
                for (gx, gz, id, meta, y) in hits {
                    let (rgb, _src, _name) = cache.palette.color(id, meta);
                    let mut c = rgb;
                    if tint {
                        let name = cache.palette.name_of(id);
                        if name == "minecraft:grass" || name == "minecraft:leaves" {
                            c = [
                                (c[0] as f32 * 0.55 + 60.0) as u8,
                                (c[1] as f32 * 0.85 + 40.0) as u8,
                                (c[2] as f32 * 0.45) as u8,
                            ];
                        }
                    }
                    let i = self.idx(gx, gz);
                    self.color[i] = c;
                    self.height[i] = y;
                }
            }
        }
    }

    /// Apply hillshading in place.
    pub fn shade(&mut self) {
        let w = self.side;
        for z in 0..self.blocks {
            for x in 0..self.blocks {
                let i = self.idx(x as i32, z as i32);
                if self.height[i] == EMPTY {
                    continue;
                }
                let s = hillshade_at(&self.height, w, i);
                let c = self.color[i];
                self.color[i] = [
                    ((c[0] as f32 * s).min(255.0)) as u8,
                    ((c[1] as f32 * s).min(255.0)) as u8,
                    ((c[2] as f32 * s).min(255.0)) as u8,
                ];
            }
        }
    }

    /// Downsample the shaded surface into a TILE_SIZE RGBA image.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut img = vec![0u8; TILE_SIZE * TILE_SIZE * 4];
        let scale = TILE_SIZE / self.blocks;
        for z in 0..self.blocks {
            for x in 0..self.blocks {
                let c = self.color[self.idx(x as i32, z as i32)];
                if c == [0, 0, 0] {
                    continue;
                }
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

/// Render one tile (TILE_SIZE x TILE_SIZE) with relief shading.
pub fn render_tile(
    cache: &mut TileCache,
    region_dir: &Path,
    zoom: i32,
    tile_x: i32,
    tile_row: i32,
    ymax: i32,
) -> Result<Vec<u8>, String> {
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
        true,
    );
    surface.shade();
    Ok(encode_png(&surface.to_rgba(), TILE_SIZE as u32, TILE_SIZE as u32))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat terrain must stay unshaded.
    #[test]
    fn hillshade_flat_is_neutral() {
        let w = 5;
        let h = vec![64i32; w * w];
        let idx = 2 * w + 2;
        assert!((hillshade_at(&h, w, idx) - 1.0).abs() < 1e-6);
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
        let lit = hillshade_at(&rises_to_se, w, idx);
        let dark = hillshade_at(&rises_to_nw, w, idx);
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
        let g = (hillshade_at(&gentle, w, idx) - 1.0).abs();
        let c = (hillshade_at(&cliff, w, idx) - 1.0).abs();
        assert!(c > g * 2.0, "cliff {} should dwarf gentle {}", c, g);
    }

    /// Empty columns must not blow up the shading of their neighbours.
    #[test]
    fn hillshade_handles_empty_columns() {
        let w = 5;
        let mut h = vec![EMPTY; w * w];
        let idx = 2 * w + 2;
        h[idx] = 70;
        let s = hillshade_at(&h, w, idx);
        assert!(s.is_finite() && s > 0.0);
    }
}
