//! Headless smoke test for the sample (no window / GPU required).
//!
//! Validates the whole authored asset stack against the real engine
//! types and exercises the gameplay schedule:
//!
//! 1. the `.soxproj` manifest loads and points at the scene;
//! 2. every `.soxaction` / `.soxinputcontext` parses;
//! 3. the `.soxscene` parses into the expected instances;
//! 4. applying it to a world produces the expected components;
//! 5. running the engine schedule auto-possesses the pawn, settles it
//!    on the floor collider, and drives the follow camera — all without
//!    a single panic.
//!
//! Run with: `cargo run --example headless_check`

use soxide_engine::gameplay::{CameraRig, MoverInputBinding, PlayerController};
use soxide_engine::input::{InputActionFile, InputContextFile};
use soxide_engine::physics::{Collider, CharacterMover};
use soxide_engine::render::{AmbientLight, Camera3d, DirectionalLight};
use soxide_engine::ecs::Entity;
use soxide_engine::{App, EntityName, Project, Scene};
use std::path::{Path, PathBuf};

fn find(app: &App, name: &str) -> Entity {
    app.world
        .iter_entities()
        .find(|&e| {
            app.world
                .get::<EntityName>(e)
                .map(|n| n.0 == name)
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("entity {name:?} not found in world"))
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let proj_path = root.join("sausage_playground.soxproj");

    // 1. Project manifest.
    let project = Project::load(&proj_path).expect("project manifest parses");
    assert_eq!(
        project.default_scene.as_deref(),
        Some(Path::new("scenes/main.soxscene")),
        "default_scene wired",
    );
    let contents = project.contents_root();
    println!("[ok] project '{}' -> {}", project.name, project.default_scene.unwrap().display());

    // 2. Input assets.
    for action in ["Move", "Jump", "Look"] {
        let p = contents.join(format!("input/{action}.soxaction"));
        let a = InputActionFile::load(&p).unwrap_or_else(|e| panic!("{action}.soxaction: {e}"));
        assert_eq!(a.name, action);
        println!("[ok] action {action} ({:?})", a.value_kind);
    }
    let ctx = InputContextFile::load(contents.join("input/gameplay.soxinputcontext"))
        .expect("gameplay.soxinputcontext parses");
    assert_eq!(ctx.mappings.len(), 7, "WASD + Jump + 2 Look mappings");
    println!("[ok] context gameplay ({} mappings)", ctx.mappings.len());

    // 3. Scene parse.
    let scene_path = contents.join("scenes/main.soxscene");
    let scene = Scene::load(&scene_path).expect("scene parses");
    assert_eq!(scene.instances.len(), 10, "expected 10 entity instances");
    let coin_instances = scene
        .instances
        .iter()
        .filter(|i| i.overrides.tags.iter().any(|t| t == "Coin"))
        .count();
    assert_eq!(coin_instances, 3, "three collectible coins");
    let player_inst = scene
        .instances
        .iter()
        .find(|i| i.overrides.name == "Player")
        .expect("Player instance present");
    assert_eq!(
        player_inst
            .overrides
            .mesh3d
            .as_ref()
            .and_then(|m| m.mesh_path.as_deref()),
        Some("meshes/sausage.fbx"),
        "player uses the sausage FBX",
    );
    assert!(
        contents.join("meshes/sausage.fbx").is_file(),
        "sausage FBX vendored into contents",
    );
    println!("[ok] scene parsed: {} instances", scene.instances.len());

    // 4. Assemble into a real App world and check component wiring.
    let mut app = App::from_project_file(&proj_path).expect("app builds from project");
    scene.apply(&mut app.world);

    let player = find(&app, "Player");
    assert!(app.world.get::<CharacterMover>(player).is_some(), "Player has CharacterMover");
    assert!(app.world.get::<MoverInputBinding>(player).is_some(), "Player has MoverInputBinding");

    let controller = find(&app, "PlayerController");
    assert!(app.world.get::<PlayerController>(controller).is_some(), "controller present");

    let camera = find(&app, "Camera");
    assert!(app.world.get::<Camera3d>(camera).is_some(), "camera has Camera3d");
    assert!(app.world.get::<CameraRig>(camera).is_some(), "camera has CameraRig");

    for terrain in ["Ground", "Ramp"] {
        let e = find(&app, terrain);
        assert!(app.world.get::<Collider>(e).is_some(), "{terrain} has a collider");
    }
    assert!(app.world.get::<DirectionalLight>(find(&app, "Sun")).is_some(), "sun light");
    assert!(app.world.get::<AmbientLight>(find(&app, "Ambient")).is_some(), "ambient light");

    let coins_alive = |app: &App| {
        app.world
            .iter_entities()
            .filter(|&e| {
                app.world
                    .get::<EntityName>(e)
                    .map(|n| n.0.starts_with("Coin"))
                    .unwrap_or(false)
            })
            .count()
    };
    assert_eq!(coins_alive(&app), 3, "three coins spawned");
    println!("[ok] component stack assembled (pawn / controller / camera / terrain / lights / 3 coins)");

    // 5. Run the schedule: auto-possess + physics + camera rig.
    // Without the platform runner advancing `Time` each frame its delta
    // stays 0; drop it so the mover / physics fall back to a fixed 1/60
    // step (the same trick the engine's own integration tests use).
    app.world.resources_mut().remove::<soxide_engine::core::Time>();
    let cam_start = app.world.get::<soxide_engine::render::SceneEntity>(camera).unwrap().transform.translation;
    for _ in 0..240 {
        app.schedule.run(&mut app.world);
    }

    let mover = app.world.get::<CharacterMover>(player).expect("mover survives ticks");
    let player_y = app
        .world
        .get::<soxide_engine::render::SceneEntity>(player)
        .unwrap()
        .transform
        .translation
        .y;
    assert!(mover.state.grounded, "player settled on the floor collider (y = {player_y})");
    assert!(player_y > 0.0, "player did not tunnel through the floor (y = {player_y})");
    println!("[ok] 240 ticks: player grounded at y = {player_y:.3}, mode = {}", mover.mode);

    let cam_end = app.world.get::<soxide_engine::render::SceneEntity>(camera).unwrap().transform.translation;
    assert!(
        (cam_end - cam_start).length() > 0.5,
        "camera rig moved to follow the possessed pawn ({cam_start:?} -> {cam_end:?})",
    );
    println!("[ok] camera rig followed possession: {cam_start:?} -> {cam_end:?}");

    // The gameplay script (game.rhai) ran each tick without panicking;
    // the player spawns far from every coin, so none were collected.
    assert_eq!(coins_alive(&app), 3, "coins survive when the player is out of range");
    println!("[ok] game.rhai ran over {} ticks (coins intact)", 240);

    println!("\nALL HEADLESS CHECKS PASSED");
}
