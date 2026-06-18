# Soxide Navmesh Demo

A minimal **navmesh navigation** sample for the
[Soxide](https://github.com/SoxWolf/Soxide) game engine. A character walks
to a destination on the far side of a wall by routing *around* it through
a gap, driven entirely by the engine's navigation subsystem
(`soxide_engine::gameplay::nav`). It is a standalone project that depends
on the engine over **git** (the engine lives in a separate repository —
game and engine trees stay physically disjoint, a hard Soxide invariant)
and assembles the whole scene from plain-text, diffable assets.

> **Engine branch.** The navmesh types (`NavAgent`, `NavObstacle`,
> `NavMeshResource`, `AgentProfile`, …) live on the engine branch
> `claude/code-review-optimization-3qaemk`. Until it merges to `main`,
> `Cargo.toml` pins the engine to that branch's HEAD commit (`6891f4d`).

## How the navmesh works here

The engine wires navigation into the `Update` schedule automatically
(`App::new` pre-inserts `NavMeshResource` and registers
`nav_maintenance_tick` + `nav_agent_tick`). This project supplies three
things:

1. **A walkable surface from level geometry.** The floor is a static
   `Collider` cuboid; `nav_maintenance_tick` triangulates every collider
   (`collect_nav_triangles`) and bakes a navmesh over it.
2. **Obstacles to route around.** Each wall segment is a **`NavObstacle`**
   whose footprint is carved out of every agent's navmesh, leaving the
   central gap as the only crossing.
3. **Agents.** Each character has a `CharacterMover` (its body) and a
   `NavAgent` (the path follower): it resolves its goal, queries the
   navmesh for its `profile`, and writes `MoveIntent` on the mover, so the
   existing kinematic mover walks the funnelled corridor.

`src/main.rs` enables the build: it declares the agent **profiles** and
flips `auto_build` on `NavMeshResource` so the mesh regenerates from world
geometry whenever it is missing or dirty.

> **Why the walls are obstacles, not colliders.** This navmesh build is
> single-floor — it keeps the *highest walkable surface per cell*. A solid
> box collider's flat **top** is itself walkable, so a collider wall would
> just raise the floor locally and would **not** block horizontal routing.
> Carving with `NavObstacle` is the engine's blocking primitive (its own
> navmesh verification scene blocks a wall the same way). The floor stays a
> real collider, so the *walkable* mesh is still generated from level
> geometry — obstacles only subtract from it.

## What's in the scene

`contents/scenes/main.soxscene` (auto-loaded on startup as the project's
`default_scene`):

| Entity | Role |
|---|---|
| **Ground** | 20 × 20 static `Collider` box — the walkable surface the navmesh is baked over |
| **WallWest** / **WallEast** | wall segments (`NavObstacle` + mesh) spanning `x ∈ [-10,-2]` and `[2,10]` at `z = 0`, leaving a 4 m gap at `x ∈ [-2,2]` |
| **Agent** | `CharacterMover` + `NavAgent` (profile `"default"`, radius 0.5). Starts behind the west wall at `z = +6`, goal at `z = -6` — must detour through the gap. Blue body. |
| **WideAgent** | same trip with profile `"wide"` (radius 1.0). Its per-agent navmesh erodes further from the walls, so it keeps a **wider berth** at the gap. Orange body. |
| **GoalAgent** / **GoalWideAgent** | small markers at each agent's goal |
| **Camera** | a static, elevated `Camera3d` framing the whole arena from above |
| **Sun** / **Ambient** | a shadow-casting directional light + ambient fill |

## Per-agent navmeshes

Two `AgentProfile`s are baked (`src/main.rs`): `"default"` (radius 0.5)
and `"wide"` (radius 1.0). Each agent routes on the navmesh for its own
profile. The headless check confirms the wide agent's path keeps a
strictly larger clearance from the walls (~1.1 m vs ~0.55 m).

## Runtime re-pathing

`contents/game.rhai` is the thin game layer. It lives in the contents
**root** (the script loader is non-recursive) and every few seconds flips
each agent's goal to the other side of the wall via `nav_goal_pos`, so
they continuously re-path. Because it writes `NavAgent.goal` (not the
mover intent), it never fights the path follower. In headless runs
`time_elapsed()` is frozen at 0, so the agents keep their scene-authored
goals — deterministic for the test.

Other nav script bindings available: `nav_goal_tag(e, tag)` (chase the
nearest tagged entity), `nav_target(e, bits)` (chase a specific entity),
`nav_stop(e)`, and `nav_rebuild()` (force a regeneration).

## Build & run

Requires Rust **1.88+** (the engine's MSRV) and read access to the
private engine repo (cargo fetches it as a git dependency). The engine
revision is pinned in `Cargo.toml`.

```sh
cargo build            # compile
cargo run              # open the window and watch the agents navigate
```

The platform crate is selected by target: `soxide-platform-linux` on
Linux, `soxide-platform-windows` on Windows.

Headless smoke test (no window/GPU) — builds the navmesh from the real
world geometry, asserts both agents route around the wall (per-profile
clearance), and walks an agent to its goal through the gap:

```sh
cargo run --example headless_check
```

## Known limitations (not bugs)

- The navmesh is generated from `Collider` geometry; mesh colliders
  contribute only their AABB, so this demo uses cuboids.
- There is no runtime debug-draw of the path yet — you verify by watching
  the character move (the headless check verifies the route analytically).
- Single-floor: one walkable surface per cell, so no bridges / overlapping
  levels.

## Continuous integration

`.github/workflows/ci.yml` builds and runs the headless check on Linux.
Because `soxide-engine` is a **private** git dependency, the workflow
needs a token to fetch it: add a Personal Access Token with read access
to `SoxWolf/Soxide` as the Actions secret **`ENGINE_TOKEN`**
(repo/org *Settings → Secrets and variables → Actions*). Without it the
build step fails with a clear error.

## Layout

```
sausage_playground.soxproj      project manifest (window, contents root, default scene)
Cargo.toml                      git deps on soxide-engine + the platform crate (nav branch)
src/main.rs                     loads the .soxproj, enables the navmesh build, runs the App
examples/headless_check.rs      no-window verification of navmesh generation + routing
.github/workflows/ci.yml        build + headless check on Linux (needs ENGINE_TOKEN secret)
contents/
├── game.rhai                   runtime goal switching (re-pathing) + HUD
├── scenes/main.soxscene        the scene above
├── input/*.soxaction           MoveForward/Back/Left/Right, Jump, Look (kept; unused by the demo)
└── input/gameplay.soxinputcontext   key bindings
```
