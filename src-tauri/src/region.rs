use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use flate2::read::{GzDecoder, ZlibDecoder};

use crate::nbt::{self, Tag};

pub fn region_path(region_dir: &Path, cx: i32, cz: i32) -> PathBuf {
    region_dir.join(format!("r.{}.{}.mca", cx >> 5, cz >> 5))
}

/// Returns decompressed chunk NBT bytes, or None if the chunk is not present.
pub fn read_chunk_nbt(region_dir: &Path, cx: i32, cz: i32) -> Result<Option<Vec<u8>>, String> {
    let path = region_path(region_dir, cx, cz);
    let mut f = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let mut header = [0u8; 8192];
    if f.read_exact(&mut header).is_err() {
        return Ok(None);
    }
    let lx = (cx & 31) as usize;
    let lz = (cz & 31) as usize;
    let idx = (lx + lz * 32) * 4;
    let offset = ((header[idx] as usize) << 16 | (header[idx + 1] as usize) << 8
        | header[idx + 2] as usize)
        * 4096;
    if offset == 0 {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(offset as u64))
        .map_err(|e| e.to_string())?;
    let mut len_buf = [0u8; 5];
    if f.read_exact(&mut len_buf).is_err() {
        return Ok(None);
    }
    let len = u32::from_be_bytes([len_buf[0], len_buf[1], len_buf[2], len_buf[3]]) as usize;
    if len <= 1 {
        return Ok(None);
    }
    let comp = len_buf[4];
    let mut raw = vec![0u8; len - 1];
    if f.read_exact(&mut raw).is_err() {
        return Ok(None);
    }
    match comp {
        1 => {
            let mut out = Vec::new();
            GzDecoder::new(&raw[..])
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            Ok(Some(out))
        }
        2 => {
            let mut out = Vec::new();
            ZlibDecoder::new(&raw[..])
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            Ok(Some(out))
        }
        3 => Ok(Some(raw)),
        other => Err(format!("unknown region compression {}", other)),
    }
}

pub struct Section {
    pub y: i32,
    pub blocks16: Option<Vec<u8>>,
    pub blocks: Option<Vec<u8>>,
    pub data16: Option<Vec<u8>>,
    pub data: Option<Vec<u8>>,
    pub add: Option<Vec<u8>>,
}

impl Section {
    /// Returns (block_id, meta) for local coords 0..15.
    pub fn block(&self, x: usize, y: usize, z: usize) -> (u16, u16) {
        let i = (y * 16 + z) * 16 + x;
        if let Some(b) = &self.blocks16 {
            if i * 2 + 1 >= b.len() {
                return (0, 0);
            }
            let id = ((b[i * 2] as u16) << 8) | b[i * 2 + 1] as u16;
            let meta = match &self.data16 {
                Some(d) if i * 2 + 1 < d.len() => ((d[i * 2] as u16) << 8) | d[i * 2 + 1] as u16,
                _ => 0,
            };
            return (id, meta);
        }
        if let Some(b) = &self.blocks {
            if i >= b.len() {
                return (0, 0);
            }
            let mut id = b[i] as u16;
            if let Some(add) = &self.add {
                if i / 2 < add.len() {
                    let nib = add[i / 2];
                    let v = if i % 2 == 0 { nib & 0x0F } else { nib >> 4 };
                    id |= (v as u16) << 8;
                }
            }
            let meta = match &self.data {
                Some(d) if i / 2 < d.len() => {
                    let nib = d[i / 2];
                    if i % 2 == 0 {
                        (nib & 0x0F) as u16
                    } else {
                        (nib >> 4) as u16
                    }
                }
                _ => 0,
            };
            return (id, meta);
        }
        (0, 0)
    }
}

pub fn parse_sections(chunk_nbt: &[u8]) -> Result<Vec<Section>, String> {
    let (_, root) = nbt::parse(chunk_nbt)?;
    let level = root.get("Level").ok_or("chunk has no Level tag")?;
    let mut out = Vec::new();
    if let Some(list) = level.get("Sections").and_then(|t| t.as_list()) {
        for s in list {
            let y = s.get("Y").and_then(|t| t.as_i32()).unwrap_or(0);
            let get_bytes = |k: &str| -> Option<Vec<u8>> {
                s.get(k).and_then(|t| match t {
                    Tag::ByteArray(b) => Some(b.clone()),
                    _ => None,
                })
            };
            out.push(Section {
                y,
                blocks16: get_bytes("Blocks16"),
                blocks: get_bytes("Blocks"),
                data16: get_bytes("Data16"),
                data: get_bytes("Data"),
                add: get_bytes("Add"),
            });
        }
    }
    out.sort_by(|a, b| b.y.cmp(&a.y));
    Ok(out)
}
