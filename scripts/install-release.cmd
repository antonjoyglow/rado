@echo off
rem Installs or updates Rado to a published release: downloads that exact installer from GitHub and runs it
rem in place (keeps settings and tokens, closes Rado, starts it again). Without an argument it asks for the
rem version, and Enter takes the latest release. Example: install-release.cmd 0.12.0
rem Double-click it from Explorer or run it from a terminal, not from a Claude desktop session.
setlocal
set "VERSION=%~1"
if defined VERSION goto :have_version
echo Looking up the latest release (a few seconds)...
for /f "delims=" %%v in ('powershell -NoProfile -Command "try { (Invoke-RestMethod -TimeoutSec 15 https://api.github.com/repos/antonjoyglow/rado/releases/latest).tag_name } catch {}"') do set "VERSION=%%v"
if defined VERSION (set /p "VERSION=Rado version to install (Enter for the latest, %VERSION%): ") else set /p "VERSION=Rado version to install: "
if not defined VERSION (echo No version given. & pause & exit /b 1)
:have_version
if /i "%VERSION:~0,1%"=="v" set "VERSION=%VERSION:~1%"
set "URL=https://github.com/antonjoyglow/rado/releases/download/v%VERSION%/Rado_%VERSION%_x64-setup.exe"
set "SETUP=%TEMP%\Rado_%VERSION%_x64-setup.exe"
echo Downloading %URL%
curl.exe -fL --retry 2 -o "%SETUP%" "%URL%"
if errorlevel 1 (echo The download failed. & pause & exit /b 1)
echo Installing Rado %VERSION%...
"%SETUP%" /P /UPDATE /R
if errorlevel 1 (echo The installer failed with exit code %errorlevel%. & pause & exit /b 1)
powershell -NoProfile -Command "'Rado version now: ' + (Get-Item \"$env:LOCALAPPDATA\Rado\rado.exe\").VersionInfo.ProductVersion"
pause
