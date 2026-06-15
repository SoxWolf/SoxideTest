// Sausage Playground — gameplay logic.
//
// IMPORTANT: this file lives in the contents ROOT, not a subfolder —
// the engine's script loader (`Scripts::load_dir`) is non-recursive and
// is pointed at `contents/`, so a script in `contents/scripts/` would
// never load.
//
// `start()` runs once; `update(dt)` runs every frame. The engine's
// built-in systems handle the mover step, possession, physics and the
// follow camera; this script adds the game layer:
//
//   - activates the "gameplay" input mapping context (loaded inactive);
//   - drives the player from the four directional Bool actions
//     (composed here because the engine's on-disk Swizzle modifier,
//     which an Axis2D WASD map would need, doesn't round-trip);
//   - collects coins by proximity;
//   - steers the enemy chaser and respawns the player if caught or fallen;
//   - sweeps the moving platform; draws a HUD.

const SPAWN_X = 0.0;
const SPAWN_Y = 1.0;
const SPAWN_Z = 4.0;
const FALL_Y = -8.0;
const PICKUP = 1.2;
const CATCH = 1.1;
const TOTAL_COINS = 3;

fn respawn(player) {
    set_translation(player, SPAWN_X, SPAWN_Y, SPAWN_Z);
}

fn start() {
    add_input_context("gameplay", 0);
    log("Sausage Playground ready - WASD move, Space jump, mouse to look");
}

fn update(dt) {
    let player = find_by_name("Player");
    if player == 0 || !is_alive(player) { return; }

    let p = get_translation(player);
    if p.is_empty() { return; }

    // --- Movement: compose a world-space direction from the four
    // directional actions and feed it to the mover. Forward is -Z (the
    // camera starts behind the player looking that way).
    let mx = 0.0;
    let mz = 0.0;
    if action_bool("MoveForward") { mz -= 1.0; }
    if action_bool("MoveBack")    { mz += 1.0; }
    if action_bool("MoveRight")   { mx += 1.0; }
    if action_bool("MoveLeft")    { mx -= 1.0; }
    let len = (mx * mx + mz * mz).sqrt();
    if len > 0.001 { mx /= len; mz /= len; }
    mover_input(player, mx, mz);
    if action_triggered("Jump") { mover_jump(player); }

    // --- Respawn on fall.
    if p[1] < FALL_Y { respawn(player); }

    // --- Coin pickup (distance-based; coins are tagged "Coin").
    let coins = entities_with_tag("Coin");
    for c in coins {
        let cp = get_translation(c);
        if cp.is_empty() { continue; }
        let dx = cp[0] - p[0];
        let dy = cp[1] - p[1];
        let dz = cp[2] - p[2];
        if dx * dx + dy * dy + dz * dz < PICKUP * PICKUP {
            despawn(c);
        }
    }

    // --- Enemy AI: steer the chaser toward the player; catching respawns.
    let enemy = find_by_name("Enemy");
    if enemy != 0 && is_alive(enemy) {
        let e = get_translation(enemy);
        if !e.is_empty() {
            let dx = p[0] - e[0];
            let dz = p[2] - e[2];
            let d = (dx * dx + dz * dz).sqrt();
            if d > 0.001 { mover_input(enemy, dx / d, dz / d); }
            if d < CATCH { respawn(player); }
        }
    }

    // --- HUD.
    let collected = TOTAL_COINS - entities_with_tag("Coin").len();
    hud_text(20.0, 20.0, 24.0, `Coins: ${collected} / ${TOTAL_COINS}`);
    let v = mover_velocity(player);
    if !v.is_empty() {
        let speed = (v[0] * v[0] + v[2] * v[2]).sqrt();
        hud_text(20.0, 50.0, 18.0, `Speed: ${speed.to_int()} m/s  (${mover_mode(player)})`);
    }
    hud_text(20.0, 74.0, 16.0, "WASD move - Space jump - mouse look - dodge the red chaser");

    // --- Sweep the moving platform (last, so a math hiccup can't block
    // the gameplay above). Frozen in headless runs (time_elapsed() == 0).
    let plat = find_by_name("Platform");
    if plat != 0 && is_alive(plat) {
        set_translation(plat, sin(time_elapsed()) * 3.0, 1.0, -1.0);
    }
}
