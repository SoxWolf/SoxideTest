//! Voxel-world ECS stress demo for the Soxide engine.
//!
//! A Minecraft-flavoured procedurally-generated world built entirely from
//! ECS entities — one entity per voxel — to lean on the ECS + the
//! instanced mesh renderer. On top of the terrain live ~100 wandering
//! creatures (steering AI), a swarm of orbiting "spirit" orbs (a custom
//! system), and a corner of tumbling dynamic-physics cubes.
//!
//! Generation cannot happen at `App::new` time because the `AssetServer`
//! (needed to make the coloured block materials) is only inserted once
//! the desktop runner starts. So [`build_app`] wires a one-shot
//! [`world_gen_tick`] system that generates the world on the first frame
//! and then disables itself.
//!
//! The generation core ([`generate_into`]) is engine-only (no window), so
//! the integration test drives it headlessly and asserts the entity
//! counts + that creatures actually move.

use soxide_engine::App;
use soxide_engine::asset::{
    AssetServer, BlendMode, ColorSpace, Material, MaterialHandle, MeshAsset, MeshVertex, Submesh,
    Texture,
};

// Export the full-`App` plugin entry points (`sox_plugin_build` +
// `sox_plugin_abi_version`) so the editor / runner can dlopen this cdylib and
// run the exact same game inside the editor (Unreal PIE / Godot style). Only
// meaningful when built as a `cdylib`; harmless in the rlib/bin builds.
soxide_engine::sox_plugin!(install_voxel_game);
use soxide_engine::core::Handle;
use soxide_engine::core::Time;
use soxide_engine::core::glam::{DQuat, DVec3};
use soxide_engine::ecs::{Entity, Stage, World};
use soxide_engine::window::{Input, KeyCode, MouseButton};
use soxide_engine::gameplay::{SteerAgent, SteerMode};
use soxide_engine::physics::{CharacterMover, Collider, MoverSettings, RigidBody};
use soxide_engine::render::{
    AmbientLight, Camera3d, DirectionalLight, Mesh3D, SceneEntity, Transform,
};

// ---------------------------------------------------------------------------
// Configuration + generation stats
// ---------------------------------------------------------------------------

/// How the terrain is realised in the ECS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terrain {
    /// One entity per voxel (~17k entities). Maximum ECS pressure; does
    /// not scale to a streaming world.
    PerVoxel,
    /// One face-culled mesh entity per chunk (a few dozen entities), each
    /// with a static trimesh collider. Scales; leaves the ECS budget for
    /// the *live* entities (creatures + orbs + physics).
    Chunked,
}

/// World-generation parameters. Inserted as a resource so the one-shot
/// [`world_gen_tick`] can read it.
#[derive(Clone, Copy, Debug)]
pub struct VoxelConfig {
    /// How terrain is spawned (per-voxel vs meshed chunks).
    pub terrain: Terrain,
    /// Chunk edge in columns (Chunked mode).
    pub chunk: i32,
    /// Half the terrain footprint: columns span `[-half, half)` on X and Z.
    pub half: i32,
    /// Noise seed.
    pub seed: u32,
    /// Blocks at or below this Y (when the surface is lower) are water.
    pub sea_level: i32,
    /// Peak surface height above Y=0.
    pub max_height: i32,
    /// Number of wandering creatures.
    pub creatures: usize,
    /// Number of orbiting spirit orbs.
    pub orbs: usize,
    /// Number of tumbling dynamic-physics cubes in the corner.
    pub dyn_cubes: usize,
}

impl Default for VoxelConfig {
    fn default() -> Self {
        Self {
            terrain: Terrain::Chunked,
            chunk: 16,
            half: 42,
            seed: 1337,
            sea_level: 5,
            max_height: 14,
            // Chunked terrain is cheap, so spend the ECS budget on a big
            // roaming population — that's where the stress lives now.
            creatures: 300,
            orbs: 150,
            dyn_cubes: 40,
        }
    }
}

/// One-shot latch so [`world_gen_tick`] generates exactly once.
#[derive(Clone, Copy, Debug, Default)]
pub struct GenState {
    pub done: bool,
}

/// What [`generate_into`] produced (for logging / tests).
#[derive(Clone, Copy, Debug, Default)]
pub struct GenStats {
    pub blocks: usize,
    pub creatures: usize,
    pub orbs: usize,
    pub dyn_cubes: usize,
}

impl GenStats {
    pub fn total_entities(&self) -> usize {
        self.blocks + self.creatures + self.orbs + self.dyn_cubes
    }
}

// ---------------------------------------------------------------------------
// Custom "spirit orb" component + its motion system
// ---------------------------------------------------------------------------

/// Accumulated simulation time for [`orbs_tick`], advanced from per-frame
/// delta (so it works with the wallclock runner and with a fixed-step test
/// alike, where `Time::elapsed` would stay pinned at zero).
#[derive(Clone, Copy, Debug, Default)]
pub struct OrbClock(pub f64);

/// A floating orb that bobs vertically and orbits its anchor. Driven by
/// [`orbs_tick`] purely from elapsed time — extra live ECS entities with
/// no physics cost.
#[derive(Clone, Copy, Debug)]
pub struct Orb {
    pub cx: f64,
    pub cz: f64,
    pub base_y: f64,
    pub amp: f64,
    pub phase: f64,
    pub bob_speed: f64,
    pub orbit_r: f64,
    pub orbit_speed: f64,
}

