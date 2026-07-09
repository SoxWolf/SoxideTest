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
echo   Plugin dylib: target\debug\sausage_playground.dll
echo   Play standalone:  cargo run --release
echo   Play in editor:   open voxel_world.soxproj, then press Play
endlocal
