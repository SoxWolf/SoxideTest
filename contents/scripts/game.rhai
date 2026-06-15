// Sausage Playground — gameplay logic.
//
// `start()` runs once; `update(dt)` runs every frame. The engine's
// built-in systems already do movement, possession, physics and the
// follow camera — this script adds the thin game layer on top:
//
//   - activates the "gameplay" input mapping context (loaded inactive);
//   - collects coins by proximity (coins are tagged "Coin");
//   - respawns the player if it falls off the world or is caught;
//   - steers the enemy chaser toward the player;
//   - sweeps the moving platform side to side;
//   - draws a small HUD.

const SPAWN_X = 0.0;
const SPAWN_Y = 1.0;
const SPAWN_Z = 4.0;
const FALL_Y = -8.0;            // below this, respawn
const PICKUP = 1.2;            // coin pickup radius (metres)
const CATCH = 1.1;            // enemy catch radius (metres)
const TOTAL_COINS = 3;

fn respawn(player) {
    set_translation(player, SPAWN_X, SPAWN_Y, SPAWN_Z);
}

fn update(dt) {
    let player = find_by_name("Player");
    if !is_alive(player) { return; }

    let p = get_translation(player);
    if p.is_empty() { return; }

    // Respawn on fall.
    if p[1] < FALL_Y { respawn(player); }

    // Coin pickup (distance-based; coins are tagged "Coin").
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

    // Enemy AI: steer the chaser toward the player; catching respawns.
    let enemy = find_by_name("Enemy");
    if is_alive(enemy) {
        let e = get_translation(enemy);
        if !e.is_empty() {
            let dx = p[0] - e[0];
            let dz = p[2] - e[2];
            let d = (dx * dx + dz * dz).sqrt();
            if d > 0.001 {
                mover_input(enemy, dx / d, dz / d);
            }
            if d < CATCH { respawn(player); }
        }
    }

    // HUD.
    let collected = TOTAL_COINS - entities_with_tag("Coin").len();
    hud_text(20.0, 20.0, 24.0, `Coins: ${collected} / ${TOTAL_COINS}`);
    let v = mover_velocity(player);
    if !v.is_empty() {
        let speed = (v[0] * v[0] + v[2] * v[2]).sqrt();
        hud_text(20.0, 50.0, 18.0, `Speed: ${speed.to_int()} m/s  (${mover_mode(player)})`);
    }
    hud_text(20.0, 74.0, 16.0, "WASD move - Space jump - Mouse look - avoid the red chaser");

    // Sweep the moving platform (set last so a math hiccup can't block
    // the gameplay above). Stays put in headless runs where Time is
    // frozen (time_elapsed() == 0).
    let plat = find_by_name("Platform");
    if is_alive(plat) {
        set_translation(plat, sin(time_elapsed()) * 3.0, 1.0, -1.0);
    }
}

fn start() {
    add_input_context("gameplay", 0);
    log("Sausage Playground ready - WASD/Space/Mouse - collect coins, dodge the chaser");
}