/// `Stage::Update` system: animate every [`Orb`]'s transform.
pub fn orbs_tick(world: &mut World) {
    let dt = world
        .get_resource::<Time>()
        .map(|x| x.delta_secs())
        .unwrap_or(1.0 / 60.0);
    let dt = if dt > 0.0 { dt } else { 1.0 / 60.0 };
    let t = {
        match world.get_resource_mut::<OrbClock>() {
            Some(c) => {
                c.0 += dt;
                c.0
            }
            None => dt,
        }
    };
    let orbs: Vec<(Entity, Orb)> = world.query::<Orb>().map(|(e, o)| (e, *o)).collect();
    for (e, o) in orbs {
        let y = o.base_y + (t * o.bob_speed + o.phase).sin() * o.amp;
        let ang = t * o.orbit_speed + o.phase;
        let x = o.cx + o.orbit_r * ang.cos();
        let z = o.cz + o.orbit_r * ang.sin();
        if let Some(se) = world.get_mut::<SceneEntity>(e) {
            se.transform.translation = DVec3::new(x, y, z);
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic terrain noise (no std rand — resume-safe, seedable)
// ---------------------------------------------------------------------------

fn hash2(x: i32, z: i32, seed: u32) -> u32 {
    let mut h = seed ^ 0x9E37_79B9;
    h = h.wrapping_add((x as u32).wrapping_mul(0x85EB_CA6B));
    h ^= h >> 13;
    h = h.wrapping_add((z as u32).wrapping_mul(0xC2B2_AE35));
    h ^= h >> 16;
    h = h.wrapping_mul(0x27D4_EB2F);
    h ^= h >> 15;
    h
}

fn rand01(x: i32, z: i32, seed: u32) -> f64 {
    (hash2(x, z, seed) as f64) / (u32::MAX as f64)
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Bilinear value noise over the integer hash lattice.
fn value_noise(x: f64, z: f64, seed: u32) -> f64 {
    let x0 = x.floor() as i32;
    let z0 = z.floor() as i32;
    let fx = smooth(x - x0 as f64);
    let fz = smooth(z - z0 as f64);
    let v00 = rand01(x0, z0, seed);
    let v10 = rand01(x0 + 1, z0, seed);
    let v01 = rand01(x0, z0 + 1, seed);
    let v11 = rand01(x0 + 1, z0 + 1, seed);
    let a = v00 + (v10 - v00) * fx;
    let b = v01 + (v11 - v01) * fx;
    a + (b - a) * fz
}

/// Fractal (fBm) surface height at a world column, in `1..=max_h`.
fn height_at(wx: i32, wz: i32, seed: u32, max_h: i32) -> i32 {
    let mut freq = 1.0 / 18.0;
    let mut amp = 1.0;
    let mut sum = 0.0;
    let mut norm = 0.0;
    for _ in 0..4 {
        sum += amp * value_noise(wx as f64 * freq, wz as f64 * freq, seed);
        norm += amp;
        freq *= 2.0;
        amp *= 0.5;
    }
    let h01 = (sum / norm).clamp(0.0, 1.0);
    (1.0 + h01.powf(1.15) * max_h as f64).round() as i32
}

// ---------------------------------------------------------------------------
// Block palette
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Kind {
    Grass,
    Dirt,
    Stone,
    Sand,
    Snow,
    Water,
}

/// Coloured materials for each block/creature/orb type. Empty (all `None`)
/// when no `AssetServer` is present (headless tests) — blocks then render
/// with the default material, which is fine for counting/movement checks.
#[derive(Default, Clone)]
pub struct Palette {
    grass: Option<MaterialHandle>,
    dirt: Option<MaterialHandle>,
    stone: Option<MaterialHandle>,
    sand: Option<MaterialHandle>,
    snow: Option<MaterialHandle>,
    water: Option<MaterialHandle>,
    creatures: Vec<Option<MaterialHandle>>,
    orb: Option<MaterialHandle>,
    prop: Option<MaterialHandle>,
}

impl Palette {
    fn block(&self, k: Kind) -> &Option<MaterialHandle> {
        match k {
            Kind::Grass => &self.grass,
            Kind::Dirt => &self.dirt,
            Kind::Stone => &self.stone,
            Kind::Sand => &self.sand,
            Kind::Snow => &self.snow,
            Kind::Water => &self.water,
        }
    }

    /// Creature material by index, tolerant of an empty (headless) palette.
    fn creature(&self, i: usize) -> Option<MaterialHandle> {
        if self.creatures.is_empty() {
            None
        } else {
            self.creatures[i % self.creatures.len()].clone()
        }
    }
}

/// Build the coloured palette from the world's `AssetServer` (if present).
fn make_palette(world: &World) -> Palette {
    let Some(a) = world.get_resource::<AssetServer>() else {
        // Headless: no materials, three empty creature slots so indexing works.
        return Palette {
            creatures: vec![None, None, None],
            ..Default::default()
        };
    };
    let solid = |rgba: [f32; 4]| Some(a.register_material(Material::default().with_base_color(rgba)));
    let water = Some(
        a.register_material(
            Material::default()
                .with_base_color([0.20, 0.45, 0.85, 0.55])
                .with_blend_mode(BlendMode::Blend),
        ),
    );
    let orb = Some(
        a.register_material(
            Material::default()
                .with_base_color([0.95, 0.80, 0.35, 1.0])
                .with_emissive([0.6, 0.45, 0.12]),
        ),
    );
    Palette {
        grass: solid([0.28, 0.60, 0.22, 1.0]),
        dirt: solid([0.45, 0.32, 0.20, 1.0]),
        stone: solid([0.50, 0.50, 0.53, 1.0]),
        sand: solid([0.85, 0.78, 0.50, 1.0]),
        snow: solid([0.95, 0.97, 1.00, 1.0]),
        water,
        creatures: vec![
            solid([0.90, 0.30, 0.30, 1.0]),
            solid([0.30, 0.55, 0.95, 1.0]),
            solid([0.85, 0.50, 0.90, 1.0]),
        ],
        orb,
        prop: solid([0.95, 0.55, 0.15, 1.0]),
    }
}

fn block_kind(surface_h: i32, y: i32, sea_level: i32, top_of_column: bool) -> Kind {
    if top_of_column && surface_h >= 12 {
        Kind::Snow
    } else if top_of_column && surface_h <= sea_level + 1 {
        Kind::Sand
    } else if top_of_column {
        Kind::Grass
    } else if y >= surface_h - 2 {
        Kind::Dirt
    } else {
        Kind::Stone
    }
}

// ---------------------------------------------------------------------------
// Spawning helpers
// ---------------------------------------------------------------------------

fn cube_mesh(mat: &Option<MaterialHandle>) -> Mesh3D {
    match mat {
        Some(h) => Mesh3D::cube().with_material(h.clone()),
        None => Mesh3D::cube(),
    }
}

fn spawn_block(world: &mut World, x: i32, y: i32, z: i32, mat: &Option<MaterialHandle>) {
    let e = world.spawn(SceneEntity::from_translation(DVec3::new(
        x as f64, y as f64, z as f64,
    )));
    world.insert(e, cube_mesh(mat));
}

// ---------------------------------------------------------------------------
// World generation
// ---------------------------------------------------------------------------

/// Generate the whole world (per-voxel terrain + life) into `world`.
/// Engine-only: no window/GPU needed, so tests can call it directly.
pub fn generate_into(world: &mut World, pal: &Palette, cfg: &VoxelConfig) -> GenStats {
    let mut stats = GenStats::default();
    generate_terrain_voxels(world, pal, cfg, &mut stats);
    spawn_life(world, pal, cfg, &mut stats);
    stats
}

/// Per-voxel terrain: a watertight surface shell (top + exposed sides),
/// one entity per block. Maximum ECS pressure.
fn generate_terrain_voxels(
    world: &mut World,
    pal: &Palette,
    cfg: &VoxelConfig,
    stats: &mut GenStats,
) {
    let (half, seed, sea, max_h) = (cfg.half, cfg.seed, cfg.sea_level, cfg.max_height);
    for x in -half..half {
        for z in -half..half {
            let h = height_at(x, z, seed, max_h);
            let neighbours = [
                height_at(x - 1, z, seed, max_h),
                height_at(x + 1, z, seed, max_h),
                height_at(x, z - 1, seed, max_h),
                height_at(x, z + 1, seed, max_h),
            ];
            let nmin = neighbours.iter().copied().min().unwrap_or(h);
            // Fill from just below the lowest neighbour up to our surface, so
            // the exposed side faces are solid but deep interior is skipped.
            let floor_y = nmin.saturating_sub(1).max(0);
            for y in floor_y..=h {
                let k = block_kind(h, y, sea, y == h);
                spawn_block(world, x, y, z, pal.block(k));
                stats.blocks += 1;
            }
            // Water fills low columns up to sea level.
            if h < sea {
                for y in (h + 1)..=sea {
                    spawn_block(world, x, y, z, pal.block(Kind::Water));
                    stats.blocks += 1;
                }
            }
        }
    }
}

/// Spawn the live entities shared by both terrain modes: wandering
/// creatures (steering AI), orbiting spirit orbs (custom system), and the
/// dynamic-physics corner.
fn spawn_life(world: &mut World, pal: &Palette, cfg: &VoxelConfig, stats: &mut GenStats) {
    let (half, seed, max_h) = (cfg.half, cfg.seed, cfg.max_height);

    // --- Creatures: wandering steering agents (gravity-free hover-walk) ----
    for i in 0..cfg.creatures {
        let i = i as i32;
        let cx = (rand01(i, 7, seed) * 2.0 - 1.0) * (half as f64 * 0.9);
        let cz = (rand01(i, 13, seed) * 2.0 - 1.0) * (half as f64 * 0.9);
        let surf = height_at(cx.round() as i32, cz.round() as i32, seed, max_h);
        let cy = surf as f64 + 1.5;
        let mat = pal.creature(i as usize);

        let e = world.spawn(SceneEntity::new(Transform {
            translation: DVec3::new(cx, cy, cz),
            scale: DVec3::splat(0.6),
            ..Default::default()
        }));
        world.insert(e, cube_mesh(&mat));
        world.insert(
            e,
            CharacterMover {
                mode: "flying".to_string(),
                settings: MoverSettings {
                    gravity_scale: 0.0,
                    max_speed: 3.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        // SteerAgent has private runtime fields, so build via Default + set
        // the public knobs rather than a struct literal.
        let mut steer = SteerAgent::default();
        steer.mode = SteerMode::Wander;
        steer.speed_scale = 0.85;
        steer.wander_turn_deg = 55.0;
        world.insert(e, steer);
        stats.creatures += 1;
    }

    // --- Spirit orbs: custom bobbing/orbiting system ----------------------
    for i in 0..cfg.orbs {
        let i = i as i32;
        let cx = (rand01(i, 31, seed) * 2.0 - 1.0) * (half as f64 * 0.85);
        let cz = (rand01(i, 37, seed) * 2.0 - 1.0) * (half as f64 * 0.85);
        let surf = height_at(cx.round() as i32, cz.round() as i32, seed, max_h);
        let orb = Orb {
            cx,
            cz,
            base_y: surf as f64 + 3.0 + rand01(i, 41, seed) * 4.0,
            amp: 0.6 + rand01(i, 43, seed) * 1.2,
            phase: rand01(i, 47, seed) * 6.283,
            bob_speed: 0.8 + rand01(i, 53, seed) * 1.4,
            orbit_r: 1.5 + rand01(i, 59, seed) * 3.0,
            orbit_speed: 0.4 + rand01(i, 61, seed) * 0.8,
        };
        let e = world.spawn(SceneEntity::new(Transform {
            translation: DVec3::new(orb.cx + orb.orbit_r, orb.base_y, orb.cz),
            scale: DVec3::splat(0.4),
            ..Default::default()
        }));
        world.insert(e, cube_mesh(&pal.orb));
        world.insert(e, orb);
        stats.orbs += 1;
    }

    // --- Physics corner: a floating platform + tumbling dynamic cubes -----
    let (px, py, pz) = (0.0_f64, 16.0_f64, 44.0_f64);
    let plat = world.spawn(SceneEntity::new(Transform {
        translation: DVec3::new(px, py, pz),
        scale: DVec3::new(12.0, 1.0, 12.0),
        ..Default::default()
    }));
    world.insert(plat, cube_mesh(&pal.stone));
    // Collider half-extents are scaled by the entity transform (12,1,12) ->
    // (6, 0.5, 6): a solid 12x1x12 platform.
    world.insert(plat, Collider::cuboid(0.5, 0.5, 0.5));
    for i in 0..cfg.dyn_cubes {
        let i = i as i32;
        let ox = px + (rand01(i, 101, seed) * 2.0 - 1.0) * 4.5;
        let oz = pz + (rand01(i, 103, seed) * 2.0 - 1.0) * 4.5;
        let oy = py + 4.0 + (i as f64) * 0.9;
        let mat = &pal.prop;
        let e = world.spawn(SceneEntity::new(Transform {
            translation: DVec3::new(ox, oy, oz),
            ..Default::default()
        }));
        world.insert(e, cube_mesh(mat));
        world.insert(e, RigidBody::dynamic());
        world.insert(e, Collider::cuboid(0.5, 0.5, 0.5));
        stats.dyn_cubes += 1;
    }
}

// ---------------------------------------------------------------------------
// Chunked terrain: one face-culled mesh entity per chunk
// ---------------------------------------------------------------------------

/// The 6 cube faces. Each: outward normal, then 4 corner offsets (from the
/// voxel's minimum corner) wound CCW as seen from outside — so with the
/// renderer's back-face culling (front = CCW) the faces show outward. Each
/// verified so `(v1-v0) x (v2-v0)` points along the normal.
const FACES: [([f32; 3], [[f32; 3]; 4]); 6] = [
    // +X
    ([1.0, 0.0, 0.0], [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 1.0], [1.0, 0.0, 1.0]]),
    // -X
    ([-1.0, 0.0, 0.0], [[0.0, 0.0, 1.0], [0.0, 1.0, 1.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]]),
    // +Y
    ([0.0, 1.0, 0.0], [[0.0, 1.0, 0.0], [0.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, 0.0]]),
    // -Y
    ([0.0, -1.0, 0.0], [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 0.0, 1.0]]),
    // +Z
    ([0.0, 0.0, 1.0], [[1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0], [0.0, 0.0, 1.0]]),
    // -Z
    ([0.0, 0.0, -1.0], [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 0.0, 0.0]]),
];


/// The block kinds that get their own submesh + colour, in a fixed order.
const CHUNK_KINDS: [Kind; 5] = [Kind::Grass, Kind::Dirt, Kind::Stone, Kind::Sand, Kind::Snow];

fn kind_slot(k: Kind) -> usize {
    match k {
        Kind::Grass => 0,
        Kind::Dirt => 1,
        Kind::Stone => 2,
        Kind::Sand => 3,
        Kind::Snow => 4,
        // Water is not meshed in Chunked mode (kept simple); treat as stone.
        Kind::Water => 2,
    }
}

fn kind_color(k: Kind) -> [f32; 4] {
    match k {
        Kind::Grass => [0.28, 0.60, 0.22, 1.0],
        Kind::Dirt => [0.45, 0.32, 0.20, 1.0],
        Kind::Stone => [0.50, 0.50, 0.53, 1.0],
        Kind::Sand => [0.85, 0.78, 0.50, 1.0],
        Kind::Snow => [0.95, 0.97, 1.00, 1.0],
        Kind::Water => [0.20, 0.45, 0.85, 1.0],
    }
}

/// Number of tiles in the block-detail atlas (grass, dirt, stone, sand,
/// snow, water — indexed to match [`kind_slot`], water last).
pub const ATLAS_TILES: usize = 6;
/// Pixel size of one atlas tile.
pub const ATLAS_TILE: usize = 16;

/// UV of each face corner within its tile, matching the [`FACES`] corner
/// order (so the two CCW triangles map the quad onto the tile).
const CORNER_UV: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];

/// Build the grayscale **detail** atlas (RGBA8) as `ATLAS_TILES` tiles in a
/// horizontal strip. Each tile is a per-block-type pattern in ~[0.55, 1.0]
/// with a slightly darker border — meant to *multiply* over the block's
/// coloured `base_color`, adding texture without changing the biome colour.
pub fn atlas_rgba() -> (Vec<u8>, u32, u32) {
    let ts = ATLAS_TILE;
    let w = ts * ATLAS_TILES;
    let h = ts;
    let mut px = vec![0u8; w * h * 4];
    for tile in 0..ATLAS_TILES {
        for y in 0..ts {
            for x in 0..ts {
                let n = hash2((tile * 131 + x) as i32, y as i32, 0x51ED) as f64 / (u32::MAX as f64);
                let edge = x == 0 || y == 0 || x == ts - 1 || y == ts - 1;
                let base = match tile {
                    0 => 0.82 + n * 0.18, // grass: speckled
                    1 => 0.72 + n * 0.24, // dirt: rough
                    2 => 0.78 + n * 0.16, // stone: mid
                    3 => 0.88 + n * 0.12, // sand: fine
                    4 => 0.92 + n * 0.08, // snow: smooth bright
                    _ => 0.85 + n * 0.10, // water: gentle
                };
                let g = (base * if edge { 0.7 } else { 1.0 }).clamp(0.0, 1.0);
                let v = (g * 255.0) as u8;
                let i = (y * w + tile * ts + x) * 4;
                px[i] = v;
                px[i + 1] = v;
                px[i + 2] = v;
                px[i + 3] = 255;
            }
        }
    }
    (px, w as u32, h as u32)
}

/// UV for a face corner in the given kind's tile.
fn tile_uv(kind_idx: usize, corner: usize) -> [f32; 2] {
    let tw = 1.0 / ATLAS_TILES as f32;
    let c = CORNER_UV[corner];
    [(kind_idx as f32 + c[0]) * tw, c[1]]
}

/// Per-voxel edits layered over the noise terrain: `true` = a placed solid
/// block, `false` = a carved-out hole. Keyed by world voxel coordinate.
#[derive(Default)]
pub struct ChunkEdits {
    voxels: std::collections::HashMap<(i32, i32, i32), bool>,
    dirty: std::collections::HashSet<(i32, i32)>,
}

impl ChunkEdits {
    /// Override a voxel's solidity and mark its chunk dirty for a rebuild.
    pub fn set(&mut self, wx: i32, wy: i32, wz: i32, is_solid: bool, chunk: i32) {
        self.voxels.insert((wx, wy, wz), is_solid);
        self.dirty
            .insert((wx.div_euclid(chunk), wz.div_euclid(chunk)));
    }
    /// Drain the set of chunks that changed since the last call.
    pub fn take_dirty(&mut self) -> Vec<(i32, i32)> {
        let out: Vec<_> = self.dirty.iter().copied().collect();
        self.dirty.clear();
        out
    }

    /// Snapshot every edit as `((x, y, z), is_solid)` — for saving.
    pub fn entries(&self) -> Vec<((i32, i32, i32), bool)> {
        self.voxels.iter().map(|(k, v)| (*k, *v)).collect()
    }

    /// Apply a loaded edit without marking the chunk dirty (the chunk is
    /// built with the edit from the start, so no rebuild is needed).
    pub fn insert_loaded(&mut self, wx: i32, wy: i32, wz: i32, is_solid: bool) {
        self.voxels.insert((wx, wy, wz), is_solid);
    }
}

fn solid_e(wx: i32, y: i32, wz: i32, seed: u32, max_h: i32, edits: &ChunkEdits) -> bool {
    if let Some(&s) = edits.voxels.get(&(wx, y, wz)) {
        return s;
    }
    y >= 0 && y <= height_at(wx, wz, seed, max_h)
}

/// [`build_chunk_mesh_with`] with no edits.
pub fn build_chunk_mesh(cx: i32, cz: i32, cfg: &VoxelConfig) -> Option<MeshAsset> {
    build_chunk_mesh_with(cx, cz, cfg, &ChunkEdits::default())
}

/// Greedy mesher: for each of the 6 face orientations, merge coplanar
/// exposed faces of the same (block-kind, face-class) into the largest
/// possible rectangles — far fewer triangles than one quad per voxel face.
/// Emits into the per-slot `buckets` (kind*3 + class). Positions are
/// chunk-local; neighbour queries use world coords so chunk seams are
/// exact.
fn greedy_terrain(
    cx: i32,
    cz: i32,
    cfg: &VoxelConfig,
    edits: &ChunkEdits,
    vertices: &mut Vec<MeshVertex>,
    buckets: &mut [Vec<u32>],
) {
    let (chunk, seed, sea, max_h) = (cfg.chunk, cfg.seed, cfg.sea_level, cfg.max_height);
    let ymax = max_h + 5;
    // Axis sizes: x_local in 0..chunk, y in 0..ymax, z_local in 0..chunk.
    let dims = [chunk, ymax, chunk];

    // Kind index (0..CHUNK_KINDS) of a solid voxel.
    let kind_at = |wx: i32, y: i32, wz: i32| -> usize {
        let h = height_at(wx, wz, seed, max_h);
        let is_top = !solid_e(wx, y + 1, wz, seed, max_h, edits);
        kind_slot(block_kind(h, y, sea, is_top))
    };

    for d in 0..3usize {
        let ua = (d + 1) % 3;
        let va = (d + 2) % 3;
        let (s_n, u_n, v_n) = (dims[d], dims[ua], dims[va]);
        for &dir in &[1i32, -1] {
            let class = if d == 1 {
                if dir > 0 { 0 } else { 2 }
            } else {
                1
            };
            for slice in 0..s_n {
                // Build the exposed-face mask for this slice.
                let mut mask = vec![-1i32; (u_n * v_n) as usize];
                for uc in 0..u_n {
                    for vc in 0..v_n {
                        let mut vloc = [0i32; 3];
                        vloc[d] = slice;
                        vloc[ua] = uc;
                        vloc[va] = vc;
                        let (wx, y, wz) = (cx * chunk + vloc[0], vloc[1], cz * chunk + vloc[2]);
                        if !solid_e(wx, y, wz, seed, max_h, edits) {
                            continue;
                        }
                        let mut nloc = vloc;
                        nloc[d] += dir;
                        let (nwx, ny, nwz) = (cx * chunk + nloc[0], nloc[1], cz * chunk + nloc[2]);
                        if solid_e(nwx, ny, nwz, seed, max_h, edits) {
                            continue; // face hidden
                        }
                        mask[(uc * v_n + vc) as usize] = kind_at(wx, y, wz) as i32;
                    }
                }
                // Greedy-merge equal-kind rectangles.
                let mut used = vec![false; (u_n * v_n) as usize];
                for u0 in 0..u_n {
                    for v0 in 0..v_n {
                        let i0 = (u0 * v_n + v0) as usize;
                        if mask[i0] < 0 || used[i0] {
                            continue;
                        }
                        let k = mask[i0];
                        // width along v
                        let mut w = 1;
                        while v0 + w < v_n {
                            let i = (u0 * v_n + v0 + w) as usize;
                            if mask[i] != k || used[i] {
                                break;
                            }
                            w += 1;
                        }
                        // height along u
                        let mut hgt = 1;
                        'ext: while u0 + hgt < u_n {
                            for vv in v0..v0 + w {
                                let i = ((u0 + hgt) * v_n + vv) as usize;
                                if mask[i] != k || used[i] {
                                    break 'ext;
                                }
                            }
                            hgt += 1;
                        }
                        for uu in u0..u0 + hgt {
                            for vv in v0..v0 + w {
                                used[(uu * v_n + vv) as usize] = true;
                            }
                        }
                        // Emit the merged quad.
                        let plane = if dir > 0 { slice + 1 } else { slice };
                        let corner = |uu: i32, vv: i32| -> [f32; 3] {
                            let mut p = [0f32; 3];
                            p[d] = plane as f32;
                            p[ua] = uu as f32;
                            p[va] = vv as f32;
                            p
                        };
                        let mut normal = [0f32; 3];
                        normal[d] = dir as f32;
                        let (u1, v1) = (u0 + hgt, v0 + w);
                        let quad = if dir > 0 {
                            [corner(u0, v0), corner(u1, v0), corner(u1, v1), corner(u0, v1)]
                        } else {
                            [corner(u0, v0), corner(u0, v1), corner(u1, v1), corner(u1, v0)]
                        };
                        let kind = k as usize;
                        let slot = kind * 3 + class;
                        let base = vertices.len() as u32;
                        for (ci, c) in quad.iter().enumerate() {
                            vertices.push(MeshVertex {
                                position: *c,
                                normal,
                                uv: tile_uv(kind, ci),
                                tangent: [0.0, 0.0, 0.0, 0.0],
                            });
                        }
                        buckets[slot].extend_from_slice(&[
                            base,
                            base + 1,
                            base + 2,
                            base,
                            base + 2,
                            base + 3,
                        ]);
                    }
                }
            }
        }
    }
}

/// Build one chunk's face-culled mesh (local coordinates, origin at the
/// chunk's `(cx*chunk, 0, cz*chunk)`), consulting `edits`. Colours ride
/// inside the mesh as per-submesh embedded materials, so no `AssetServer`
/// is needed to build it. Returns `None` for an empty chunk.
pub fn build_chunk_mesh_with(
    cx: i32,
    cz: i32,
    cfg: &VoxelConfig,
    edits: &ChunkEdits,
) -> Option<MeshAsset> {
    let (chunk, seed, sea, max_h) = (cfg.chunk, cfg.seed, cfg.sea_level, cfg.max_height);
    let mut vertices: Vec<MeshVertex> = Vec::new();
    // One index list per (kind, face-class) slot for per-face directional
    // shading (top / side / bottom), plus one trailing slot for the water
    // surface. Concatenated into submeshes later.
    const WATER_SLOT: usize = CHUNK_KINDS.len() * 3;
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); WATER_SLOT + 1];

    greedy_terrain(cx, cz, cfg, edits, &mut vertices, &mut buckets);

    // Water surface: a flat translucent quad at sea level over every
    // submerged column (surface height < sea), giving valleys a water plane.
    for lx in 0..chunk {
        for lz in 0..chunk {
            let wx = cx * chunk + lx;
            let wz = cz * chunk + lz;
            if height_at(wx, wz, seed, max_h) >= sea {
                continue;
            }
            let (normal, corners) = FACES[2]; // +Y, CCW from above
            let base = vertices.len() as u32;
            for (ci, c) in corners.iter().enumerate() {
                vertices.push(MeshVertex {
                    position: [lx as f32 + c[0], sea as f32, lz as f32 + c[2]],
                    normal,
                    uv: tile_uv(5, ci), // water tile
                    tangent: [0.0, 0.0, 0.0, 0.0],
                });
            }
            buckets[WATER_SLOT]
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    if vertices.is_empty() {
        return None;
    }

    // Concatenate the per-kind index buckets into one buffer + submeshes.
    let mut indices: Vec<u32> = Vec::new();
    let mut submeshes: Vec<Submesh> = Vec::new();
    let mut materials: Vec<Material> = Vec::new();
    for (slot, bucket) in buckets.into_iter().enumerate() {
        if bucket.is_empty() {
            continue;
        }
        let start = indices.len() as u32;
        let count = bucket.len() as u32;
        indices.extend_from_slice(&bucket);
        let material_slot = materials.len() as u32;
        let mat = if slot == WATER_SLOT {
            Material::default()
                .with_base_color([0.18, 0.40, 0.78, 0.6])
                .with_blend_mode(BlendMode::Blend)
        } else {
            let kind = CHUNK_KINDS[slot / 3];
            let shade = [1.0f32, 0.72, 0.5][slot % 3];
            let c = kind_color(kind);
            Material::default().with_base_color([c[0] * shade, c[1] * shade, c[2] * shade, c[3]])
        };
        materials.push(mat);
        submeshes.push(Submesh {
            index_start: start,
            index_count: count,
            material_slot: Some(material_slot),
        });
    }

    Some(MeshAsset {
        vertices,
        indices,
        submeshes,
        materials,
        skin: None,
    })
}

/// Holds the block-detail atlas texture handle once registered.
#[derive(Default)]
pub struct AtlasTex(pub Option<Handle<Texture>>);

/// Register the block-detail atlas once and return its handle (cached in
/// the `AtlasTex` resource). `None` if there's no `AssetServer` yet.
fn ensure_atlas(world: &mut World) -> Option<Handle<Texture>> {
    if let Some(h) = world.get_resource::<AtlasTex>().and_then(|a| a.0.clone()) {
        return Some(h);
    }
    let (px, w, h) = atlas_rgba();
    let handle = world.get_resource::<AssetServer>()?.register_texture(Texture {
        width: w,
        height: h,
        rgba: px,
        color_space: ColorSpace::Srgb,
        hdr: None,
    });
    if let Some(a) = world.get_resource_mut::<AtlasTex>() {
        a.0 = Some(handle.clone());
    }
    Some(handle)
}

/// Point every material of a chunk mesh at the detail atlas (multiplies
/// over the coloured `base_color`).
fn apply_atlas(mesh: &mut MeshAsset, atlas: &Option<Handle<Texture>>) {
    if let Some(t) = atlas {
        for m in &mut mesh.materials {
            m.base_color_texture = Some(t.clone());
        }
    }
}

/// Generate chunked terrain: register each chunk mesh with the
/// `AssetServer` and spawn one entity per chunk (mesh + static trimesh
/// collider). Needs the `AssetServer` (present once the runner starts, or
/// inserted by a headless test). Returns the number of chunk entities.
pub fn generate_chunked_terrain(world: &mut World, cfg: &VoxelConfig) -> usize {
    let atlas = ensure_atlas(world);
    let chunk = cfg.chunk;
    // Chunk grid covering [-half, half) columns.
    let lo = (-cfg.half).div_euclid(chunk);
    let hi = (cfg.half - 1).div_euclid(chunk);
    let mut spawned = 0;
    for cx in lo..=hi {
        for cz in lo..=hi {
            let Some(mut mesh) = build_chunk_mesh(cx, cz, cfg) else {
                continue;
            };
            apply_atlas(&mut mesh, &atlas);
            // Register the procedural mesh -> handle (needs AssetServer).
            let handle = match world.get_resource::<AssetServer>() {
                Some(a) => a.register_mesh(mesh),
                None => return spawned, // headless-without-assets: skip terrain
            };
            let origin = DVec3::new((cx * chunk) as f64, 0.0, (cz * chunk) as f64);
            let e = world.spawn(SceneEntity::from_translation(origin));
            world.insert(e, Mesh3D::from_handle(handle));
            // Static trimesh collider built from the chunk mesh geometry.
            world.insert(
                e,
                Collider {
                    shape: soxide_engine::physics::Shape::Mesh { convex: false },
                    restitution: 0.0,
                    friction: 0.9,
                    sensor: false,
                    handle: None,
                },
            );
            spawned += 1;
        }
    }
    spawned
}

/// One-shot `Stage::PreUpdate` system: generate the world on the first
/// frame (once the runner has inserted the `AssetServer`), then latch off.
pub fn world_gen_tick(world: &mut World) {
    let done = world.get_resource::<GenState>().map(|g| g.done).unwrap_or(true);
    if done {
        return;
    }
    let cfg = *world
        .get_resource::<VoxelConfig>()
        .expect("VoxelConfig resource present");
    let palette = make_palette(world);

    let (terrain_desc, chunks) = match cfg.terrain {
        Terrain::PerVoxel => {
            let mut stats = GenStats::default();
            generate_terrain_voxels(world, &palette, &cfg, &mut stats);
            spawn_life(world, &palette, &cfg, &mut stats);
            (format!("{} voxel entities", stats.blocks), 0)
        }
        Terrain::Chunked => {
            let chunks = generate_chunked_terrain(world, &cfg);
            let mut stats = GenStats::default();
            spawn_life(world, &palette, &cfg, &mut stats);
            (format!("{chunks} chunk meshes"), chunks)
        }
    };
    let _ = chunks;

    if let Some(g) = world.get_resource_mut::<GenState>() {
        g.done = true;
    }
    let entities = world.iter_entities().count();
    eprintln!(
        "voxel-game: terrain = {terrain_desc}; {} live entities total in the ECS",
        entities
    );
}

// ---------------------------------------------------------------------------
// First-person player, infinite chunk streaming, and block editing
// ---------------------------------------------------------------------------

/// First-person player: a gravity-driven `CharacterMover` pawn plus a
/// mouse-look camera that follows it at eye height. Driven by
/// [`player_control_tick`].
#[derive(Clone, Copy, Debug)]
pub struct Player {
    /// The pawn entity (has the `CharacterMover`); `None` until spawned.
    pub pawn: Option<Entity>,
    /// Last known pawn position (used by streaming).
    pub pos: DVec3,
    pub yaw: f64,
    pub pitch: f64,
    pub sensitivity: f64,
    /// Camera height above the pawn origin.
    pub eye: f64,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            pawn: None,
            pos: DVec3::new(0.0, 45.0, 0.0),
            yaw: 0.0,
            pitch: -0.15,
            sensitivity: 0.0025,
            eye: 1.5,
        }
    }
}

