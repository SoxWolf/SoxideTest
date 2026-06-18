// Soxide Navmesh Demo — runtime goal switching.
//
// The heavy lifting (navmesh generation from the level colliders, path
// finding, and steering the movers) is done by the engine's nav systems,
// which run in `Update` BEFORE scripts. This script only RE-TARGETS the
// agents: every few seconds it flips each agent's goal to the other side
// of the wall, so they continuously re-path around it. Because
// `nav_goal_pos` writes `NavAgent.goal` (not the mover intent), it never
// fights the path follower.
//
// Engine quirks this file works around (see the engine docs):
//   * Scripts::load_dir is non-recursive and rooted at `contents/`, so
//     this file must live in the contents ROOT.
//   * call_fn does not run top-level statements, so tunables live as
//     locals inside the fn bodies.
//   * In headless runs `time_elapsed()` is frozen at 0, so the agents
//     keep their scene-authored goals (z = -6) — deterministic for tests.

fn start() {
    log("Navmesh demo ready - agents route around the wall through the gap");
}

fn update(dt) {
    // Goal X per agent (matches each agent's start X, behind a wall
    // segment) and the Z it ping-pongs between.
    let agent_x = -7.0;
    let wide_x = -4.0;
    let far_z = -6.0;   // across the wall
    let near_z = 6.0;   // back to the start side
    let period = 12.0;  // full there-and-back cycle, seconds

    // First half of the cycle: head across; second half: head back.
    let go_far = (time_elapsed() % period) < (period * 0.5);
    let z = if go_far { far_z } else { near_z };

    let agent = find_by_name("Agent");
    if agent != 0 && is_alive(agent) {
        nav_goal_pos(agent, agent_x, 0.0, z);
    }
    let wide = find_by_name("WideAgent");
    if wide != 0 && is_alive(wide) {
        nav_goal_pos(wide, wide_x, 0.0, z);
    }

    hud_text(20.0, 20.0, 22.0, "Navmesh demo: agents path around the wall via the gap");
    hud_text(20.0, 48.0, 16.0, "blue = nav radius 0.5    orange = nav radius 1.0 (wider berth)");
}
