//! Headless smoke test for the sample (no window / GPU required).
//!
//! Validates the whole authored asset stack against the real engine
//! types and exercises the gameplay schedule end to end:
//!
//! 1. the `.soxproj` manifest loads and points at the scene;
//! 2. every `.soxaction` / `.soxinputcontext` parses;
//! 3. the `.soxscene` parses into the expected instances;
//! 4. applying it to a world produces the expected components and the
//!    gameplay script actually loads;
//! 5. running the schedule auto-possesses the pawn, settles it on the
//!    floor, drives the follow camera, and — when input is fed — the
//!    script moves the character. All without a panic.
//!
//! Run with: `cargo run --example headless_check`

use soxide_engine::ecs::Entity;
use soxide_engine::gameplay::{AiController, CameraRig, PlayerController};
use soxide_engine::input::{InputActionFile, InputContextFile};
use soxide_engine::physics::{BodyKind, CharacterMover, Collider, RigidBody};
use soxide_engine::render::{AmbientLight, Camera3d, DirectionalLight, SceneEntity};
use soxide_engine::script::Scripts;
use soxide_engine::window::input::ButtonState;
use soxide_engine::window::{Input, KeyCode};
use soxide_engine::{App, EntityName, Project, Scene};
use std::path::{Path, PathBuf};

fn find(app: &App, name: &str) -> Entity {
    app.world
        .iter_entities()
        .find(|&e| app.world.get::<EntityName>(e).map(|n| n.0 == name).unwrap_or(false))
        .unwrap_or_else(|| panic!("entity {name:?} not found in world"))
}