impl Player {
    /// Full look rotation (yaw + pitch) for the camera.
    fn look_rot(&self) -> DQuat {
        DQuat::from_rotation_y(self.yaw) * DQuat::from_rotation_x(self.pitch)
    }
    /// Look direction (with pitch) — used for aiming block edits.
    fn look_dir(&self) -> DVec3 {
        self.look_rot() * DVec3::NEG_Z
    }
    /// Horizontal (yaw-only) forward / right for walking.
    fn walk_axes(&self) -> (DVec3, DVec3) {
        let yr = DQuat::from_rotation_y(self.yaw);
        (yr * DVec3::NEG_Z, yr * DVec3::X)
    }
}

/// Tracks streamed chunks. Each loaded chunk keeps its mesh `Handle` so the
/// registered `MeshAsset` can be freed on unload (see the TODO in
/// [`chunk_stream_tick`]). `empty` remembers all-air chunks so they aren't
/// rebuilt every frame.
#[derive(Default)]
pub struct ChunkManager {
    loaded: std::collections::HashMap<(i32, i32), (Entity, Handle<MeshAsset>)>,
    empty: std::collections::HashSet<(i32, i32)>,
    pub radius: i32,
}

/// `Stage::Update` system: mouse-look + WASD drive the player pawn's
/// `CharacterMover` (gravity-walking, colliding with the terrain trimesh),
/// Space jumps, and left/right click carve / place a block. The camera
/// follows the pawn at eye height.
pub fn player_control_tick(world: &mut World) {
    // Snapshot input (immutable borrow) before mutating resources.
    let Some(input) = world.get_resource::<Input>() else {
        return;
    };
    let (mdx, mdy) = input.mouse_delta();
    let (w, s) = (input.pressed(KeyCode::W), input.pressed(KeyCode::S));
    let (a, d) = (input.pressed(KeyCode::A), input.pressed(KeyCode::D));
    let jump = input.just_pressed(KeyCode::Space);
    let carve = input.mouse_just_pressed(MouseButton::Left);
    let place = input.mouse_just_pressed(MouseButton::Right);
    let save_key = input.just_pressed(KeyCode::F5);

    let mut p = *world
        .get_resource::<Player>()
        .expect("Player resource present");
    p.yaw -= mdx * p.sensitivity;
    p.pitch = (p.pitch - mdy * p.sensitivity).clamp(-1.54, 1.54);

    // Horizontal move intent in the yaw frame.
    let (fwd, right) = p.walk_axes();
    let mut mv = fwd * ((w as i32 - s as i32) as f64) + right * ((d as i32 - a as i32) as f64);
    if mv.length_squared() > 1e-6 {
        mv = mv.normalize();
    } else {
        mv = DVec3::ZERO;
    }

    // Drive the pawn's mover.
    let pawn = p.pawn;
    if let Some(pawn) = pawn {
        if let Some(m) = world.get_mut::<CharacterMover>(pawn) {
            m.intent.dir = mv;
            m.intent.jump = jump;
        }
        if let Some(se) = world.get::<SceneEntity>(pawn) {
            p.pos = se.transform.translation;
        }
    }

    let cam_pos = p.pos + DVec3::new(0.0, p.eye, 0.0);
    let look_rot = p.look_rot();
    let look_dir = p.look_dir();
    *world.get_resource_mut::<Player>().unwrap() = p;

    // Camera follows the pawn (extract id first so the query borrow ends).
    let cam = world.query::<Camera3d>().next().map(|(e, _)| e);
    if let Some(cam) = cam {
        if let Some(se) = world.get_mut::<SceneEntity>(cam) {
            se.transform.translation = cam_pos;
            se.transform.rotation = look_rot;
        }
    }

    // Block editing: raymarch from the camera to the first solid voxel.
    if carve || place {
        let cfg = *world.get_resource::<VoxelConfig>().unwrap();
        let hit = {
            let edits = world.get_resource::<ChunkEdits>().unwrap();
            raymarch_voxel(cam_pos, look_dir, 7.0, &cfg, edits)
        };
        if let Some((solid_v, empty_v)) = hit {
            if let Some(e) = world.get_resource_mut::<ChunkEdits>() {
                if carve {
                    e.set(solid_v.0, solid_v.1, solid_v.2, false, cfg.chunk);
                } else if let Some(pv) = empty_v {
                    e.set(pv.0, pv.1, pv.2, true, cfg.chunk);
                }
            }
        }
    }

    // F5: save the world (edits + seed + player pose).
    if save_key {
        world_save_now(world);
    }
}

