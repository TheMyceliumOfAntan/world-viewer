//! End-to-end: point the viewer at a synthetic instance that contains all
//! three waypoint formats, and confirm all are loaded and merged.

use std::path::PathBuf;

use world_viewer_lib::testing;

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("wv-waypoint-e2e").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn loads_all_three_sources_together() {
    let root = tmp_dir("instance");
    let save = root.join("saves").join("TestWorld");
    std::fs::create_dir_all(&save).unwrap();

    // --- JourneyMap ---
    let jm = root.join("journeymap").join("data").join("sp").join("TestWorld").join("waypoints");
    std::fs::create_dir_all(&jm).unwrap();
    std::fs::write(
        jm.join("home.json"),
        r#"{"name":"jm-home","x":10,"y":64,"z":20,"r":255,"g":0,"b":0,
            "enable":true,"type":"Normal","dimensions":[0]}"#,
    )
    .unwrap();

    // --- Xaero (minimap tree, dim%0 and dim%-1) ---
    let xa = root.join("xaero").join("minimap").join("TestWorld");
    std::fs::create_dir_all(xa.join("dim%0")).unwrap();
    std::fs::create_dir_all(xa.join("dim%-1")).unwrap();
    std::fs::write(
        xa.join("dim%0").join("waypoints.txt"),
        "#\n#waypoint:name:initials:x:y:z:color:disabled:type:set:rotate_on_tp:tp_yaw:visibility_type:destination\n#\n\
         waypoint:xa-home:H:100:65:-200:13:false:0:gui.xaero_default:false:0:0:false\n\
         waypoint:gui.xaero_deathpoint:D:-50:70:30:0:false:1:gui.xaero_default:false:0:1:true\n\
         waypoint:disabled-one:X:1:2:3:0:true:0:gui.xaero_default:false:0:0:false\n",
    )
    .unwrap();
    std::fs::write(
        xa.join("dim%-1").join("waypoints.txt"),
        "waypoint:nether-spot:N:-426:74:-867:0:false:0:gui.xaero_default:false:0:0:false\n",
    )
    .unwrap();

    // --- VoxelMap ---
    let vm = root.join("voxelmap");
    std::fs::create_dir_all(&vm).unwrap();
    std::fs::write(
        vm.join("TestWorld.points"),
        "name:vm-point,x:256,z:233,y:63,enabled:true,red:0.4,green:0.24,blue:0.57,suffix:,world:,dimensions:0#\n\
         name:vm-nether,x:5,z:6,y:70,enabled:true,red:1.0,green:1.0,blue:1.0,suffix:,world:,dimensions:-1#\n\
         name:vm-disabled,x:9,z:9,y:9,enabled:false,red:0.0,green:0.0,blue:0.0,suffix:,world:,dimensions:0#\n",
    )
    .unwrap();

    let wps = testing::load_waypoints(&root, "TestWorld");

    let jm_count = wps.iter().filter(|w| w.source == "journeymap").count();
    let xa_count = wps.iter().filter(|w| w.source == "xaero").count();
    let vm_count = wps.iter().filter(|w| w.source == "voxelmap").count();

    assert_eq!(jm_count, 1, "JourneyMap: {:?}", wps);
    // xaero: 2 overworld (home + deathpoint) + 1 nether; disabled skipped
    assert_eq!(xa_count, 3, "Xaero: {:?}", wps);
    // voxelmap: 2 enabled (overworld + nether); disabled skipped
    assert_eq!(vm_count, 2, "VoxelMap: {:?}", wps);

    // Dimension routing must be per-source correct.
    let xa_home = wps.iter().find(|w| w.name == "xa-home").unwrap();
    assert_eq!(xa_home.dimension, 0);
    assert_eq!((xa_home.x, xa_home.y, xa_home.z), (100, 65, -200));

    let xa_nether = wps.iter().find(|w| w.name == "nether-spot").unwrap();
    assert_eq!(xa_nether.dimension, -1, "dim%-1 must become -1");

    let vm_nether = wps.iter().find(|w| w.name == "vm-nether").unwrap();
    assert_eq!(vm_nether.dimension, -1, "dimensions:-1 must become -1");

    // Death point naming and kind.
    let death = wps.iter().find(|w| w.kind == "Death").unwrap();
    assert_eq!(death.name, "死亡点", "xaero i18n death key must be localised");

    // Disabled entries from both mods must be dropped.
    assert!(!wps.iter().any(|w| w.name == "disabled-one"));
    assert!(!wps.iter().any(|w| w.name == "vm-disabled"));

    eprintln!(
        "merged: {} journeymap + {} xaero + {} voxelmap = {} total",
        jm_count,
        xa_count,
        vm_count,
        wps.len()
    );
}

#[test]
fn instance_without_any_waypoint_data_is_safe() {
    let root = tmp_dir("empty-instance");
    std::fs::create_dir_all(root.join("saves").join("W")).unwrap();
    let wps = testing::load_waypoints(&root, "W");
    assert!(wps.is_empty(), "expected no waypoints, got {:?}", wps);
}
