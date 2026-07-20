//! Verifies the full-`App` plugin path end to end, exactly the way the
//! editor/runner load it: dlopen the compiled cdylib, check the ABI-version
//! probe, then call `sox_plugin_build(&mut App)` against a live App and drive
//! the schedule. This exercises the real dylib boundary (two copies of
//! `soxide-engine` sharing types by TypeId), so if it passes, the game runs
//! in the editor.
#![allow(clippy::unwrap_used)]

use libloading::{Library, Symbol};
use soxide_engine::App;
use soxide_engine::asset::{
    AssetServer, ColorSpace, MeshAsset, MeshDecoder, MeshLoadContext, NullWatcher, Texture,
    TextureDecoder,
};
use soxide_engine::core::{SoxError, SoxResult, Time};
use soxide_engine::physics::CharacterMover;
use soxide_engine::render::{Camera3d, Mesh3D};
use std::ffi::{c_void, CStr};
use std::os::raw::c_char;
use std::sync::Arc;

fn headless_assets() -> AssetServer {
    struct NoTex;
    impl TextureDecoder for NoTex {
        fn decode(&self, _b: &[u8]) -> SoxResult<Texture> {
            Ok(Texture { width: 1, height: 1, rgba: vec![255; 4], color_space: ColorSpace::Srgb, hdr: None })
        }
    }
    struct NoMesh;
    impl MeshDecoder for NoMesh {
        fn decode(&self, _b: &[u8], _c: &MeshLoadContext<'_>) -> SoxResult<MeshAsset> {
            Err(SoxError::other("headless"))
        }
    }
    AssetServer::new(
        std::env::temp_dir().join("voxel-plugin-test"),
        Arc::new(NoTex),
        Arc::new(NoMesh),
        Box::new(NullWatcher::default()),
    )
}

#[test]
fn full_app_plugin_loads_and_runs_like_the_editor() {
    let dylib = format!("{}/target/debug/libsausage_playground.so", env!("CARGO_MANIFEST_DIR"));
    unsafe {
        let lib = Library::new(&dylib).expect("dlopen the compiled plugin cdylib");

        // 1) ABI-version probe must match the host — the runner's gate.
        let ver: Symbol<unsafe extern "C" fn() -> *const c_char> =
            lib.get(b"sox_plugin_abi_version").expect("sox_plugin_abi_version symbol");
        let plugin_ver = CStr::from_ptr(ver()).to_str().unwrap();
        assert_eq!(
            plugin_ver,
            soxide_engine::plugin_abi_version(),
            "plugin ABI version must match the host"
        );

        // 2) Full-`App` build entry point (Rust ABI): App passed as *mut c_void.
        let build: Symbol<unsafe extern "Rust" fn(*mut c_void)> =
            lib.get(b"sox_plugin_build").expect("sox_plugin_build symbol");

        let mut app = App::new();
        build(&mut app as *mut App as *mut c_void);

        // 3) The plugin mutated OUR App across the dylib boundary.
        assert!(
            app.world.get_resource::<sausage_playground::Player>().is_some(),
            "plugin inserted the Player resource"
        );
        assert!(app.world.query::<Camera3d>().count() >= 1, "plugin spawned a camera");
        assert!(app.world.query::<CharacterMover>().count() >= 1, "plugin spawned the player pawn");

        // 4) Drive the schedule with a headless AssetServer: the plugin's
        //    streaming systems generate chunk meshes — proving resources the
        //    TEST inserts are found by the PLUGIN's systems (TypeId parity).
        app.world.insert_resource(headless_assets());
        app.world.resources_mut().remove::<Time>();
        for _ in 0..8 {
            app.schedule.run(&mut app.world);
        }
        let chunks = app.world.query::<Mesh3D>().filter(|(_, m)| m.handle.is_some()).count();
        assert!(chunks > 0, "plugin systems streamed chunk meshes across the boundary");

        drop(app);
        drop(lib);
    }
}