/// Step a ray through the voxel grid; return `(first_solid_voxel,
/// last_empty_voxel_before_it)` within `max_dist`, or `None`.
fn raymarch_voxel(
    origin: DVec3,
    dir: DVec3,
    max_dist: f64,
    cfg: &VoxelConfig,
    edits: &ChunkEdits,
) -> Option<((i32, i32, i32), Option<(i32, i32, i32)>)> {
    let step = 0.05;
    let n = (max_dist / step) as i32;
    let mut prev: Option<(i32, i32, i32)> = None;
    for i in 0..n {
        let p = origin + dir * (i as f64 * step);
        let v = (
            p.x.floor() as i32,
            p.y.floor() as i32,
            p.z.floor() as i32,
        );
        if Some(v) == prev {
            continue;
        }
        if solid_e(v.0, v.1, v.2, cfg.seed, cfg.max_height, edits) {
            return Some((v, prev));
        }
        prev = Some(v);
    }
    None
}

/// `Stage::Update` system: stream chunks in/out around the player and
/// rebuild any chunk whose blocks were edited. Needs the `AssetServer`.
pub fn chunk_stream_tick(world: &mut World) {
    let cfg = *world.get_resource::<VoxelConfig>().unwrap();
    let chunk = cfg.chunk;
    let atlas = ensure_atlas(world);
    let ppos = world.get_resource::<Player>().map(|p| p.pos).unwrap_or(DVec3::ZERO);
    let pcx = (ppos.x as i32).div_euclid(chunk);
    let pcz = (ppos.z as i32).div_euclid(chunk);

    // Own the manager so we can spawn/despawn on the world freely.
    let Some(mut mgr) = world.resources_mut().remove::<ChunkManager>() else {
        return;
    };
    let r = mgr.radius.max(1);

    // Desired chunk set.
    let mut desired = std::collections::HashSet::new();
    for cx in (pcx - r)..=(pcx + r) {
        for cz in (pcz - r)..=(pcz + r) {
            desired.insert((cx, cz));
        }
    }

    // Helper: despawn a chunk entity and free its registered mesh so the
    // AssetServer doesn't grow without bound as the world streams. Relies on
    // `AssetServer::remove_mesh` (Soxide main).
    fn unload(world: &mut World, mgr: &mut ChunkManager, c: (i32, i32)) {
        if let Some((e, handle)) = mgr.loaded.remove(&c) {
            world.despawn(e);
            if let Some(a) = world.get_resource::<AssetServer>() {
                a.remove_mesh(&handle);
            }
        }
        mgr.empty.remove(&c);
    }

    // Rebuild edited chunks: drop them so they re-load fresh below.
    let dirty = world
        .get_resource_mut::<ChunkEdits>()
        .map(|e| e.take_dirty())
        .unwrap_or_default();
    for c in dirty {
        unload(world, &mut mgr, c);
    }

    // Unload chunks well outside the radius (hysteresis: keep a 2-chunk
    // margin so crossing a boundary back and forth doesn't thrash).
    let keep = r + 2;
    let to_unload: Vec<(i32, i32)> = mgr
        .loaded
        .keys()
        .copied()
        .filter(|(cx, cz)| (cx - pcx).abs() > keep || (cz - pcz).abs() > keep)
        .collect();
    for c in to_unload {
        unload(world, &mut mgr, c);
    }

    // Load missing chunks (bounded per frame to avoid hitches).
    let mut budget = 6;
    for (cx, cz) in desired {
        if mgr.loaded.contains_key(&(cx, cz)) || mgr.empty.contains(&(cx, cz)) {
            continue;
        }
        if budget == 0 {
            break;
        }
        let mesh = {
            let edits = world.get_resource::<ChunkEdits>().unwrap();
            build_chunk_mesh_with(cx, cz, &cfg, edits)
        };
        let Some(mut mesh) = mesh else {
            mgr.empty.insert((cx, cz)); // all air — don't retry every frame
            continue;
        };
        apply_atlas(&mut mesh, &atlas);
        let Some(handle) = world
            .get_resource::<AssetServer>()
            .map(|a| a.register_mesh(mesh))
        else {
            break; // no AssetServer yet
        };
        let origin = DVec3::new((cx * chunk) as f64, 0.0, (cz * chunk) as f64);
        let e = world.spawn(SceneEntity::from_translation(origin));
        world.insert(e, Mesh3D::from_handle(handle.clone()));
        world.insert(
            e,
            Collider {
                shape: soxide_engine::physics::Shape::Mesh { convex: false },
                restitution: 0.0,
                friction: 0.9,
                sensor: false,
                handle: None,
            },
        );
        mgr.loaded.insert((cx, cz), (e, handle));
        budget -= 1;
    }

    world.insert_resource(mgr);
}

