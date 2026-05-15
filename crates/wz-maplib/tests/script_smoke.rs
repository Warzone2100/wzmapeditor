//! End-to-end tests for the script-map runtime.

use std::path::PathBuf;

use wz_maplib::io_wz::{ScriptError, run_script_map, run_script_source};

#[test]
fn syntax_error_is_reported() {
    let source = "this is not valid javascript $$$";
    let err = run_script_source("syntax", source, 0).unwrap_err();
    match err {
        ScriptError::Compile(msg) => assert!(!msg.is_empty()),
        e => panic!("unexpected error: {e:?}"),
    }
}

#[test]
fn missing_set_map_data_is_reported() {
    let source = r#"
        var x = gameRand();
        var y = gameRand(100);
        log("x=" + x);
        log("y=" + y);
    "#;
    let err = run_script_source("smoke", source, 42).unwrap_err();
    match err {
        ScriptError::Other(msg) => assert!(msg.contains("setMapData"), "{msg}"),
        e => panic!("unexpected error: {e:?}"),
    }
}

#[test]
fn game_rand_is_deterministic_for_same_seed() {
    let source = r#"
        var x = gameRand();
        throw new Error("VALUE=" + x);
    "#;
    let a = run_script_source("rng", source, 12345)
        .unwrap_err()
        .to_string();
    let b = run_script_source("rng", source, 12345)
        .unwrap_err()
        .to_string();
    assert_eq!(a, b);

    let c = run_script_source("rng", source, 67890)
        .unwrap_err()
        .to_string();
    assert_ne!(a, c);
}

#[test]
fn set_map_data_populates_minimal_map() {
    let source = r#"
        setMapData(
            2, 2,
            [0, 1, 2, 3],
            [10, 20, 30, 40],
            [{ name: "A0CommandCentre", position: [128, 128], direction: 0, modules: 0, player: 0 }],
            [{ name: "ViperMG", position: [256, 384], direction: 16384, player: 1 }],
            [{ name: "Tree1", position: [512, 640], direction: 0 }]
        );
    "#;
    let map = run_script_source("setmap", source, 0).expect("should succeed");
    assert_eq!(map.map_data.width, 2);
    assert_eq!(map.map_data.height, 2);
    assert_eq!(map.map_data.tiles.len(), 4);
    assert_eq!(map.map_data.tiles[0].texture, 0);
    assert_eq!(map.map_data.tiles[3].texture, 3);
    assert_eq!(map.map_data.tiles[1].height, 20);

    assert_eq!(map.structures.len(), 1);
    assert_eq!(map.structures[0].name, "A0CommandCentre");
    assert_eq!(map.structures[0].player, 0);
    assert_eq!(map.structures[0].position.x, 128);

    assert_eq!(map.droids.len(), 1);
    assert_eq!(map.droids[0].player, 1);
    assert_eq!(map.droids[0].direction, 16384);

    assert_eq!(map.features.len(), 1);
    assert_eq!(map.features[0].name, "Tree1");
}

#[test]
fn fractal_noise_returns_array_of_expected_size() {
    let source = r#"
        var w = 4, h = 4;
        var noise = generateFractalValueNoise(w, h, 100, 5, 4);
        if (!Array.isArray(noise)) throw new Error("not array");
        if (noise.length !== w * h) throw new Error("bad length: " + noise.length);
        var tex = [];
        for (var i = 0; i < w * h; ++i) tex.push(0);
        setMapData(w, h, tex, noise, [], [], []);
    "#;
    let map = run_script_source("noise", source, 7).expect("should succeed");
    assert_eq!(map.map_data.width, 4);
    assert_eq!(map.map_data.height, 4);
    assert!(map.map_data.tiles.iter().any(|t| t.height != 0));
}

/// Run the real Warzone `azuda.wz` script map. Skipped automatically when
/// the fixture isn't present (so the test stays portable across checkouts).
#[test]
fn azuda_wz_loads_with_fixed_seed() {
    let path = PathBuf::from("/Users/liamy/personal/wzmapeditor/test-maps/azuda.wz");
    if !path.exists() {
        eprintln!("azuda.wz fixture missing at {} — skipping", path.display());
        return;
    }
    let map = run_script_map(&path, 0xDEAD_BEEF).expect("azuda.wz should load");
    assert!(map.map_data.width > 0 && map.map_data.height > 0);
    assert_eq!(
        map.map_data.tiles.len(),
        (map.map_data.width as usize) * (map.map_data.height as usize)
    );

    // Re-running with the same seed must produce the same heightmap.
    let again = run_script_map(&path, 0xDEAD_BEEF).expect("rerun");
    assert_eq!(map.map_data.tiles.len(), again.map_data.tiles.len());
    for (a, b) in map.map_data.tiles.iter().zip(again.map_data.tiles.iter()) {
        assert_eq!(a.height, b.height);
        assert_eq!(a.texture, b.texture);
    }
}

#[test]
fn fractal_noise_is_deterministic_for_same_seed() {
    let source = "
        var w = 8, h = 8;
        var noise = generateFractalValueNoise(w, h, 1000, 5, 4);
        var tex = [];
        for (var i = 0; i < w * h; ++i) tex.push(0);
        setMapData(w, h, tex, noise, [], [], []);
    ";
    let a = run_script_source("a", source, 9).expect("a");
    let b = run_script_source("b", source, 9).expect("b");
    for (ta, tb) in a.map_data.tiles.iter().zip(b.map_data.tiles.iter()) {
        assert_eq!(ta.height, tb.height);
    }
}
