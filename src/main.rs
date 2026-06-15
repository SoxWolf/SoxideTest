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
    let app = match App::from_project_file(&proj) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("failed to load project {}: {e}", proj.display());
            std::process::exit(1);
        }
    };

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
