// Sausage Playground — gameplay logic.
//
// `start()` runs once; `update(dt)` runs every frame. The engine's
// built-in systems already do movement, possession, physics and the
// follow camera — this script adds the thin game layer on top:
//
//   - activates the "gameplay" input mapping context (loaded inactive);
//   - collects coins by proximity (coins are tagged "Coin");
//   - respawns the player if it falls off the world;
//   - draws a small HUD.

const SPAWN_X = 0.0;
const SPAWN_Y = 1.0;
const SPAWN_Z = 4.0;
const FALL_Y = -8.0;            // below this, respawn
const PICKUP = 1.2;            // pickup radius (metres)
const TOTAL_COINS = 3;

fn start() {
    // Contexts load registered-but-inactive; make ours live.
    add_input_context("gameplay", 0);
    log("Sausage Playground ready - WASD/Space/Mouse");
}

fn update(dt) {
    let player = find_by_name("Player");
    if !is_alive(player) { return; }

    let p = get_translation(player);
    if p.is_empty() { return; }

    // Respawn on fall.
    if p[1] < FALL_Y {
        set_translation(player, SPAWN_X, SPAWN_Y, SPAWN_Z);
    }

    // Distance-based coin pickup.
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

    // HUD.
    let collected = TOTAL_COINS - entities_with_tag("Coin").len();
    hud_text(20.0, 20.0, 24.0, `Coins: ${collected} / ${TOTAL_COINS}`);

    let v = mover_velocity(player);
    if !v.is_empty() {
        let speed = (v[0] * v[0] + v[2] * v[2]).sqrt();
        hud_text(20.0, 50.0, 18.0, `Speed: ${speed.to_int()} m/s  (${mover_mode(player)})`);
    }
    hud_text(20.0, 74.0, 16.0, "WASD move - Space jump - Mouse look");
}
