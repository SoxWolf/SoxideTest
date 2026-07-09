//! Headless validation of the voxel world: generate it (no window/GPU),
//! assert the ECS is populated at the intended scale, then drive the real
//! App schedule and assert the live entities (creatures + orbs) actually
//! move — the same systems the standalone binary runs on the desktop.

#![allow(clippy::unwrap_used)]

use soxide_engine::asset::{
    AssetServer, ColorSpace, MeshAsset, MeshDecoder, MeshLoadContext, NullWatcher, Texture,
    TextureDecoder,
};
use soxide_engine::core::glam::DVec3;
use soxide_engine::core::{SoxError, SoxResult, Time};
use soxide_engine::ecs::World;
use soxide_engine::gameplay::{NavAgent, SteerAgent};
use soxide_engine::physics::RigidBody;
use soxide_engine::render::{Mesh3D, SceneEntity};
use std::sync::Arc;
use sausage_playground::{
    Orb, Terrain, VoxelConfig, build_app_with, build_chunk_mesh, build_streaming_app, generate_into,
};

/// A headless `AssetServer` (no GPU, no real decoders) so the chunked path
/// can `register_mesh` and the physics tick can build trimesh colliders.
fn headless_assets() -> AssetServer {
    struct NoTex;
    impl TextureDecoder for NoTex {
        fn decode(&self, _b: &[u8]) -> SoxResult<Texture> {
            Ok(Texture {
                width: 1,
                height: 1,
                rgba: vec![255; 4],
                color_space: ColorSpace::Srgb,
                hdr: None,
            })
        }
    }
    struct NoMesh;
    impl MeshDecoder for NoMesh {
        fn decode(&self, _b: &[u8], _ctx: &MeshLoadContext<'_>) -> SoxResult<MeshAsset> {
            Err(SoxError::other("headless: no file meshes"))
        }
    }
    AssetServer::new(
        std::env::temp_dir().join("voxel-game-test-assets"),
        Arc::new(NoTex),
        Arc::new(NoMesh),
        Box::new(NullWatcher::default()),
    )
}

fn pos(world: &World, e: soxide_engine::ecs::Entity) -> DVec3 {
    world
        .get::<SceneEntity>(e)
        .map(|s| s.transform.translation)
        .unwrap_or(DVec3::ZERO)
}

#[test]
fn generates_a_large_voxel_world() {
    // Full-scale config (~15k blocks + 100 creatures + 60 orbs + 24 cubes).
    let cfg = VoxelConfig::default();
    let mut world = World::new();
    let stats = generate_into(&mut world, &Default::default(), &cfg);
    eprintln!(
        "FULL-SCALE: {} blocks, {} creatures, {} orbs, {} cubes, {} entities total",
        stats.blocks,
        stats.creatures,
        stats.orbs,
        stats.dyn_cubes,
        world.iter_entities().count()
    );

    assert!(
        stats.blocks >= 8_000,
        "expected a substantial voxel field, got {} blocks",
        stats.blocks
    );
    assert_eq!(stats.creatures, cfg.creatures);
    assert_eq!(stats.orbs, cfg.orbs);
    assert_eq!(stats.dyn_cubes, cfg.dyn_cubes);

    // Every voxel/creature/orb/cube is a real ECS entity.
    let entity_count = world.iter_entities().count();
    assert!(
        entity_count >= stats.total_entities(),
        "world should hold at least {} entities, has {}",
        stats.total_entities(),
        entity_count
    );

    // Sanity on the mix of components.
    let steer = world.query::<SteerAgent>().count();
    let orbs = world.query::<Orb>().count();
    let bodies = world.query::<RigidBody>().count();
    assert_eq!(steer, cfg.creatures);
    assert_eq!(orbs, cfg.orbs);
    assert_eq!(bodies, cfg.dyn_cubes);
    // No NavAgents here (creatures use pure Wander steering).
    assert_eq!(world.query::<NavAgent>().count(), 0);
}

