# The device UI as a desktop window against ink/mock-rado.mjs. Edit target\preview-list.json while it runs to
# change the list on the "laptop"; stop the mock to see the offline banner.
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)
$env:CARGO_TARGET_DIR = "$PWD\target"
$list = 'target\preview-list.json'
if (-not (Test-Path $list)) {
    New-Item -ItemType Directory -Force target | Out-Null
    Copy-Item fixtures\list.json $list
}
$mock = Start-Process node -ArgumentList 'mock-rado.mjs', $list, '7879' -PassThru -NoNewWindow
try {
    # The window's logical size is the device's (363 x 483); 2 makes it readable on a desktop monitor.
    if (-not $env:SLINT_SCALE_FACTOR) { $env:SLINT_SCALE_FACTOR = '2' }
    & ..\scripts\msvc.cmd cargo run -p ink-app -- preview.cfg
} finally {
    Stop-Process -Id $mock.Id -ErrorAction SilentlyContinue
}