/// One-shot: spawn the orbs + physics-corner landmarks near origin once the
/// AssetServer exists. Creatures are handled by [`creature_stream_tick`], so
/// this spawns the population with `creatures = 0`.
pub fn life_init_tick(world: &mut World) {
    let done = world.get_resource::<GenState>().map(|g| g.done).unwrap_or(true);
    if done || world.get_resource::<AssetServer>().is_none() {
        return;
    }
    let mut cfg = *world.get_resource::<VoxelConfig>().unwrap();
    cfg.creatures = 0; // creatures stream around the player instead
    let pal = make_palette(world);
    let mut stats = GenStats::default();
    spawn_life(world, &pal, &cfg, &mut stats);
    if let Some(g) = world.get_resource_mut::<GenState>() {
        g.done = true;
    }
}

/// Cached creature materials (built once) + a spawn counter, so
/// [`creature_stream_tick`] never re-registers materials each frame.
#[derive(Default)]
pub struct SpawnAssets {
    creatures: Vec<Option<MaterialHandle>>,
    ready: bool,
    counter: u64,
}

/// `Stage::Update` system: keep a roaming creature population around the
/// player — despawn creatures that fall far behind, spawn new ones ahead —
/// so an infinite world stays alive without unbounded entity growth.
pub fn creature_stream_tick(world: &mut World) {
    let cfg = *world.get_resource::<VoxelConfig>().unwrap();
    let (seed, max_h) = (cfg.seed, cfg.max_height);

    // Build creature materials once (needs the AssetServer).
    {
        let ready = world.get_resource::<SpawnAssets>().map(|s| s.ready).unwrap_or(true);
        if !ready {
            let mats = world.get_resource::<AssetServer>().map(|a| {
                let m = |rgba: [f32; 4]| Some(a.register_material(Material::default().with_base_color(rgba)));
                vec![m([0.90, 0.30, 0.30, 1.0]), m([0.30, 0.55, 0.95, 1.0]), m([0.85, 0.50, 0.90, 1.0])]
            });
            if let Some(mats) = mats {
                if let Some(s) = world.get_resource_mut::<SpawnAssets>() {
                    s.creatures = mats;
                    s.ready = true;
                }
            }
        }
    }
    let Some(sa) = world.get_resource::<SpawnAssets>() else {
        return;
    };
    if !sa.ready {
        return;
    }
    let mats = sa.creatures.clone();
    let mut counter = sa.counter;

    let ppos = world.get_resource::<Player>().map(|p| p.pos).unwrap_or(DVec3::ZERO);
    let hdist = |p: DVec3| ((p.x - ppos.x).powi(2) + (p.z - ppos.z).powi(2)).sqrt();
    let (spawn_r, despawn_r) = (60.0_f64, 95.0_f64);
    let target = cfg.creatures.min(220);

    // Despawn creatures that drifted too far behind the player.
    let creatures: Vec<(Entity, DVec3)> = world
        .query::<SteerAgent>()
        .map(|(e, _)| (e, DVec3::ZERO))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(e, _)| {
            let p = world.get::<SceneEntity>(e).map(|s| s.transform.translation).unwrap_or(ppos);
            (e, p)
        })
        .collect();
    let mut near = 0usize;
    for (e, p) in &creatures {
        if hdist(*p) > despawn_r {
            world.despawn(*e);
        } else if hdist(*p) <= spawn_r {
            near += 1;
        }
    }

    // Spawn new creatures within spawn radius to top up the population.
    let mut budget = 8;
    while near < target && budget > 0 {
        let a = rand01(counter as i32, 1, seed) * std::f64::consts::TAU;
        let r = 8.0 + rand01(counter as i32, 2, seed) * (spawn_r - 8.0);
        let cx = ppos.x + r * a.cos();
        let cz = ppos.z + r * a.sin();
        let surf = height_at(cx.round() as i32, cz.round() as i32, seed, max_h);
        let cy = surf as f64 + 1.5;
        let mat = if mats.is_empty() {
            None
        } else {
            mats[(counter as usize) % mats.len()].clone()
        };
        let e = world.spawn(SceneEntity::new(Transform {
            translation: DVec3::new(cx, cy, cz),
            scale: DVec3::splat(0.6),
            ..Default::default()
        }));
        world.insert(e, cube_mesh(&mat));
        world.insert(
            e,
            CharacterMover {
                mode: "flying".to_string(),
                settings: MoverSettings {
                    gravity_scale: 0.0,
                    max_speed: 3.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let mut steer = SteerAgent::default();
        steer.mode = SteerMode::Wander;
        steer.speed_scale = 0.85;
        steer.wander_turn_deg = 55.0;
        world.insert(e, steer);
        near += 1;
        counter = counter.wrapping_add(1);
        budget -= 1;
    }
    if let Some(s) = world.get_resource_mut::<SpawnAssets>() {
        s.counter = counter;
    }
}

// ---------------------------------------------------------------------------
// World persistence (save the edits + seed + player; terrain is procedural)
// ---------------------------------------------------------------------------

/// Where the world is saved/loaded. A plain-text format — only the edits
/// (carved/placed blocks), the seed, and the player pose need saving; the
/// base terrain is regenerated from the seed.
pub struct SavePath(pub std::path::PathBuf);

/// One-shot latch so the save file is loaded exactly once, before streaming.
#[derive(Default)]
pub struct WorldLoaded(pub bool);

/// Parsed save file.
pub struct WorldSave {
    pub seed: u32,
    pub player: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub edits: Vec<((i32, i32, i32), bool)>,
}

/// Serialise a world to the plain-text save format.
pub fn save_world(path: &std::path::Path, save: &WorldSave) -> std::io::Result<()> {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "seed {}", save.seed);
    let _ = writeln!(
        s,
        "player {} {} {} {} {}",
        save.player[0], save.player[1], save.player[2], save.yaw, save.pitch
    );
    for ((x, y, z), solid) in &save.edits {
        let _ = writeln!(s, "e {} {} {} {}", x, y, z, if *solid { 1 } else { 0 });
    }
    std::fs::write(path, s)
}

/// Parse a save file, or `None` if it doesn't exist / is unreadable.
pub fn load_world(path: &std::path::Path) -> Option<WorldSave> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut save = WorldSave {
        seed: 0,
        player: [0.0, 30.0, 0.0],
        yaw: 0.0,
        pitch: 0.0,
        edits: Vec::new(),
    };
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["seed", s] => save.seed = s.parse().unwrap_or(0),
            ["player", x, y, z, yaw, pitch] => {
                save.player = [
                    x.parse().unwrap_or(0.0),
                    y.parse().unwrap_or(30.0),
                    z.parse().unwrap_or(0.0),
                ];
                save.yaw = yaw.parse().unwrap_or(0.0);
                save.pitch = pitch.parse().unwrap_or(0.0);
            }
            ["e", x, y, z, s] => {
                if let (Ok(x), Ok(y), Ok(z)) = (x.parse(), y.parse(), z.parse()) {
                    save.edits.push(((x, y, z), *s == "1"));
                }
            }
            _ => {}
        }
    }
    Some(save)
}

