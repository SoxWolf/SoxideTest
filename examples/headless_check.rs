//! Headless smoke test for the navmesh demo (no window / GPU required).
//!
//! Validates the whole authored asset stack against the real engine types
//! and exercises the navigation schedule end to end:
//!
//! 1. the `.soxproj` manifest loads and points at the scene;
//! 2. every `.soxaction` / `.soxinputcontext` parses;
//! 3. the `.soxscene` parses into the expected instances, and the agents
//!    carry the right `NavAgent` profiles;
//! 4. applying it to a world produces the expected components and the
//!    gameplay script loads;
//! 5. the engine builds a navmesh from the level's collider geometry, and
//!    pathfinding routes AROUND the wall (never through it), with the
//!    larger profile keeping a wider berth;
//! 6. running the schedule actually walks the agent from one side of the
//!    wall to its goal on the other side — crossing the wall band only
//!    through the gap. All without a panic.
//!
//! Run with: `cargo run --example headless_check`

use soxide_engine::core::glam::DVec3;
use soxide_engine::ecs::Entity;
use soxide_engine::gameplay::{AgentProfile, NavAgent, NavMeshResource, NavObstacle, QueryFilter};
use soxide_engine::input::{InputActionFile, InputContextFile};
use soxide_engine::physics::{BodyKind, CharacterMover, Collider, RigidBody};
use soxide_engine::render::{AmbientLight, Camera3d, DirectionalLight, SceneEntity};
use soxide_engine::script::Scripts;
use soxide_engine::{App, EntityName, Project, Scene};
use std::path::{Path, PathBuf};

fn find(app: &App, name: &str) -> Entity {
    app.world
        .iter_entities()
        .find(|&e| app.world.get::<EntityName>(e).map(|n| n.0 == name).unwrap_or(false))
        .unwrap_or_else(|| panic!("entity {name:?} not found in world"))
}

fn pos(app: &App, e: Entity) -> DVec3 {
    app.world.get::<SceneEntity>(e).unwrap().transform.translation
}

/// Horizontal distance from a point to the wall footprint (the two static
/// boxes that span x in [-10,-2] and [2,10] at z in [-0.5,0.5]). Zero
/// means the point is inside a wall.
fn wall_dist(x: f64, z: f64) -> f64 {
    let to_box = |x0: f64, x1: f64| {
        let dx = (x0 - x).max(0.0).max(x - x1);
        let dz = (-0.5 - z).max(0.0).max(z - 0.5);
        (dx * dx + dz * dz).sqrt()
    };
    to_box(-10.0, -2.0).min(to_box(2.0, 10.0))
}