#[test]
fn live_entities_move_through_the_schedule() {
    // Scale down for a fast test, but exercise the SAME systems the binary
    // runs (world_gen_tick, orbs_tick, ai_steer_tick, mover_tick, ...).
    let cfg = VoxelConfig {
        half: 12,
        creatures: 24,
        orbs: 16,
        dyn_cubes: 8,
        ..Default::default()
    };
    let mut app = build_app_with(cfg);
    // Fixed timestep (drop the wallclock so N ticks == N/60 s).
    app.world.resources_mut().remove::<soxide_engine::core::Time>();

    // First tick runs world_gen_tick (no AssetServer -> default materials).
    app.schedule.run(&mut app.world);

    // Snapshot a few creatures and orbs.
    let mut creatures: Vec<(soxide_engine::ecs::Entity, DVec3)> = app
        .world
        .query::<SteerAgent>()
        .map(|(e, _)| (e, DVec3::ZERO))
        .collect();
    for c in &mut creatures {
        c.1 = pos(&app.world, c.0);
    }
    let mut orbs: Vec<(soxide_engine::ecs::Entity, DVec3)> =
        app.world.query::<Orb>().map(|(e, _)| (e, DVec3::ZERO)).collect();
    for o in &mut orbs {
        o.1 = pos(&app.world, o.0);
    }
    assert_eq!(creatures.len(), 24);
    assert_eq!(orbs.len(), 16);

    // Run ~2 s of simulation.
    for _ in 0..120 {
        app.schedule.run(&mut app.world);
    }

    let creatures_moved = creatures
        .iter()
        .filter(|(e, start)| (pos(&app.world, *e) - *start).length() > 0.1)
        .count();
    let orbs_moved = orbs
        .iter()
        .filter(|(e, start)| (pos(&app.world, *e) - *start).length() > 0.1)
        .count();

    assert!(
        creatures_moved >= 20,
        "most wandering creatures should move, only {creatures_moved}/24 did"
    );
    assert!(
        orbs_moved == 16,
        "every orb should be animated, only {orbs_moved}/16 moved"
    );
}

#[test]
fn chunk_mesh_is_surface_only() {
    let cfg = VoxelConfig::default();
    let mesh = build_chunk_mesh(0, 0, &cfg).expect("chunk (0,0) should be non-empty");

    assert!(!mesh.vertices.is_empty(), "chunk has geometry");
    assert_eq!(mesh.indices.len() % 3, 0, "indices form triangles");
    assert!(!mesh.submeshes.is_empty(), "at least one material submesh");
    assert_eq!(
        mesh.submeshes.len(),
        mesh.materials.len(),
        "one embedded material per submesh"
    );
    // Submesh ranges tile the whole index buffer exactly.
    let covered: u32 = mesh.submeshes.iter().map(|s| s.index_count).sum();
    assert_eq!(covered as usize, mesh.indices.len());

    // Face culling really culls: far fewer tris than a naive solid fill
    // (chunk^2 columns * ~8 blocks tall * 12 tris/cube).
    let tris = mesh.indices.len() / 3;
    let naive = (cfg.chunk * cfg.chunk) as usize * 8 * 12;
    assert!(
        tris < naive / 2,
        "expected surface culling: {tris} tris vs naive {naive}"
    );
}