fn player_pos(app: &App, e: Entity) -> soxide_engine::core::glam::DVec3 {
    app.world.get::<SceneEntity>(e).unwrap().transform.translation
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
    let actions = ["MoveForward", "MoveBack", "MoveLeft", "MoveRight", "Jump", "Look"];
    for action in actions {
        let p = contents.join(format!("input/{action}.soxaction"));
        let a = InputActionFile::load(&p).unwrap_or_else(|e| panic!("{action}.soxaction: {e}"));
        assert_eq!(a.name, action);
    }
    let ctx = InputContextFile::load(contents.join("input/gameplay.soxinputcontext"))
        .expect("gameplay.soxinputcontext parses");
    assert_eq!(ctx.mappings.len(), 6, "WASD + Jump + Look(yaw)");
    println!("[ok] input: {} actions + gameplay context ({} mappings)", actions.len(), ctx.mappings.len());

    // 3. Scene parse.
    let scene = Scene::load(&contents.join("scenes/main.soxscene")).expect("scene parses");
    assert_eq!(scene.instances.len(), 12, "expected 12 entity instances");
    let coins = scene.instances.iter().filter(|i| i.overrides.tags.iter().any(|t| t == "Coin")).count();
    assert_eq!(coins, 3, "three collectible coins");
    // The sausage mesh lives on the Player's child entity (scaled down +
    // offset so its feet sit on the ground; see the scene comment).
    let player_inst = scene.instances.iter().find(|i| i.overrides.name == "Player").expect("Player");
    let mesh_child = player_inst
        .overrides
        .children
        .iter()
        .find(|c| c.mesh3d.as_ref().and_then(|m| m.mesh_path.as_deref()) == Some("meshes/sausage.fbx"))
        .expect("Player has a child carrying the sausage FBX");
    let mesh_scale = mesh_child.scene_entity.unwrap().transform.scale.x;
    assert!(mesh_scale < 0.5, "sausage scaled down to fit the scene (scale = {mesh_scale})");
    assert!(contents.join("meshes/sausage.fbx").is_file(), "sausage FBX vendored");
    println!("[ok] scene parsed: {} instances", scene.instances.len());

    // 4. Assemble + confirm the gameplay script loaded (it lives in the
    //    contents root because Scripts::load_dir is non-recursive).
    let mut app = App::from_project_file(&proj_path).expect("app builds from project");
    let script_count = app.world.get_resource::<Scripts>().map(|s| s.len()).unwrap_or(0);
    assert!(script_count >= 1, "game.rhai loaded (Scripts::len = {script_count})");
    scene.apply(&mut app.world);

    let player = find(&app, "Player");
    assert!(app.world.get::<CharacterMover>(player).is_some(), "Player has CharacterMover");
    let controller = find(&app, "PlayerController");
    assert!(app.world.get::<PlayerController>(controller).is_some(), "controller present");
    let camera = find(&app, "Camera");
    assert!(app.world.get::<Camera3d>(camera).is_some(), "camera has Camera3d");
    assert!(app.world.get::<CameraRig>(camera).is_some(), "camera has CameraRig");
    for terrain in ["Ground", "Ramp"] {
        assert!(app.world.get::<Collider>(find(&app, terrain)).is_some(), "{terrain} collider");
    }
    assert!(app.world.get::<DirectionalLight>(find(&app, "Sun")).is_some(), "sun light");
    assert!(app.world.get::<AmbientLight>(find(&app, "Ambient")).is_some(), "ambient light");

    let coins_alive = |app: &App| {
        app.world
            .iter_entities()
            .filter(|&e| app.world.get::<EntityName>(e).map(|n| n.0.starts_with("Coin")).unwrap_or(false))
            .count()
    };
    assert_eq!(coins_alive(&app), 3, "three coins spawned");

    let platform = find(&app, "Platform");
    assert!(app.world.get::<Collider>(platform).is_some(), "platform collider");
    assert!(
        matches!(app.world.get::<RigidBody>(platform).map(|b| b.kind), Some(BodyKind::Kinematic)),
        "platform is a kinematic body",
    );
    let enemy = find(&app, "Enemy");
    assert!(app.world.get::<AiController>(enemy).is_some(), "enemy AiController");
    assert!(app.world.get::<CharacterMover>(enemy).is_some(), "enemy CharacterMover");
    println!("[ok] stack assembled + game.rhai loaded (pawn/controller/camera/terrain/lights/coins/platform/enemy)");

    // The enemy must be NON-LETHAL: on contact it gets sent home, and the
    // player is NOT reset to spawn (that was the reset loop). Put the
    // player away from spawn, sit the enemy on top of it, tick once, and
    // confirm the player stayed put while the enemy went home.
    {
        use soxide_engine::core::glam::DVec3;
        app.world.get_mut::<SceneEntity>(player).unwrap().transform.translation = DVec3::new(3.0, 1.0, 4.0);
        app.world.get_mut::<SceneEntity>(enemy).unwrap().transform.translation = DVec3::new(3.0, 1.0, 4.4);
        app.schedule.run(&mut app.world);
        let pp = player_pos(&app, player);
        assert!(
            (pp.x - 3.0).abs() < 1.0 && pp.z > 2.0,
            "enemy contact must NOT reset the player to spawn (player now at {pp:?})",
        );
        assert!(player_pos(&app, enemy).z < -5.0, "enemy sent home on contact");
        // Put the player back at spawn so the later phases are unaffected.
        app.world.get_mut::<SceneEntity>(player).unwrap().transform.translation = DVec3::new(0.0, 1.0, 4.0);
        println!("[ok] enemy is non-lethal: contact sends it home, player not reset");
    }

    // Despawn the enemy so the physics/ramp assertions below are
    // deterministic (it would otherwise wander into them).
    app.world.despawn(enemy);

    // 5a. Settle: no input. Drop Time so mover/physics use a fixed 1/60
    //     step (Time is normally advanced by the platform runner).
    app.world.resources_mut().remove::<soxide_engine::core::Time>();
    let cam_start = player_pos(&app, camera);
    for _ in 0..240 {
        app.schedule.run(&mut app.world);
    }
    let mover = app.world.get::<CharacterMover>(player).expect("mover survives ticks");
    let py = player_pos(&app, player).y;
    assert!(mover.state.grounded, "player settled on the floor (y = {py})");
    assert!(py > 0.0, "player did not tunnel through the floor (y = {py})");
    println!("[ok] 240 ticks: player grounded at y = {py:.3}, mode = {}", mover.mode);

    let cam_end = player_pos(&app, camera);
    assert!((cam_end - cam_start).length() > 0.5, "camera rig followed the pawn");
    println!("[ok] camera rig followed possession: {cam_start:?} -> {cam_end:?}");
    assert_eq!(coins_alive(&app), 3, "coins intact while the player is out of range");

    // 5b. THE REGRESSION TEST for "the character doesn't move": hold W
    //     (MoveForward). The gameplay script must read the input action
    //     and drive the mover forward (-Z). This is exactly the path
    //     that was broken (script not loaded + Swizzle modifier dead).
    let z_before = player_pos(&app, player).z;
    for _ in 0..45 {
        if let Some(input) = app.world.get_resource_mut::<Input>() {
            input.feed_key(KeyCode::W, ButtonState::Pressed);
        }
        app.schedule.run(&mut app.world);
    }
    let after = player_pos(&app, player);
    assert!(
        after.z < z_before - 0.5,
        "holding W must move the player forward (-Z): z {z_before:.2} -> {:.2}",
        after.z,
    );
    println!("[ok] input drives movement: holding W moved player z {z_before:.2} -> {:.2}", after.z);

    // 5c. Exercise update() PAST the early-return region: teleport the
    //     player onto a coin and confirm the script collects it. This
    //     guards against script runtime errors in update() (e.g. the
    //     top-level-const bug that aborted update before this code ran).
    let coin = find(&app, "Coin1");
    let coin_pos = player_pos(&app, coin);
    app.world.get_mut::<SceneEntity>(player).unwrap().transform.translation = coin_pos;
    let before = coins_alive(&app);
    for _ in 0..3 {
        app.schedule.run(&mut app.world);
    }
    assert!(
        coins_alive(&app) < before,
        "game.rhai update() must run past its early sections and collect the coin \
         (a script error would leave it: {before} coins before, {} after)",
        coins_alive(&app),
    );
    println!("[ok] script update() runs fully: coin collected on contact ({before} -> {})", coins_alive(&app));

    // 5d. Ramp climb: drop the player just in front of the ramp and hold
    //     W; the mover must walk it UP (y rises), proving the reshaped
    //     ~14° slope is actually walkable.
    if let Some(input) = app.world.get_resource_mut::<Input>() {
        input.feed_key(KeyCode::W, ButtonState::Released);
    }
    app.world.get_mut::<SceneEntity>(player).unwrap().transform.translation =
        soxide_engine::core::glam::DVec3::new(0.0, 1.0, 0.8);
    for _ in 0..15 {
        app.schedule.run(&mut app.world); // settle at the ramp foot
    }
    let y_foot = player_pos(&app, player).y;
    for _ in 0..90 {
        if let Some(input) = app.world.get_resource_mut::<Input>() {
            input.feed_key(KeyCode::W, ButtonState::Pressed);
        }
        app.schedule.run(&mut app.world);
    }
    let top = player_pos(&app, player);
    assert!(
        top.y > y_foot + 0.4,
        "player must climb the ramp (y {y_foot:.2} -> {:.2} at z {:.2})",
        top.y,
        top.z,
    );
    println!("[ok] ramp is walkable: player climbed y {y_foot:.2} -> {:.2} (z {:.2})", top.y, top.z);

    println!("\nALL HEADLESS CHECKS PASSED");
}
