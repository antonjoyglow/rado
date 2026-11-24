@echo off
rem Installs or updates Rado to a published release, e.g. install-release.cmd 0.11.0: downloads that exact
rem installer from GitHub and runs it in place (keeps settings and tokens, closes Rado, starts it again).
rem Double-click it from Explorer or run it from a terminal, not from a Claude desktop session.
setlocal
set "VERSION=%~1"
if not defined VERSION set /p "VERSION=Rado version to install (e.g. 0.11.0): "
if not defined VERSION (echo No version given. & pause & exit /b 1)
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