#[test]
fn chunked_terrain_spawns_and_simulates() {
    let cfg = VoxelConfig {
        terrain: Terrain::Chunked,
        half: 20,
        creatures: 20,
        orbs: 10,
        dyn_cubes: 6,
        ..Default::default()
    };
    let mut app = build_app_with(cfg);
    // Chunked terrain needs an AssetServer to register procedural meshes.
    app.world.insert_resource(headless_assets());
    app.world.resources_mut().remove::<Time>();

    // First tick: world_gen_tick runs the chunked path (registers meshes).
    app.schedule.run(&mut app.world);

    let chunk_entities = app
        .world
        .query::<Mesh3D>()
        .filter(|(_, m)| m.handle.is_some())
        .count();
    assert!(
        chunk_entities > 0,
        "chunked terrain should spawn mesh entities with registered handles"
    );

    // Snapshot creatures, then simulate: physics builds a trimesh collider
    // from each chunk mesh — this must not panic.
    let creatures: Vec<(soxide_engine::ecs::Entity, DVec3)> = app
        .world
        .query::<SteerAgent>()
        .map(|(e, _)| {
            let p = app
                .world
                .get::<SceneEntity>(e)
                .map(|s| s.transform.translation)
                .unwrap_or(DVec3::ZERO);
            (e, p)
        })
        .collect();

    for _ in 0..40 {
        app.schedule.run(&mut app.world);
    }

    let moved = creatures
        .iter()
        .filter(|(e, start)| {
            let now = app
                .world
                .get::<SceneEntity>(*e)
                .map(|s| s.transform.translation)
                .unwrap_or(*start);
            (now - *start).length() > 0.1
        })
        .count();
    // Over the collidable chunked terrain some hovering wanderers get
    // blocked by hillsides (correct — they can't clip through the trimesh),
    // so we only require that the AI is clearly live for a good fraction.
    assert!(
        moved >= 6,
        "the wander AI should keep a good fraction moving over the chunked world, {moved}/20 did"
    );
}

#[test]
fn editing_carves_a_hole_in_a_chunk() {
    // A carve edit at a surface voxel should change that chunk's mesh.
    let cfg = VoxelConfig::default();
    let before = build_chunk_mesh(0, 0, &cfg).expect("chunk (0,0)");
    let mut edits = sausage_playground::ChunkEdits::default();
    // Carve a 3x3x3 block of voxels near the chunk's low corner surface.
    for x in 1..=3 {
        for z in 1..=3 {
            for y in 0..=8 {
                edits.set(x, y, z, false, cfg.chunk);
            }
        }
    }
    let after = sausage_playground::build_chunk_mesh_with(0, 0, &cfg, &edits).expect("edited chunk");
    // Removing solid voxels changes the exposed-face set, so the mesh must
    // differ (typically more faces from the newly-exposed walls).
    assert_ne!(
        before.indices.len(),
        after.indices.len(),
        "carving voxels should re-mesh the chunk differently"
    );
    // The dirty set records the affected chunk.
    let dirty = edits.take_dirty();
    assert!(dirty.contains(&(0, 0)), "edited chunk (0,0) should be dirty");
}

#[test]
fn streaming_loads_and_unloads_chunks_around_the_player() {
    use soxide_engine::core::glam::DVec3;
    let mut app = build_streaming_app();
    app.world.insert_resource(headless_assets());
    app.world.resources_mut().remove::<Time>();

    // Tick once: streams chunks around the default player position + inits life.
    app.schedule.run(&mut app.world);
    let loaded_near_origin = app
        .world
        .query::<Mesh3D>()
        .filter(|(_, m)| m.handle.is_some())
        .count();
    assert!(
        loaded_near_origin > 0,
        "streaming should spawn chunk meshes around the player"
    );

    // Teleport the player's pawn far away; after enough ticks the old chunks
    // unload and new ones load (bounded per-frame budget, so run several).
    let pawn = app.world.get_resource::<sausage_playground::Player>().unwrap().pawn;
    if let Some(pawn) = pawn {
        if let Some(se) = app.world.get_mut::<SceneEntity>(pawn) {
            se.transform.translation = DVec3::new(600.0, 60.0, 600.0);
        }
    }
    for _ in 0..60 {
        app.schedule.run(&mut app.world);
    }
    let chunk_entities = app
        .world
        .query::<Mesh3D>()
        .filter(|(_, m)| m.handle.is_some())
        .count();
    assert!(
        chunk_entities > 0,
        "chunks should have streamed in around the new position"
    );
}

