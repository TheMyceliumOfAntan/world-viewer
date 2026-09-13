use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::nbt;
use crate::palette::{self, Palette, PlayerInfo, Waypoint};

#[derive(Debug, Clone, serde::Serialize)]
pub struct DimensionInfo {
    pub id: i32,
    pub name: String,
    pub region_dir: String,
    pub chunk_count: u32,
    pub has_data: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorldInfo {
    pub save_dir: String,
    pub save_name: String,
    pub level_name: String,
    pub instance_root: String,
    pub dimensions: Vec<DimensionInfo>,
    pub player: Option<PlayerInfo>,
    pub waypoints: Vec<Waypoint>,
    pub palette_entries: usize,
    pub palette_mapped_blocks: usize,
}

pub struct World {
    pub save_dir: PathBuf,
    pub instance_root: PathBuf,
    pub level_dat: PathBuf,
    pub dimensions: Vec<DimensionInfo>,
    pub palette: Palette,
    pub waypoints: Vec<Waypoint>,
    pub player: Option<PlayerInfo>,
}

fn count_regions(dir: &Path) -> (u32, u32) {
    let mut files = 0u32;
    let mut chunks = 0u32;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return (0, 0),
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("mca") {
            continue;
        }
        files += 1;
        if let Ok(data) = std::fs::read(&p) {
            if data.len() >= 8192 {
                for i in 0..1024 {
                    let cnt = data[i * 4 + 3];
                    if cnt != 0 {
                        chunks += 1;
                    }
                }
            }
        }
    }
    (files, chunks)
}

/// Detect instance root: walk up from save dir until a `journeymap` or `mods` dir is found.
fn detect_instance_root(save_dir: &Path) -> PathBuf {
    let mut cur = save_dir.to_path_buf();
    for _ in 0..5 {
        if cur.join("journeymap").exists() || cur.join("mods").exists() {
            return cur;
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => break,
        }
    }
    save_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| save_dir.to_path_buf())
}

pub fn open_world(save_dir: &Path, instance_root_override: Option<PathBuf>) -> Result<World, String> {
    if !save_dir.is_dir() {
        return Err(format!("存档目录不存在: {}", save_dir.display()));
    }
    let level_dat = palette::find_level_dat(save_dir);
    if !level_dat.is_file() {
        return Err(format!("找不到 level.dat: {}", level_dat.display()));
    }
    let instance_root = instance_root_override
        .unwrap_or_else(|| detect_instance_root(save_dir));

    let palette = Palette::load(&instance_root, &level_dat)?;
    let save_name = read_level_name(&level_dat).unwrap_or_else(|| {
        save_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    let mut dimensions = Vec::new();
    // Overworld
    let (files, chunks) = count_regions(&save_dir.join("region"));
    dimensions.push(DimensionInfo {
        id: 0,
        name: palette::dimension_name(0, &instance_root.join("config"))
            .unwrap_or_else(|| "主世界".into()),
        region_dir: save_dir.join("region").to_string_lossy().into_owned(),
        chunk_count: chunks,
        has_data: files > 0,
    });

    // DIM* dirs
    let mut dim_ids: Vec<i32> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(save_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(rest) = name.strip_prefix("DIM") {
                if let Ok(id) = rest.parse::<i32>() {
                    dim_ids.push(id);
                }
            }
        }
    }
    dim_ids.sort();
    for id in dim_ids {
        let dir = save_dir.join(format!("DIM{}", id)).join("region");
        let (files, chunks) = count_regions(&dir);
        if files == 0 {
            continue;
        }
        dimensions.push(DimensionInfo {
            id,
            name: palette::dimension_name(id, &instance_root.join("config"))
                .unwrap_or_else(|| format!("DIM{}", id)),
            region_dir: dir.to_string_lossy().into_owned(),
            chunk_count: chunks,
            has_data: true,
        });
    }

    let waypoints = palette::load_waypoints(&instance_root, &save_name);
    let player = palette::load_player(&level_dat);

    Ok(World {
        save_dir: save_dir.to_path_buf(),
        instance_root,
        level_dat,
        dimensions,
        palette,
        waypoints,
        player,
    })
}

pub fn read_level_name(level_dat: &Path) -> Option<String> {
    let bytes = std::fs::read(level_dat).ok()?;
    let mut out = Vec::new();
    {
        use flate2::read::GzDecoder;
        use std::io::Read;
        GzDecoder::new(&bytes[..]).read_to_end(&mut out).ok()?;
    }
    let (_, root) = nbt::parse(&out).ok()?;
    root.get("Data")?
        .get("LevelName")?
        .as_str()
        .map(|s| s.to_string())
}

impl World {
    pub fn info(&self) -> WorldInfo {
        WorldInfo {
            save_dir: self.save_dir.to_string_lossy().into_owned(),
            save_name: read_level_name(&self.level_dat).unwrap_or_default(),
            level_name: read_level_name(&self.level_dat).unwrap_or_default(),
            instance_root: self.instance_root.to_string_lossy().into_owned(),
            dimensions: self.dimensions.clone(),
            player: self.player.clone(),
            waypoints: self.waypoints.clone(),
            palette_entries: self.palette.by_uid_len(),
            palette_mapped_blocks: self.palette.block_names.len(),
        }
    }

    pub fn dimension(&self, id: i32) -> Option<&DimensionInfo> {
        self.dimensions.iter().find(|d| d.id == id)
    }
}

/// Map of dim id -> palette lookups used by the renderer.
pub fn dims_by_id(world: &World) -> HashMap<i32, String> {
    world
        .dimensions
        .iter()
        .map(|d| (d.id, d.name.clone()))
        .collect()
}
