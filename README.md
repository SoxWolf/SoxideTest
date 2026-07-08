# Voxel World — a Soxide game

A Minecraft-flavoured, infinite, procedurally-generated voxel world built on
the [Soxide](https://github.com/SoxWolf/Soxide) engine (pulled in as a pinned
git dependency — the game tree and the engine tree stay physically disjoint).

The whole game is built in code (`src/lib.rs`); the binary just hands the
assembled `App` to the platform runner, which starts the simulation
un-paused.

## Run it

```sh
cargo run --release
```

**Controls:** `W/A/S/D` walk (gravity + terrain collision), mouse look,
`Space` jump, **left-click** carve a block, **right-click** place one, `F5`
save. Chunks stream in/out around you as you explore; edits re-mesh the
affected chunk live and persist to `voxel_world.save`.

## What's in it

| System | How |
|--------|-----|
| **Procedural terrain** | deterministic fBm value-noise heightmap, biomes (grass / dirt / stone / sand / snow) + a translucent water plane at sea level |
| **Greedy meshing** | coplanar same-block faces merge into big quads (few triangles per chunk); per-face directional shading (top/side/bottom) |
| **Textures** | a per-block detail atlas registered via `AssetServer::register_texture`, sampled through per-face UVs on every chunk material |
| **Streaming** | chunks load/unload around the player (`chunk_stream_tick`), with hysteresis; unloaded chunk meshes are freed via `AssetServer::remove_mesh` (no leak) |
| **First-person player** | a gravity `CharacterMover` pawn + mouse-look follow camera |
| **Block editing** | raycast → carve / place → mark the chunk dirty → re-mesh |
| **Live creatures** | a `SteerAgent { Wander }` population that streams around the player, materials cached once |
| **Persistence** | edits + seed + player pose saved to a plain-text file, restored on startup |

## Verification

```sh
cargo test --test voxel      # 13 headless tests (no window / GPU)
```

Covers: full-scale generation, greedy-meshing triangle reduction, chunk
streaming load/unload, mesh freeing on unload, block editing, texture atlas +
UVs, gravity landing + walking, creature streaming, and save/load round-trip.

## Engine dependency

Pinned to a `main` revision of `SoxWolf/Soxide` in `Cargo.toml`. CI needs a
PAT with read access to that private repo as the `ENGINE_TOKEN` Actions
secret. Building locally also needs the platform system libs:
`libudev-dev libasound2-dev pkg-config`.

## Also here: the navmesh demo

The repo's earlier navmesh sample lives on as an example plus its authored
assets (`contents/`, `sausage_playground.soxproj`,
`examples/headless_check.rs`). Run its headless check with:

```sh
cargo run --example headless_check
```