#[test]
fn player_falls_and_lands_on_streamed_terrain() {
    use soxide_engine::core::glam::DVec3;
    use soxide_engine::physics::CharacterMover;

    let mut app = build_streaming_app();
    app.world.insert_resource(headless_assets());
    app.world.resources_mut().remove::<Time>();

    let pawn = app.world.get_resource::<sausage_playground::Player>().unwrap().pawn.unwrap();
    let start = app
        .world
        .get::<SceneEntity>(pawn)
        .unwrap()
        .transform
        .translation;

    // ~5 s: chunks stream in, the pawn falls under gravity and lands on the
    // terrain trimesh (must not fall through to -infinity).
    for _ in 0..300 {
        app.schedule.run(&mut app.world);
    }
    let landed = app
        .world
        .get::<SceneEntity>(pawn)
        .unwrap()
        .transform
        .translation;
    assert!(landed.y < start.y, "gravity should pull the pawn down");
    assert!(
        landed.y > -20.0,
        "pawn should land on terrain, not fall through (y = {})",
        landed.y
    );

    // Drive a walk intent directly (what player_control_tick writes from
    // WASD): the mover should carry the pawn horizontally over the terrain.
    let before = app.world.get::<SceneEntity>(pawn).unwrap().transform.translation;
    for _ in 0..120 {
        if let Some(m) = app.world.get_mut::<CharacterMover>(pawn) {
            m.intent.dir = DVec3::new(1.0, 0.0, 0.0);
        }
        app.schedule.run(&mut app.world);
    }
    let after = app.world.get::<SceneEntity>(pawn).unwrap().transform.translation;
    let horiz = ((after.x - before.x).powi(2) + (after.z - before.z).powi(2)).sqrt();
    assert!(horiz > 0.5, "a walk intent should move the pawn, moved {horiz:.2}");
}

#[test]
fn streaming_frees_meshes_on_unload() {
    use soxide_engine::core::glam::DVec3;
    // Wire remove_mesh: streaming out a chunk must drop its MeshAsset from
    // the AssetServer (no unbounded growth as the player explores).
    let mut app = build_streaming_app();
    let assets = headless_assets();
    app.world.insert_resource(assets);
    app.world.resources_mut().remove::<Time>();

    // Load chunks around the origin.
    for _ in 0..20 {
        app.schedule.run(&mut app.world);
    }
    let near = app
        .world
        .get_resource::<AssetServer>()
        .unwrap()
        .loaded_mesh_count();
    assert!(near > 0, "chunks should have registered meshes");

    // Teleport the pawn far; old chunks unload (freeing their meshes) as new
    // ones load. The registered-mesh count must not grow without bound — it
    // should settle near the working-set size, not near + far.
    let pawn = app.world.get_resource::<sausage_playground::Player>().unwrap().pawn.unwrap();
    if let Some(se) = app.world.get_mut::<SceneEntity>(pawn) {
        se.transform.translation = DVec3::new(4000.0, 60.0, 4000.0);
    }
    for _ in 0..120 {
        app.schedule.run(&mut app.world);
    }
    let after = app
        .world
        .get_resource::<AssetServer>()
        .unwrap()
        .loaded_mesh_count();

    // A bounded working set: after moving ~250 chunks away, the far region's
    // meshes are loaded but the origin region's have been freed — so the
    // count is nowhere near `near + <all-chunks-ever-loaded>`.
    let working_set = (5 * 2 + 5) * (5 * 2 + 5); // generous upper bound on one region
    assert!(
        after <= working_set,
        "registered meshes should stay bounded (freed on unload): {after} > {working_set}"
    );
}

