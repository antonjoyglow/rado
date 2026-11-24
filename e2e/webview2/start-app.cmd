@echo off
rem Starts Rado with tauri dev for npm run e2e:webview2: its own identifier (own data folder and
rem single-instance key, so an installed Rado is untouched), a throwaway Credential Manager service
rem (no real token is read or written) and WebView2's DevTools protocol on port 9222.
setlocal
cd /d "%~dp0..\.."
set "RADO_KEYRING_SERVICE=rado-e2e-throwaway"
set "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222"
call scripts\msvc.cmd npx tauri dev --config e2e\webview2\tauri.e2e.conf.json
