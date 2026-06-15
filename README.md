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
| **Player** | the engine's test **sausage** skinned mesh (`meshes/sausage.fbx`), a `CharacterMover` (kinematic collide-and-slide), a `MoverInputBinding` (Move/Jump → intent), tagged `Player` |
| **PlayerController** | possesses the `Player`-tagged pawn by tag (`auto_possess_tag`) |
| **Camera** | `Camera3d` + a third-person `CameraRig` spring-arm that follows player 0 and orbits on the `Look` action |
| **Sun** / **Ambient** | a shadow-casting directional light + ambient fill |

The Controller → Pawn split is UE-flavoured: the **controller** is the
brain (possession), the **pawn** is the body (mover + mesh), and the
**camera rig** follows whatever the matching controller possesses.

## Input

Authored as Enhanced-Input assets under `contents/input/`:

- `Move.soxaction` (Axis2D), `Jump.soxaction` (Bool, `Pressed`), `Look.soxaction` (Axis2D)
- `gameplay.soxinputcontext` binds **WASD** → Move, **Space** → Jump, **mouse** → Look

Contexts load inactive; `contents/scripts/input_setup.rhai` activates
`gameplay` at startup with one `add_input_context("gameplay", 0)` call.

| Action | Keys |
|---|---|
| Move | `W` `A` `S` `D` (camera-relative) |
| Jump | `Space` |
| Look | Mouse |

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

## Layout

```
sausage_playground.soxproj      project manifest (window, contents root, default scene)
Cargo.toml                      git deps on soxide-engine + the platform crate
src/main.rs                     loads the .soxproj, hands the App to the platform runner
contents/
├── scenes/main.soxscene        the scene above
├── input/*.soxaction           Move / Jump / Look actions
├── input/gameplay.soxinputcontext   key bindings
├── scripts/input_setup.rhai    activates the input context
└── meshes/sausage.fbx          the player mesh (from the engine's test data)
```
