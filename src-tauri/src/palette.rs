use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Palette {
    /// uid -> meta -> (rgb, name)
    by_uid: HashMap<String, HashMap<u16, ([u8; 3], String)>>,
    /// block id -> uid (from level.dat FML.ItemData, \x01 prefix)
    pub block_names: HashMap<u16, String>,
    pub fallback: [u8; 3],
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some([r, g, b])
}

/// Parses `var colorpalette={...};` JS wrapper into JSON.
fn strip_js_wrapper(text: &str) -> &str {
    let start = match text.find('{') {
        Some(i) => i,
        None => return text,
    };
    let end = match text.rfind('}') {
        Some(i) => i + 1,
        None => return text,
    };
    &text[start..end]
}

impl Palette {
    pub fn empty() -> Self {
        Palette {
            by_uid: HashMap::new(),
            block_names: HashMap::new(),
            fallback: [80, 80, 80],
        }
    }

    pub fn load(instance_root: &Path, level_dat: &Path) -> Result<Self, String> {
        let mut pal = Palette::empty();

        // block id -> name from level.dat FML.ItemData (\x01 = blocks)
        if let Ok(bytes) = std::fs::read(level_dat) {
            if let Ok(decompressed) = decompress_gzip(&bytes) {
                if let Ok((_, root)) = crate::nbt::parse(&decompressed) {
                    if let Some(list) = root
                        .get("FML")
                        .and_then(|f| f.get("ItemData"))
                        .and_then(|t| t.as_list())
                    {
                        for e in list {
                            let k = e.get("K").and_then(|t| t.as_str());
                            let v = e.get("V").and_then(|t| t.as_i32());
                            if let (Some(k), Some(v)) = (k, v) {
                                let mut chars = k.chars();
                                if let Some(prefix) = chars.next() {
                                    if prefix == '\u{1}' && v >= 0 && v <= u16::MAX as i32 {
                                        pal.block_names
                                            .insert(v as u16, chars.as_str().to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // colors from JourneyMap colorpalette.json
        let cp = instance_root.join("journeymap").join("colorpalette.json");
        if let Ok(text) = std::fs::read_to_string(&cp) {
            let json_text = strip_js_wrapper(&text);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_text) {
                if let Some(arr) = v.get("basicColors").and_then(|b| b.as_array()) {
                    for e in arr {
                        let uid = e.get("uid").and_then(|u| u.as_str()).unwrap_or("");
                        let meta = e.get("meta").and_then(|m| m.as_i64()).unwrap_or(0) as u16;
                        let color = e.get("color").and_then(|c| c.as_str()).unwrap_or("");
                        let name = e.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        if let Some(rgb) = parse_hex(color) {
                            pal.by_uid
                                .entry(uid.to_string())
                                .or_default()
                                .insert(meta, (rgb, name.to_string()));
                        }
                    }
                }
            }
        }
        Ok(pal)
    }

    /// Look up color by block id + meta. Returns (rgb, source, name).
    pub fn color(&self, id: u16, meta: u16) -> ([u8; 3], &'static str, String) {
        if id == 0 {
            return (self.fallback, "air", String::new());
        }
        if let Some(name) = self.block_names.get(&id) {
            if let Some(metas) = self.by_uid.get(name.as_str()) {
                if let Some((rgb, disp)) = metas.get(&meta) {
                    return (*rgb, "exact", disp.clone());
                }
                if let Some((rgb, disp)) = metas.get(&0) {
                    return (*rgb, "meta0", disp.clone());
                }
                if let Some((rgb, disp)) = metas.values().next() {
                    return (*rgb, "first", disp.clone());
                }
            }
            return (self.fallback, "unknown", name.clone());
        }
        (self.fallback, "unknown", format!("id:{}", id))
    }

    pub fn by_uid_len(&self) -> usize {
        self.by_uid.len()
    }

    pub fn name_of(&self, id: u16) -> String {
        self.block_names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("id:{}", id))
    }
}

fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut out = Vec::new();
    GzDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// JourneyMap waypoint
#[derive(Debug, Clone, serde::Serialize)]
pub struct Waypoint {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub dimension: i32,
    pub color: String,
    pub kind: String,
}

pub fn load_waypoints(instance_root: &Path, save_name: &str) -> Vec<Waypoint> {
    let dir: PathBuf = instance_root
        .join("journeymap")
        .join("data")
        .join("sp")
        .join(save_name)
        .join("waypoints");
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let v: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let dims: Vec<i32> = v
            .get("dimensions")
            .and_then(|d| d.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_i64()).map(|x| x as i32).collect())
            .unwrap_or_default();
        let rgb = format!(
            "#{:02x}{:02x}{:02x}",
            v.get("r").and_then(|x| x.as_i64()).unwrap_or(255) as u8,
            v.get("g").and_then(|x| x.as_i64()).unwrap_or(255) as u8,
            v.get("b").and_then(|x| x.as_i64()).unwrap_or(255) as u8
        );
        let enabled = v.get("enable").and_then(|e| e.as_bool()).unwrap_or(true);
        if !enabled {
            continue;
        }
        for dim in dims {
            out.push(Waypoint {
                name: v
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("waypoint")
                    .to_string(),
                x: v.get("x").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
                y: v.get("y").and_then(|x| x.as_i64()).unwrap_or(64) as i32,
                z: v.get("z").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
                dimension: dim,
                color: rgb.clone(),
                kind: v
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Normal")
                    .to_string(),
            });
        }
    }
    out
}

/// Player position from level.dat
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlayerInfo {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub dimension: i32,
    pub name: String,
}

pub fn load_player(level_dat: &Path) -> Option<PlayerInfo> {
    let bytes = std::fs::read(level_dat).ok()?;
    let decompressed = decompress_gzip(&bytes).ok()?;
    let (_, root) = crate::nbt::parse(&decompressed).ok()?;
    let player = root.get("Data").and_then(|d| d.get("Player"))?;
    let pos = player.get("Pos")?.as_f64s()?;
    if pos.len() < 3 {
        return None;
    }
    let dimension = player.get("Dimension").and_then(|t| t.as_i32()).unwrap_or(0);
    let name = player
        .get("name")
        .and_then(|t| t.as_str())
        .unwrap_or("Player")
        .to_string();
    Some(PlayerInfo {
        x: pos[0],
        y: pos[1],
        z: pos[2],
        dimension,
        name,
    })
}

/// Friendly dimension names assembled from mod configs + known defaults.
pub fn dimension_name(dim: i32, config_dir: &Path) -> Option<String> {
    let known = match dim {
        -1 => Some("下界"),
        0 => Some("主世界"),
        1 => Some("末地"),
        7 => Some("暮色森林"),
        2 => Some("幽灵世界 (RandomThings)"),
        55 => Some("梦境 (Witchery)"),
        56 => Some("苦痛维度 (Witchery)"),
        70 => Some("镜界 (Witchery)"),
        100 => Some("深暗世界 (ExtraUtilities)"),
        112 => Some("千年村庄 (ExtraUtilities)"),
        _ => None,
    };
    if let Some(k) = known {
        return Some(k.to_string());
    }
    // Galacticraft: planets.conf / core.conf
    let gc_map: &[(&str, i32)] = &[
        ("idDimensionMoon", 28),
        ("dimensionIDMars", 29),
        ("dimensionIDAsteroids", 30),
    ];
    for (key, id) in gc_map {
        if *id == dim {
            let label = match *key {
                "idDimensionMoon" => "月球",
                "dimensionIDMars" => "火星",
                "dimensionIDAsteroids" => "小行星带",
                _ => key,
            };
            return Some(format!("{} (Galacticraft)", label));
        }
    }
    // GalaxySpace dimensions.conf
    let gs = config_dir.join("GalaxySpace").join("dimensions.conf");
    if let Ok(text) = std::fs::read_to_string(&gs) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                if let Some(name) = k.strip_prefix("I:dimensionID") {
                    if v.trim().parse::<i32>().ok() == Some(dim) {
                        return Some(format!("{} (GalaxySpace)", name));
                    }
                }
            }
        }
    }
    None
}

pub fn find_level_dat(save_dir: &Path) -> PathBuf {
    save_dir.join("level.dat")
}
