//! Sausage Playground — a minimal third-person Soxide sample.
//!
//! `main` loads the sibling `.soxproj` manifest and hands the resulting
//! `App` to the platform runner. Everything that makes up the game — the
//! terrain + ramp colliders, the skinned character, its `CharacterMover`
//! / `MoverInputBinding` / `PlayerController`, the follow `CameraRig`,
//! the lights, and the input assets — is authored as plain-text assets
//! under `contents/` and loaded by the engine:
//!
//! - the project's `default_scene` (`contents/scenes/main.soxscene`) is
//!   auto-loaded by the runner on startup;
//! - the `.soxaction` / `.soxinputcontext` files under `contents/input/`
//!   are registered by `App::from_project_file`;
//! - `contents/scripts/input_setup.rhai` activates the `gameplay`
//!   input mapping context once at startup.
//!
//! So this binary is deliberately tiny: the engine does the rest.

use soxide_engine::App;
use std::path::PathBuf;

fn main() {
    let proj = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sausage_playground.soxproj");
    let mut app = match App::from_project_file(&proj) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("failed to load project {}: {e}", proj.display());
            std::process::exit(1);
        }
    };

    // Turn on navmesh generation. `App::new` pre-inserts an (empty,
    // auto-build off) `NavMeshResource`; here we declare the agent
    // profiles to bake a navmesh for and flip `auto_build` so
    // `nav_maintenance_tick` (re)generates the mesh from the level's
    // collision geometry whenever it is missing or dirty. Two profiles
    // give two per-agent navmeshes: "default" matches the demo agent's
    // body, "wide" keeps a larger berth from the walls.
    if let Some(nav) = app
        .world
        .get_resource_mut::<soxide_engine::gameplay::NavMeshResource>()
    {
        use soxide_engine::gameplay::AgentProfile;
        nav.profiles = vec![
            AgentProfile {
                name: "default".into(),
                radius: 0.5,
                height: 1.8,
                max_step: 0.4,
                max_slope_deg: 50.0,
            },
            AgentProfile {
                name: "wide".into(),
                radius: 1.0,
                height: 1.8,
                max_step: 0.4,
                max_slope_deg: 50.0,
            },
        ];
        nav.auto_build = true;
    }

    #[cfg(target_os = "linux")]
    use soxide_platform_linux as platform;
    #[cfg(target_os = "windows")]
    use soxide_platform_windows as platform;

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Err(e) = platform::run(app) {
        eprintln!("sausage_playground exited with error: {e}");
        std::process::exit(1);
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = app;
        eprintln!("sausage_playground: no platform crate compiled for this OS");
        std::process::exit(2);
    }
}