/// Gather the current world state and write it to the save path.
pub fn world_save_now(world: &World) {
    let Some(path) = world.get_resource::<SavePath>().map(|p| p.0.clone()) else {
        return;
    };
    let seed = world.get_resource::<VoxelConfig>().map(|c| c.seed).unwrap_or(0);
    let p = world.get_resource::<Player>().copied().unwrap_or_default();
    let edits = world
        .get_resource::<ChunkEdits>()
        .map(|e| e.entries())
        .unwrap_or_default();
    let save = WorldSave {
        seed,
        player: [p.pos.x, p.pos.y, p.pos.z],
        yaw: p.yaw,
        pitch: p.pitch,
        edits,
    };
    if let Err(e) = save_world(&path, &save) {
        eprintln!("voxel-game: save failed: {e}");
    } else {
        eprintln!("voxel-game: saved {} edits to {}", save.edits.len(), path.display());
    }
}

/// One-shot `Stage::PreUpdate` system: load the save file (if any) into the
/// edits + seed + player before the first chunk streams in.
pub fn world_load_tick(world: &mut World) {
    if world.get_resource::<WorldLoaded>().map(|w| w.0).unwrap_or(true) {
        return;
    }
    if let Some(w) = world.get_resource_mut::<WorldLoaded>() {
        w.0 = true;
    }
    let Some(path) = world.get_resource::<SavePath>().map(|p| p.0.clone()) else {
        return;
    };
    let Some(save) = load_world(&path) else {
        return;
    };
    if let Some(cfg) = world.get_resource_mut::<VoxelConfig>() {
        cfg.seed = save.seed;
    }
    if let Some(e) = world.get_resource_mut::<ChunkEdits>() {
        for ((x, y, z), s) in &save.edits {
            e.insert_loaded(*x, *y, *z, *s);
        }
    }
    // Restore the player pose (pawn position + look).
    let pawn = world.get_resource::<Player>().and_then(|p| p.pawn);
    if let Some(p) = world.get_resource_mut::<Player>() {
        p.pos = DVec3::new(save.player[0], save.player[1], save.player[2]);
        p.yaw = save.yaw;
        p.pitch = save.pitch;
    }
    if let Some(pawn) = pawn {
        if let Some(se) = world.get_mut::<SceneEntity>(pawn) {
            se.transform.translation = DVec3::new(save.player[0], save.player[1], save.player[2]);
        }
    }
    eprintln!("voxel-game: loaded {} edits from save", save.edits.len());
}