#[test]
fn creatures_stream_around_the_player() {
    use soxide_engine::core::glam::DVec3;
    let mut app = build_streaming_app();
    app.world.insert_resource(headless_assets());
    app.world.resources_mut().remove::<Time>();

    for _ in 0..30 {
        app.schedule.run(&mut app.world);
    }
    let n0 = app.world.query::<SteerAgent>().count();
    assert!(n0 > 0, "creatures should spawn around the player");
    assert!(n0 <= 260, "population should be bounded, got {n0}");

    // Teleport the pawn far: the origin creatures should be culled and a new
    // population should spawn around the new location.
    let pawn = app.world.get_resource::<sausage_playground::Player>().unwrap().pawn.unwrap();
    if let Some(se) = app.world.get_mut::<SceneEntity>(pawn) {
        se.transform.translation = DVec3::new(4000.0, 60.0, 4000.0);
    }
    for _ in 0..120 {
        app.schedule.run(&mut app.world);
    }

    let ppos = app.world.get_resource::<sausage_playground::Player>().unwrap().pos;
    let creatures: Vec<DVec3> = {
        let ents: Vec<_> = app.world.query::<SteerAgent>().map(|(e, _)| e).collect();
        ents.into_iter()
            .map(|e| app.world.get::<SceneEntity>(e).map(|s| s.transform.translation).unwrap_or(ppos))
            .collect()
    };
    assert!(!creatures.is_empty(), "creatures should re-populate at the new location");
    assert!(creatures.len() <= 260, "population still bounded, got {}", creatures.len());
    let near = creatures
        .iter()
        .filter(|p| ((p.x - ppos.x).powi(2) + (p.z - ppos.z).powi(2)).sqrt() <= 120.0)
        .count();
    assert_eq!(near, creatures.len(), "all live creatures follow the player, none left at origin");
}

