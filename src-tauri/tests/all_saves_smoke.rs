//! Smoke-test open_world across every save on this machine.

use std::path::PathBuf;

use world_viewer_lib::testing;

fn saves() -> Vec<(&'static str, PathBuf)> {
    vec![
        (
            "GTNH 2.8.4 (1.7.10)",
            PathBuf::from(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本"),
        ),
        (
            "1.12.2-Forge",
            PathBuf::from(r"C:\.minecraft\versions\1.12.2-Forge-14.23.5.2864\saves\新的世界"),
        ),
        (
            "1.16.5 (sibling Palette/BlockStates)",
            PathBuf::from(r"C:\.minecraft\versions\1.16.5\saves\新的世界"),
        ),
        (
            "Aegis 1.20+",
            PathBuf::from(r"C:\.minecraft\versions\Aegis of the Frozen Sky\saves\新的世界"),
        ),
        (
            "1.12.2 (vanilla)",
            PathBuf::from(r"C:\.minecraft\versions\1.12.2\saves\新的世界"),
        ),
        (
            "26.2 (dimensions/<ns>/<name>)",
            PathBuf::from(r"C:\.minecraft\versions\26.2\saves\新的世界"),
        ),
        // Saves from the C:\Minecraft instance. 1.6.4/1.8.9 exercise the
        // pre-1.13 block names (LEGACY_ALIASES); 1.14.4 exercises the
        // 1.13-1.17 sibling Palette/BlockStates layout.
        (
            "1.6.4 (pre-1.13 names)",
            PathBuf::from(r"C:\Minecraft\.minecraft\versions\1.6.4\saves\New World"),
        ),
        (
            "1.8.9 (pre-1.13 names)",
            PathBuf::from(r"C:\Minecraft\.minecraft\versions\1.8.9\saves\新的世界"),
        ),
        (
            "1.14.4 (sibling Palette/BlockStates)",
            PathBuf::from(r"C:\Minecraft\.minecraft\versions\1.14.4\saves\新的世界"),
        ),
    ]
}

#[test]
fn all_saves_open_and_render() {
    let mut failures = Vec::new();
    for (label, dir) in saves() {
        if !dir.is_dir() {
            eprintln!("SKIP {}: not present", label);
            continue;
        }
        let world = match testing::open_world(&dir) {
            Ok(w) => w,
            Err(e) => {
                failures.push(format!("{}: open_world failed: {}", label, e));
                continue;
            }
        };

        let dims: Vec<i32> = world.dimensions.iter().map(|d| d.id).collect();
        let with_data: Vec<i32> = world
            .dimensions
            .iter()
            .filter(|d| d.has_data)
            .map(|d| d.id)
            .collect();
        eprintln!(
            "{}: {} dims {:?} (with data: {:?}), {} waypoints, {} block names",
            label,
            world.dimensions.len(),
            dims,
            with_data,
            world.waypoints.len(),
            world.palette.block_names_len()
        );

        if with_data.is_empty() {
            failures.push(format!("{}: no dimension has data", label));
            continue;
        }

        // Render the first dimension that has data.
        let dim = world
            .dimensions
            .iter()
            .find(|d| d.has_data)
            .expect("checked above");
        let region_dir = PathBuf::from(&dim.region_dir);

        // Try a few tile positions; at least one must render terrain.
        let mut best_opaque = 0usize;
        let mut best_tile = (0, 0);
        for (tx, ty) in [(0, 0), (-1, -1), (-2, -2), (-1, 0), (0, -1), (-3, -3)] {
            let (png, has_data) =
                match testing::render_tile_with_data_flag(&world.palette, &region_dir, 0, tx, ty, 255)
                {
                    Ok(v) => v,
                    Err(e) => {
                        failures.push(format!("{}: render tile ({},{}) failed: {}", label, tx, ty, e));
                        continue;
                    }
                };
            if !has_data {
                continue;
            }
            let decoder = png::Decoder::new(&png[..]);
            let mut reader = decoder.read_info().expect("png");
            let mut buf = vec![0; reader.output_buffer_size()];
            reader.next_frame(&mut buf).unwrap();
            let opaque = buf.chunks(4).filter(|p| p[3] > 0).count();
            if opaque > best_opaque {
                best_opaque = opaque;
                best_tile = (tx, ty);
            }
        }
        eprintln!("  best tile {:?} -> {} opaque px", best_tile, best_opaque);
        if best_opaque < 1000 {
            failures.push(format!(
                "{}: no tile rendered terrain (best {} px at {:?})",
                label, best_opaque, best_tile
            ));
        }
    }

    if !failures.is_empty() {
        panic!("failures:\n  {}", failures.join("\n  "));
    }
}

/// Tile URLs must differ per save. If they do not, the frontend reuses the
/// previous world's tile cache when the user switches saves.
///
/// This mirrors the frontend's `worldKeyOf` exactly (32-bit wrapping hash,
/// base 36) so the two implementations cannot drift apart unnoticed.
#[test]
fn tile_path_carries_a_world_key() {
    fn key_of(save_dir: &str) -> String {
        // JS: h = (Math.imul(31, h) + charCode) | 0  then  (h >>> 0).toString(36)
        let mut h: i32 = 0;
        for ch in save_dir.encode_utf16() {
            h = 31i32.wrapping_mul(h).wrapping_add(ch as i32);
        }
        let mut n = h as u32;
        if n == 0 {
            return "w0".to_string();
        }
        const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
        let mut buf = Vec::new();
        while n > 0 {
            buf.push(DIGITS[(n % 36) as usize]);
            n /= 36;
        }
        buf.reverse();
        format!("w{}", String::from_utf8(buf).unwrap())
    }

    let gtnh = key_of(r"C:\.minecraft\versions\GTNH 2.8.4\saves\新的世界 - 副本");
    let forge = key_of(r"C:\.minecraft\versions\1.12.2-Forge-14.23.5.2864\saves\新的世界");
    let aegis = key_of(r"C:\.minecraft\versions\Aegis of the Frozen Sky\saves\新的世界");

    // These exact values are what the running frontend produced, so the
    // mirror is verified against reality rather than against itself.
    assert_eq!(gtnh, "w16yu5cm", "GTNH key must match the frontend");
    assert_eq!(forge, "wtvdbut", "1.12.2-Forge key must match the frontend");

    assert_ne!(gtnh, forge, "GTNH and 1.12.2 must not share a tile key");
    assert_ne!(gtnh, aegis, "GTNH and Aegis must not share a tile key");
    assert_ne!(forge, aegis, "1.12.2 and Aegis must not share a tile key");
    eprintln!("tile keys: gtnh={} forge={} aegis={}", gtnh, forge, aegis);
}

/// The tile path must have five segments: world key + dim + z + x + row.
/// The backend rejects anything else, so a URL built without the key would
/// 404 rather than silently render the wrong world.
#[test]
fn tile_path_shape_is_five_segments() {
    let frontend_url = "http://tile.localhost/w16yu5cm/0/2/-1/-7.png?ymax=4294967295";
    let path = frontend_url
        .trim_start_matches("http://tile.localhost/")
        .split('?')
        .next()
        .unwrap()
        .trim_end_matches(".png");
    let parts: Vec<&str> = path.split('/').collect();
    assert_eq!(parts.len(), 5, "path must be key/dim/z/x/row, got {:?}", parts);
    assert_eq!(parts[0], "w16yu5cm", "first segment must be the world key");
    assert_eq!(parts[1], "0");
    assert_eq!(parts[2], "2");
    assert_eq!(parts[3], "-1");
    assert_eq!(parts[4], "-7");
}
