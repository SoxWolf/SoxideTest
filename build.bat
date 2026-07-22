@echo off
REM ============================================================
REM  Build the Voxel World game plugin (+ the standalone binary).
REM
REM  The editor loads  target\debug\sausage_playground.dll  via
REM  plugins\voxel\voxel.soxplugin, so run this BEFORE opening
REM  voxel_world.soxproj in the editor (and again after any code
REM  change, then use the editor's "Reload" on the plugin).
REM ============================================================
setlocal
cd /d "%~dp0"

echo Building Voxel World (debug: plugin cdylib + bin)...
cargo build
if errorlevel 1 (
    echo.
    echo *** Build FAILED ***
    exit /b 1
)

echo.
echo Build OK.

REM The editor loads exactly this file; if it's missing, the Plugins tab
REM shows "loaded: 0". Confirm it exists so a silent path problem can't hide.
if exist "target\debug\sausage_playground.dll" (
    echo   Plugin dylib PRESENT: target\debug\sausage_playground.dll
) else (
    echo   *** Plugin dylib MISSING: target\debug\sausage_playground.dll ***
    echo       The editor will show "loaded: 0". Check [lib] crate-type = cdylib in Cargo.toml.
)
echo.
echo   Play standalone:  cargo run --release
echo   Play in editor:   open voxel_world.soxproj, then press Play (then Plugins ^> Reload all)
echo.
echo   IMPORTANT: the editor and this plugin must be built from the SAME engine
echo   revision AND the SAME rustc. If the editor's Console shows an "ABI mismatch"
echo   line, rebuild whichever side is stale so both strings match.
endlocal
