// Activate the "gameplay" input mapping context once at startup.
//
// `App::from_project_file` registers every `.soxinputcontext` under
// `contents/` by file stem, but leaves them inactive — a game decides
// which contexts are live. This one-line script makes "gameplay" live
// (priority 0) so WASD / Space / mouse-look reach the player.

fn start() {
    add_input_context("gameplay", 0);
}

fn update(dt) {
    // Nothing per-frame: the engine's built-in systems
    // (mover_input_tick, mover_tick, camera_rig_tick) drive the rest.
}