/// Closest approach of a path to the walls (densely sampled along each
/// segment so a corner-grazing string-pulled route is caught).
fn path_clearance(path: &[DVec3]) -> f64 {
    let mut m = f64::INFINITY;
    for w in path.windows(2) {
        for i in 0..=20 {
            let p = w[0].lerp(w[1], i as f64 / 20.0);
            m = m.min(wall_dist(p.x, p.z));
        }
    }
    m
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

    // 2. Input assets still parse (kept for completeness; the demo itself
    //    is mouse/keyboard-free — the agents are driven by the navmesh).
    let actions = ["MoveForward", "MoveBack", "MoveLeft", "MoveRight", "Jump", "Look"];
    for action in actions {
        let p = contents.join(format!("input/{action}.soxaction"));
        let a = InputActionFile::load(&p).unwrap_or_else(|e| panic!("{action}.soxaction: {e}"));
        assert_eq!(a.name, action);
    }
    InputContextFile::load(contents.join("input/gameplay.soxinputcontext"))
        .expect("gameplay.soxinputcontext parses");
    println!("[ok] input: {} actions + gameplay context", actions.len());

    // 3. Scene parse: the demo instances + nav profiles on the agents.
    let scene = Scene::load(&contents.join("scenes/main.soxscene")).expect("scene parses");
    assert_eq!(scene.instances.len(), 10, "expected 10 entity instances");
    let agent_inst = scene.instances.iter().find(|i| i.overrides.name == "Agent").expect("Agent");
    assert_eq!(
        agent_inst.overrides.nav_agent.as_ref().expect("Agent has a NavAgent").profile,
        "default",
        "default agent routes on the default navmesh",
    );
    let wide_inst = scene.instances.iter().find(|i| i.overrides.name == "WideAgent").expect("WideAgent");
    assert_eq!(
        wide_inst.overrides.nav_agent.as_ref().expect("WideAgent has a NavAgent").profile,
        "wide",
        "wide agent routes on the wide navmesh",
    );
    println!("[ok] scene parsed: {} instances, agents carry nav profiles", scene.instances.len());

    // 4. Assemble + confirm the gameplay script loaded.
    let mut app = App::from_project_file(&proj_path).expect("app builds from project");
    let script_count = app.world.get_resource::<Scripts>().map(|s| s.len()).unwrap_or(0);
    assert!(script_count >= 1, "game.rhai loaded (Scripts::len = {script_count})");
    scene.apply(&mut app.world);

    let agent = find(&app, "Agent");
    let wide = find(&app, "WideAgent");
    assert!(app.world.get::<CharacterMover>(agent).is_some(), "Agent has a CharacterMover body");
    assert_eq!(app.world.get::<NavAgent>(agent).expect("Agent NavAgent").profile, "default");
    assert_eq!(app.world.get::<NavAgent>(wide).expect("WideAgent NavAgent").profile, "wide");
    // The floor collider is the navmesh's walkable input; the walls carve
    // it via NavObstacle.
    assert!(
        matches!(app.world.get::<RigidBody>(find(&app, "Ground")).map(|b| b.kind), Some(BodyKind::Fixed)),
        "floor is static geometry",
    );
    assert!(app.world.get::<Collider>(find(&app, "Ground")).is_some(), "floor has a collider (navmesh input)");
    for wall in ["WallWest", "WallEast"] {
        assert!(app.world.get::<NavObstacle>(find(&app, wall)).is_some(), "{wall} carves the navmesh (NavObstacle)");
    }
    assert!(app.world.get::<Camera3d>(find(&app, "Camera")).is_some(), "camera has Camera3d");
    assert!(app.world.get::<DirectionalLight>(find(&app, "Sun")).is_some(), "sun light");
    assert!(app.world.get::<AmbientLight>(find(&app, "Ambient")).is_some(), "ambient light");
    println!("[ok] stack assembled + game.rhai loaded (agents/walls/floor/camera/lights)");

    // 5. Enable navmesh generation exactly like src/main.rs, then let the
    //    schedule build it. Drop Time so the mover/physics use a fixed
    //    1/60 step and the build is deterministic.
    {
        let nav = app
            .world
            .get_resource_mut::<NavMeshResource>()
            .expect("App::new pre-inserts NavMeshResource");
        nav.profiles = vec![
            AgentProfile { name: "default".into(), radius: 0.5, height: 1.8, max_step: 0.4, max_slope_deg: 50.0 },
            AgentProfile { name: "wide".into(), radius: 1.0, height: 1.8, max_step: 0.4, max_slope_deg: 50.0 },
        ];
        nav.auto_build = true;
    }
    app.world.resources_mut().remove::<soxide_engine::core::Time>();
    for _ in 0..5 {
        app.schedule.run(&mut app.world);
    }
    assert!(
        app.world.get_resource::<NavMeshResource>().unwrap().set.is_some(),
        "navmesh auto-built from the world's collider geometry",
    );


    // 5a. Pathfinding routes AROUND the wall, per profile.
    let (def_len, c_def, c_wide) = {
        let res = app.world.get_resource::<NavMeshResource>().unwrap();
        let filter = QueryFilter::default();
        let p_def = res
            .find_path("default", &filter, DVec3::new(-7.0, 0.0, 6.0), DVec3::new(-7.0, 0.0, -6.0))
            .expect("default agent finds a path");
        let p_wide = res
            .find_path("wide", &filter, DVec3::new(-4.0, 0.0, 6.0), DVec3::new(-4.0, 0.0, -6.0))
            .expect("wide agent finds a path");
        (p_def.len(), path_clearance(&p_def), path_clearance(&p_wide))
    };
    assert!(def_len >= 3, "the route detours around the wall (not a straight line): {def_len} waypoints");
    assert!(c_def > 0.0, "default route never crosses a wall (clearance {c_def:.2} m)");
    assert!(
        c_wide > c_def + 0.2,
        "the wider profile keeps a larger berth from the walls (default {c_def:.2} m vs wide {c_wide:.2} m)",
    );
    println!("[ok] navmesh routes around the wall: {def_len} waypoints, clearance default {c_def:.2} m < wide {c_wide:.2} m");

    // 6. End-to-end: the schedule walks the Agent from z = +6 to its goal
    //    at z = -6, crossing the wall band ONLY through the central gap.
    let start = pos(&app, agent);
    assert!(start.z > 5.0, "agent starts on the near side");
    let mut crossed_in_gap = true;
    for _ in 0..900 {
        app.schedule.run(&mut app.world);
        let p = pos(&app, agent);
        if p.z.abs() <= 0.6 && (p.x < -2.6 || p.x > 2.6) {
            crossed_in_gap = false; // entered the wall band outside the gap
        }
    }
    let end = pos(&app, agent);
    assert!(crossed_in_gap, "agent crossed the wall band only through the gap (ended {end:?})");
    assert!(end.z < -5.0, "agent reached the far side near its goal (z = {:.2})", end.z);
    assert!((end.x + 7.0).abs() < 1.5, "agent ended near its goal x = -7 (x = {:.2})", end.x);
    println!("[ok] agent navigated around the wall: start {start:?} -> goal {end:?}");

    println!("\nALL HEADLESS CHECKS PASSED");
}
