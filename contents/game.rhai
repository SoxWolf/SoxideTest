// Sausage Playground — gameplay logic.
//
// IMPORTANT engine quirks this file works around:
//
//  * The script loader (`Scripts::load_dir`) is non-recursive and points
//    at `contents/`, so this script must live in the contents ROOT.
//  * Script functions are invoked via `call_fn`, which does NOT execute
//    top-level statements — so top-level `const` / `let` are invisible
//    inside `fn` bodies ("Variable not found"). Tunables therefore live
//    as locals inside `update()`.
//
// `start()` runs once; `update(dt)` runs every frame. The engine's
// built-in systems handle the mover step, possession, physics and the
// follow camera; this script adds the game layer.

fn start() {
    add_input_context("gameplay", 0);
    log("Sausage Playground ready - WASD move, Space jump, mouse to look");
}

fn update(dt) {
    // Tunables (locals — see the note above re: top-level const).
    let spawn_x = 0.0;
    let spawn_y = 1.0;
    let spawn_z = 4.0;
    let fall_y = -8.0;
    let pickup2 = 1.2 * 1.2;   // squared pickup radius
    let catch2 = 1.1 * 1.1;    // squared enemy-catch radius
    let total_coins = 3;
    // Enemy home (matches the scene). On a catch we send the enemy back
    // here too, so it can't sit on the spawn point and re-catch instantly.
    let enemy_x = 9.0;
    let enemy_y = 1.0;
    let enemy_z = -9.0;

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
    if p[1] < fall_y { set_translation(player, spawn_x, spawn_y, spawn_z); }

    // --- Coin pickup (distance-based; coins are tagged "Coin").
    for c in entities_with_tag("Coin") {
        let cp = get_translation(c);
        if cp.is_empty() { continue; }
        let dx = cp[0] - p[0];
        let dy = cp[1] - p[1];
        let dz = cp[2] - p[2];
        if dx * dx + dy * dy + dz * dz < pickup2 {
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
            let d2 = dx * dx + dz * dz;
            let d = d2.sqrt();
            if d > 0.001 { mover_input(enemy, dx / d, dz / d); }
            // Non-lethal: on contact the enemy is just sent home. The
            // player is NEVER reset by the enemy (that caused a reset
            // loop). The only respawn is falling off the world, above.
            if d2 < catch2 { set_translation(enemy, enemy_x, enemy_y, enemy_z); }
        }
    }

    // --- HUD.
    let collected = total_coins - entities_with_tag("Coin").len();
    hud_text(20.0, 20.0, 24.0, `Coins: ${collected} / ${total_coins}`);
    let v = mover_velocity(player);
    if !v.is_empty() {
        let speed = (v[0] * v[0] + v[2] * v[2]).sqrt();
        hud_text(20.0, 50.0, 18.0, `Speed: ${speed.to_int()} m/s  (${mover_mode(player)})`);
    }
    hud_text(20.0, 74.0, 16.0, "WASD move - Space jump - mouse look - dodge the red chaser");

    // --- Sweep the moving platform (last, so a hiccup can't block the
    // gameplay above). Frozen in headless runs (time_elapsed() == 0).
    let plat = find_by_name("Platform");
    if plat != 0 && is_alive(plat) {
        // Slide along Z at x = 6 (off the spawn->ramp path).
        set_translation(plat, 6.0, 0.6, sin(time_elapsed()) * 3.0);
    }
}
