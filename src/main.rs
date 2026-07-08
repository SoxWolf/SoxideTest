//! Voxel World — a Minecraft-style Soxide game.
//!
//! An infinite, procedurally-generated voxel world you explore in first
//! person: gravity-walking player (WASD + mouse, Space to jump), left-click
//! to carve a block, right-click to place one, F5 to save. Chunks stream in
//! and out around you (greedy-meshed, textured, with a water plane), a live
//! creature population follows you, and edits + player position persist to a
//! save file.
//!
//! Everything is built in code (see `lib.rs`); the binary just hands the
//! assembled `App` to the platform runner, which starts the simulation
//! un-paused.

fn main() {
    let app = sausage_playground::build_streaming_app();

    #[cfg(target_os = "linux")]
    use soxide_platform_linux as platform;
    #[cfg(target_os = "windows")]
    use soxide_platform_windows as platform;

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Err(e) = platform::run(app) {
        eprintln!("voxel world exited with error: {e}");
        std::process::exit(1);
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = app;
        eprintln!("no desktop platform for this target");
    }
}
