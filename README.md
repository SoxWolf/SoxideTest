# Sausage Playground

A minimal **third-person** sample for the [Soxide](https://github.com/SoxWolf/Soxide)
game engine. It is a standalone project that depends on the engine over
**git** (the engine lives in a separate repository — game and engine
trees stay physically disjoint, a hard Soxide invariant) and assembles a
playable character stack entirely from plain-text, diffable assets.

## What's in the scene

`contents/scenes/main.soxscene` (auto-loaded on startup as the project's
`default_scene`):

| Entity | Components |
|---|---|
| **Ground** | 20 × 1 × 20 static box — unit-cube mesh scaled up, `Fixed` body + `Cuboid` collider |
| **Ramp** | tilted (~20°) static box, below the mover's 45° slope limit so it's walkable |
| **Player** | the engine's test **sausage** skinned mesh (`meshes/sausage.fbx`) + a `CharacterMover` (kinematic collide-and-slide), tagged `Player`. Movement is driven by `game.rhai` (see below). |
| **PlayerController** | possesses the `Player`-tagged pawn by tag (`auto_possess_tag`) |
| **Camera** | `Camera3d` + a third-person `CameraRig` spring-arm that follows player 0 and orbits on the `Look` action |
| **Sun** / **Ambient** | a shadow-casting directional light + ambient fill |
| **Coin1–3** | gold cubes tagged `Coin`, collected by proximity (see `game.rhai`) |
| **Platform** | a `Kinematic` box off to the right (x=6), slid along Z by `game.rhai`; hoppable, out of the spawn→ramp path |
| **Enemy** | a slow red chaser (`CharacterMover` + `AiController`) steered by `game.rhai`; non-lethal — on contact *it* is sent home, never the player |

The ground, ramp, coins, platform and enemy use inline PBR materials.

The Controller → Pawn split is UE-flavoured: the **controller** is the
brain (possession), the **pawn** is the body (mover + mesh), and the
**camera rig** follows whatever the matching controller possesses.

## Input

Authored as Enhanced-Input assets under `contents/input/`:

- `MoveForward` / `MoveBack` / `MoveLeft` / `MoveRight` (`Bool`), `Jump` (`Bool`, `Pressed`), `Look` (`Axis2D`)
- `gameplay.soxinputcontext` binds **WASD** → the four directional actions, **Space** → Jump, **mouse X** → Look

Contexts load inactive; `contents/game.rhai` activates `gameplay` at
startup with one `add_input_context("gameplay", 0)` call, then composes
the four directional actions into a movement vector and feeds the mover.

| Action | Keys |
|---|---|
| Move | `W` `A` `S` `D` (world axis) |
| Jump | `Space` |
| Look | Mouse (yaw) — cursor is hidden at startup |

`main.rs` hides the OS cursor (`CursorGrab::HIDDEN`) for mouse-look. The
desktop runner reads motion from the **windowed cursor position** (no
pointer-lock / raw-motion path), so the cursor can't be truly captured —
turning is bounded by how far the cursor can travel across the window
before motion stops. Adjust turn speed with the CameraRig's
`look_sensitivity` in `contents/scenes/main.soxscene`. To quit, close the
window or `Ctrl-C` the terminal.

> **Engine note.** Movement is composed from four single-key `Bool`
> actions instead of one camera-relative `Axis2D` "Move", and Look is
> mouse-yaw only, because the engine's on-disk **`Swizzle` modifier**
> (which folding WASD onto an Axis2D and the mouse-Y onto pitch would
> require) does not round-trip through the `.soxinputcontext`
> `ModifierSpec` in this engine revision — its enum field serializes to
> `()` and decodes back as the identity. Avoiding `Swizzle` keeps the
> input fully functional from plain text.

## Gameplay script

`contents/game.rhai` is the thin game layer on top of the engine's
built-in systems (mover step, possession, physics, follow camera). It
lives in the contents **root** (not a subfolder) because the script
loader is non-recursive. Each frame it:

- drives the player from the four directional actions (`mover_input`) + jump;
- collects coins within ~1.2 m of the player (by tag + distance);
- steers the enemy toward the player; on contact the **enemy** is sent home (non-lethal — it never resets the player);
- slides the moving platform along Z, off to the side (`set_translation`);
- respawns the player only if it falls below `y = -8`;
- draws a HUD (coins collected, speed, movement mode, controls).

## Build & run

Requires Rust **1.88+** (the engine's MSRV) and read access to the
private engine repo (cargo fetches it as a git dependency). The engine
revision is pinned in `Cargo.toml`.

```sh
cargo build            # compile
cargo run              # open the window and play
```

The platform crate is selected by target: `soxide-platform-linux` on
Linux, `soxide-platform-windows` on Windows.

Headless smoke test (no window/GPU):

```sh
cargo run --example headless_check
```

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
Cargo.toml                      git deps on soxide-engine + the platform crate
src/main.rs                     loads the .soxproj, hands the App to the platform runner
examples/headless_check.rs      no-window verification of the whole asset stack
.github/workflows/ci.yml        build + headless check on Linux (needs ENGINE_TOKEN secret)
contents/
├── game.rhai                   movement + input activation + coins + enemy + platform + HUD + respawn
├── scenes/main.soxscene        the scene above
├── input/*.soxaction           MoveForward/Back/Left/Right, Jump, Look
├── input/gameplay.soxinputcontext   key bindings (no modifiers)
└── meshes/sausage.fbx          the player mesh (from the engine's test data)
```
