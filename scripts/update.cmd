@echo off
rem Builds the installer and updates the installed Rado in place: /UPDATE keeps the existing install
rem instead of uninstalling it first, /P runs without pages and closes a running Rado, /R restarts it.
rem It builds into target\app because Claude sessions holding rado-mcp keep target\release\rado-mcp.exe locked.
setlocal
cd /d "%~dp0.."
set "CARGO_TARGET_DIR=%CD%\src-tauri\target\app"
call scripts\msvc.cmd npx tauri build || exit /b 1
set "SETUP="
for /f "delims=" %%f in ('dir /b /o-d "%CARGO_TARGET_DIR%\release\bundle\nsis\Rado_*_x64-setup.exe"') do if not defined SETUP set "SETUP=%%f"
if not defined SETUP (echo No installer found under %CARGO_TARGET_DIR%\release\bundle\nsis & exit /b 1)
echo Updating Rado with %SETUP% ...
start "" /wait "%CARGO_TARGET_DIR%\release\bundle\nsis\%SETUP%" /P /UPDATE /R
if errorlevel 1 (echo The installer failed with exit code %errorlevel% & exit /b 1)
echo Rado is updated and running.