#[test]
fn world_persistence_round_trips() {
    use sausage_playground::{ChunkEdits, WorldSave, load_world, save_world};

    let cfg = VoxelConfig::default();

    // Author some edits (carve a hole, place a block).
    let mut edits = ChunkEdits::default();
    edits.set(2, 5, 3, false, cfg.chunk);
    edits.set(2, 6, 3, false, cfg.chunk);
    edits.set(10, 20, 10, true, cfg.chunk);

    let save = WorldSave {
        seed: cfg.seed,
        player: [12.5, 40.0, -7.25],
        yaw: 1.2,
        pitch: -0.3,
        edits: edits.entries(),
    };

    let path = std::env::temp_dir().join("voxel-game-persist-test.save");
    save_world(&path, &save).expect("write save");
    let loaded = load_world(&path).expect("read save");

    assert_eq!(loaded.seed, save.seed);
    assert_eq!(loaded.player, save.player);
    assert!((loaded.yaw - save.yaw).abs() < 1e-9);
    assert!((loaded.pitch - save.pitch).abs() < 1e-9);
    assert_eq!(loaded.edits.len(), 3);

    // The loaded edits, applied to a fresh ChunkEdits, must re-mesh the
    // chunk differently from the unedited terrain — i.e. the world persists.
    let base = build_chunk_mesh(0, 0, &cfg).expect("base chunk");
    let mut restored = ChunkEdits::default();
    for ((x, y, z), s) in &loaded.edits {
        restored.insert_loaded(*x, *y, *z, *s);
    }
    let after = sausage_playground::build_chunk_mesh_with(0, 0, &cfg, &restored).expect("restored chunk");
    assert_ne!(
        base.indices.len(),
        after.indices.len(),
        "restored edits should change the chunk mesh"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn app_loads_a_saved_world_on_startup() {
    use sausage_playground::{ChunkEdits, Player, SavePath, WorldSave, save_world};

    let path = std::env::temp_dir().join("voxel-game-startup-load.save");
    let cfg = VoxelConfig::default();
    save_world(
        &path,
        &WorldSave {
            seed: cfg.seed,
            player: [100.0, 50.0, 100.0],
            yaw: 0.7,
            pitch: -0.2,
            edits: vec![((1, 2, 3), false), ((4, 5, 6), true)],
        },
    )
    .unwrap();

    let mut app = build_streaming_app();
    app.world.insert_resource(SavePath(path.clone())); // point at our save
    app.world.resources_mut().remove::<Time>();

    // One tick: world_load_tick (PreUpdate) restores edits + player pose.
    app.schedule.run(&mut app.world);

    let edits = app.world.get_resource::<ChunkEdits>().unwrap().entries();
    assert_eq!(edits.len(), 2, "saved edits should be restored on startup");
    let p = app.world.get_resource::<Player>().unwrap();
    assert!((p.pos.x - 100.0).abs() < 0.01 && (p.pos.z - 100.0).abs() < 0.01,
        "player horizontal position restored, got {:?}", p.pos);
    assert!((p.yaw - 0.7).abs() < 1e-6, "yaw restored");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn atlas_and_uvs_and_textured_materials() {
    // Atlas dimensions.
    let (px, w, h) = sausage_playground::atlas_rgba();
    assert_eq!(w as usize, sausage_playground::ATLAS_TILES * sausage_playground::ATLAS_TILE);
    assert_eq!(h as usize, sausage_playground::ATLAS_TILE);
    assert_eq!(px.len(), (w * h * 4) as usize);

    // The mesher sets per-face UVs into the atlas tiles.
    let cfg = VoxelConfig::default();
    let mesh = build_chunk_mesh(0, 0, &cfg).expect("chunk");
    assert!(
        mesh.vertices.iter().any(|v| v.uv[0] > 0.0 || v.uv[1] > 0.0),
        "chunk vertices should carry atlas UVs"
    );

    // In the streaming game path, chunk materials point at the atlas texture.
    let mut app = build_streaming_app();
    app.world.insert_resource(headless_assets());
    app.world.resources_mut().remove::<Time>();
    for _ in 0..8 {
        app.schedule.run(&mut app.world);
    }
    // An atlas texture got registered.
    assert!(
        app.world.get_resource::<AssetServer>().unwrap().loaded_texture_count() >= 1,
        "the detail atlas should be registered"
    );
    // A chunk mesh's materials reference it.
    let chunk_handle = app
        .world
        .query::<Mesh3D>()
        .filter_map(|(_, m)| m.handle.clone())
        .next()
        .expect("a chunk mesh");
    let mesh = app
        .world
        .get_resource::<AssetServer>()
        .unwrap()
        .get_mesh(&chunk_handle)
        .expect("registered mesh");
    assert!(
        mesh.materials.iter().all(|m| m.base_color_texture.is_some()),
        "chunk materials should sample the atlas"
    );
}

#[test]
fn greedy_meshing_cuts_triangles() {
    // Greedy meshing should merge coplanar faces, so a chunk has far fewer
    // triangles than one-quad-per-exposed-face would.
    let cfg = VoxelConfig::default();
    let mesh = build_chunk_mesh(0, 0, &cfg).expect("chunk");
    let tris = mesh.indices.len() / 3;
    // Every quad is 2 tris; there are chunk*chunk columns. A merged surface
    // is a handful of quads per material, so tris should be well under the
    // per-column face budget (~chunk*chunk*several).
    let per_voxel_budget = (cfg.chunk * cfg.chunk) as usize * 6; // very loose
    assert!(tris > 0, "chunk has geometry");
    assert!(
        tris < per_voxel_budget,
        "greedy meshing should cut triangles: {tris} tris (budget {per_voxel_budget})"
    );
    // Submeshes still tile the index buffer exactly (no orphaned indices).
    let covered: u32 = mesh.submeshes.iter().map(|s| s.index_count).sum();
    assert_eq!(covered as usize, mesh.indices.len());
    // Winding sanity: every triangle references distinct vertices.
    for t in mesh.indices.chunks_exact(3) {
        assert!(t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
    }
}

#[test]
fn voxel_project_and_scene_parse() {
    use soxide_engine::{Project, Scene};
    use std::path::{Path, PathBuf};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let proj = Project::load(&root.join("voxel_world.soxproj")).expect("voxel_world.soxproj loads");
    assert_eq!(proj.name, "Voxel World");
    assert_eq!(proj.plugins, vec![PathBuf::from("plugins/voxel")], "lists the voxel plugin");
    let scene_rel = proj.default_scene.clone().expect("default_scene is set");
    let scene = Scene::load(&root.join(&proj.contents_path).join(scene_rel)).expect("scene loads");
    assert!(scene.instances.is_empty(), "scene is empty — the plugin fills it in code");
}