/// Build a **playable** app: infinite streamed chunks, a first-person
/// gravity-walking player (WASD + mouse, Space to jump, click to
/// carve/place blocks, F5 to save), plus the creature/orb population. This
/// is what the game binary runs.
pub fn build_streaming_app() -> App {
    let mut app = App::new();
    install_voxel_game(&mut app);
    app
}

/// Install the whole voxel game onto an existing `App` — systems, resources,
/// the player pawn, camera and lights. Used both by [`build_streaming_app`]
/// (the standalone binary) and by the `sox_plugin_build` entry point, so the
/// exact same game runs inside the editor as a full-`App` plugin.
pub fn install_voxel_game(app: &mut App) {
    let mut cfg = VoxelConfig::default();
    cfg.terrain = Terrain::Chunked;
    app.insert_resource(cfg);
    app.insert_resource(ChunkEdits::default());
    app.insert_resource(GenState { done: false });
    app.insert_resource(OrbClock::default());
    app.insert_resource(ChunkManager {
        loaded: Default::default(),
        empty: Default::default(),
        radius: 5,
    });

    // Player pawn: a gravity-driven CharacterMover that walks/collides on
    // the terrain trimesh. Spawns high and falls onto the streamed ground.
    let spawn = DVec3::new(0.0, 60.0, 0.0);
    let pawn = app.world.spawn(SceneEntity::from_translation(spawn));
    app.world.insert(
        pawn,
        CharacterMover {
            mode: "walking".to_string(),
            settings: MoverSettings {
                max_speed: 9.0,
                jump_speed: 7.0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let mut player = Player::default();
    player.pawn = Some(pawn);
    player.pos = spawn;
    app.insert_resource(player);

    app.insert_resource(SpawnAssets::default());
    app.insert_resource(AtlasTex::default());
    app.insert_resource(SavePath(std::path::PathBuf::from("voxel_world.save")));
    app.insert_resource(WorldLoaded(false));
    // Load a saved world (if any) before the first chunk streams in.
    app.add_system(Stage::PreUpdate, world_load_tick);
    app.add_system(Stage::Update, player_control_tick);
    app.add_system(Stage::Update, chunk_stream_tick);
    app.add_system(Stage::Update, life_init_tick);
    app.add_system(Stage::Update, creature_stream_tick);
    app.add_system(Stage::Update, orbs_tick);

    // Camera (driven by player_control_tick) + lights.
    let cam = app.world.spawn(SceneEntity::default());
    app.world.insert(cam, Camera3d::default());
    app.world.spawn(DirectionalLight {
        direction: DVec3::new(-0.5, -1.0, -0.35),
        color: [1.0, 0.97, 0.9],
        intensity: 3.0,
        ..Default::default()
    });
    app.world.spawn(AmbientLight {
        color: [0.55, 0.65, 0.85],
        intensity: 0.30,
    });
}

// ---------------------------------------------------------------------------
// Visualisation: render-ready geometry (for the offscreen screenshot tool)
// ---------------------------------------------------------------------------

/// Render data decoupled from wgpu: coloured meshes (terrain chunks) and
/// coloured boxes (creatures / orbs / props), plus a camera. The
/// `voxel_shot` example turns these into draw calls.
pub mod viz {
    use super::*;

    /// A coloured triangle mesh in world space.
    pub struct VizMesh {
        pub positions: Vec<[f32; 3]>,
        pub normals: Vec<[f32; 3]>,
        pub uvs: Vec<[f32; 2]>,
        pub indices: Vec<u32>,
        pub color: [f32; 3],
    }

    /// A coloured axis-aligned box (rendered as a scaled cube).
    pub struct VizBox {
        pub center: [f32; 3],
        pub half: [f32; 3],
        pub color: [f32; 3],
    }

    pub struct VizScene {
        pub meshes: Vec<VizMesh>,
        pub boxes: Vec<VizBox>,
        pub cam_eye: [f32; 3],
        pub cam_target: [f32; 3],
    }

    /// [`viz_scene_ex`] with no edits and a default overhead camera.
    pub fn viz_scene(cfg: &VoxelConfig) -> VizScene {
        let eye = [0.0, cfg.half as f32 * 1.15, cfg.half as f32 * 1.75];
        viz_scene_ex(cfg, &ChunkEdits::default(), eye, [0.0, 6.0, 0.0])
    }

    /// Build a static render snapshot of the world (terrain as chunk meshes
    /// consulting `edits`, life as boxes) with an explicit camera. Pure —
    /// no ECS / AssetServer / GPU.
    pub fn viz_scene_ex(
        cfg: &VoxelConfig,
        edits: &ChunkEdits,
        eye: [f32; 3],
        target: [f32; 3],
    ) -> VizScene {
        let mut meshes = Vec::new();

        // Terrain: face-culled chunk meshes, split per material submesh.
        let chunk = cfg.chunk;
        let lo = (-cfg.half).div_euclid(chunk);
        let hi = (cfg.half - 1).div_euclid(chunk);
        for cx in lo..=hi {
            for cz in lo..=hi {
                let Some(m) = build_chunk_mesh_with(cx, cz, cfg, edits) else {
                    continue;
                };
                let (ox, oz) = ((cx * chunk) as f32, (cz * chunk) as f32);
                for sm in &m.submeshes {
                    let col = m.materials[sm.material_slot.unwrap_or(0) as usize].base_color;
                    let (start, end) = (
                        sm.index_start as usize,
                        (sm.index_start + sm.index_count) as usize,
                    );
                    let mut remap = std::collections::HashMap::new();
                    let mut positions = Vec::new();
                    let mut normals = Vec::new();
                    let mut uvs = Vec::new();
                    let mut indices = Vec::new();
                    for &idx in &m.indices[start..end] {
                        let ni = *remap.entry(idx).or_insert_with(|| {
                            let v = &m.vertices[idx as usize];
                            positions.push([v.position[0] + ox, v.position[1], v.position[2] + oz]);
                            normals.push(v.normal);
                            uvs.push(v.uv);
                            (positions.len() - 1) as u32
                        });
                        indices.push(ni);
                    }
                    meshes.push(VizMesh {
                        positions,
                        normals,
                        uvs,
                        indices,
                        color: [col[0], col[1], col[2]],
                    });
                }
            }
        }

        // Life: recompute the same spawn positions used by `spawn_life`.
        let mut boxes = Vec::new();
        let (half, seed, max_h) = (cfg.half, cfg.seed, cfg.max_height);
        let creature_cols = [[0.90, 0.30, 0.30], [0.30, 0.55, 0.95], [0.85, 0.50, 0.90]];
        for i in 0..cfg.creatures {
            let i = i as i32;
            let cx = (rand01(i, 7, seed) * 2.0 - 1.0) * (half as f64 * 0.9);
            let cz = (rand01(i, 13, seed) * 2.0 - 1.0) * (half as f64 * 0.9);
            let surf = height_at(cx.round() as i32, cz.round() as i32, seed, max_h);
            let cy = surf as f64 + 1.5;
            boxes.push(VizBox {
                center: [cx as f32, cy as f32, cz as f32],
                half: [0.3, 0.3, 0.3],
                color: creature_cols[(i as usize) % 3],
            });
        }
        for i in 0..cfg.orbs {
            let i = i as i32;
            let cx = (rand01(i, 31, seed) * 2.0 - 1.0) * (half as f64 * 0.85);
            let cz = (rand01(i, 37, seed) * 2.0 - 1.0) * (half as f64 * 0.85);
            let surf = height_at(cx.round() as i32, cz.round() as i32, seed, max_h);
            let by = surf as f64 + 3.0 + rand01(i, 41, seed) * 4.0;
            boxes.push(VizBox {
                center: [cx as f32, by as f32, cz as f32],
                half: [0.2, 0.2, 0.2],
                color: [0.95, 0.80, 0.35],
            });
        }
        // Physics corner platform + a few cubes (static snapshot).
        boxes.push(VizBox {
            center: [0.0, 16.0, 44.0],
            half: [6.0, 0.5, 6.0],
            color: [0.50, 0.50, 0.53],
        });
        for i in 0..cfg.dyn_cubes {
            let i = i as i32;
            let ox = 0.0 + (rand01(i, 101, seed) * 2.0 - 1.0) * 4.5;
            let oz = 44.0 + (rand01(i, 103, seed) * 2.0 - 1.0) * 4.5;
            let oy = 16.0 + 1.0 + (i as f64) * 0.5;
            boxes.push(VizBox {
                center: [ox as f32, oy as f32, oz as f32],
                half: [0.5, 0.5, 0.5],
                color: [0.95, 0.55, 0.15],
            });
        }

        VizScene {
            meshes,
            boxes,
            cam_eye: eye,
            cam_target: target,
        }
    }
}

// ---------------------------------------------------------------------------
// App assembly
// ---------------------------------------------------------------------------

/// Build a fully-wired `App`: camera, lights, the world-gen config, and the
/// generation + orb systems. Hand this to the desktop runner (which inserts
/// the `AssetServer` and drives the schedule) to play it.
pub fn build_app() -> App {
    build_app_with(VoxelConfig::default())
}

/// Like [`build_app`] but with an explicit config (used by tests to scale down).
pub fn build_app_with(cfg: VoxelConfig) -> App {
    let mut app = App::new();
    app.insert_resource(cfg);
    app.insert_resource(GenState { done: false });
    app.insert_resource(OrbClock::default());
    app.insert_resource(AtlasTex::default());
    app.add_system(Stage::PreUpdate, world_gen_tick);
    app.add_system(Stage::Update, orbs_tick);

    // Overview camera, angled down over the terrain.
    let eye = DVec3::new(0.0, 52.0, 82.0);
    let cam = app
        .world
        .spawn(SceneEntity::looking_at(eye, DVec3::new(0.0, 6.0, 0.0), DVec3::Y));
    app.world.insert(cam, Camera3d::default());

    // Sun + sky ambient.
    app.world.spawn(DirectionalLight {
        direction: DVec3::new(-0.5, -1.0, -0.35),
        color: [1.0, 0.97, 0.9],
        intensity: 3.0,
        ..Default::default()
    });
    app.world.spawn(AmbientLight {
        color: [0.55, 0.65, 0.85],
        intensity: 0.30,
    });

    app
}
