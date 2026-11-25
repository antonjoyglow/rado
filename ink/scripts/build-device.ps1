# Cross-builds the PocketBook app to target\rado-ink.app: ARMv7, glibc 2.23, InkView SDK 6.10.
# Needs zig (CARGO_ZIGBUILD_ZIG_PATH, default C:\dev\tools\zig-x86_64-windows-0.16.0\zig.exe), cargo-zigbuild and
# the armv7-unknown-linux-gnueabi Rust target.
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)
$env:CARGO_TARGET_DIR = "$PWD\target"
if (-not $env:CARGO_ZIGBUILD_ZIG_PATH) { $env:CARGO_ZIGBUILD_ZIG_PATH = 'C:\dev\tools\zig-x86_64-windows-0.16.0\zig.exe' }
$zig = $env:CARGO_ZIGBUILD_ZIG_PATH
if (-not (Test-Path $zig)) { throw "zig not found at $zig; set CARGO_ZIGBUILD_ZIG_PATH" }

# Slint's system-font support links libfontconfig through fontique, which can't load it with dlopen. The build links
# against a stub with fontconfig's soname; the device's own libfontconfig.so.1 loads at run time.
$stub = (New-Item -ItemType Directory -Force target\fontconfig-stub).FullName
$meta = cargo metadata --format-version 1 --filter-platform armv7-unknown-linux-gnueabi | ConvertFrom-Json
$sys = $meta.packages | Where-Object name -eq 'yeslogic-fontconfig-sys' | Select-Object -First 1
if (-not $sys) { throw 'yeslogic-fontconfig-sys is no longer a dependency; drop the fontconfig stub' }
$names = Select-String -Path (Join-Path (Split-Path $sys.manifest_path) 'src\lib.rs') -Pattern '\bfn (Fc\w+)' -AllMatches |
    ForEach-Object { $_.Matches } | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique
Set-Content "$stub\stub.c" ($names | ForEach-Object { "void $_(void) {}" })
& $zig cc -target arm-linux-gnueabi.2.23 -shared "-Wl,-soname,libfontconfig.so.1" -o "$stub\libfontconfig.so" "$stub\stub.c"
if ($LASTEXITCODE) { throw 'zig cc failed on the fontconfig stub' }
# The pkg-config crate splits this output like a shell, which drops backslashes.
$libDir = $stub -replace '\\', '/'
Set-Content "$stub\pkg-config.cmd" @"
@echo off
echo %* | findstr /c:"--modversion" >nul && (echo 2.13.1& exit /b 0)
echo %* | findstr /c:"--libs" >nul && (echo -L$libDir -lfontconfig& exit /b 0)
exit /b 0
"@
$env:PKG_CONFIG = "$stub\pkg-config.cmd"
$env:PKG_CONFIG_ALLOW_CROSS = '1'

cargo zigbuild --release --target armv7-unknown-linux-gnueabi.2.23 -p ink-app --no-default-features --features device
if ($LASTEXITCODE) { throw 'cargo zigbuild failed' }

# zig's linker refuses -z execstack, which the PocketBook loader expects; set PF_X on PT_GNU_STACK as execstack -s does.
$app = 'target\rado-ink.app'
Copy-Item target\armv7-unknown-linux-gnueabi\release\rado-ink $app -Force
node scripts\execstack.mjs $app
if ($LASTEXITCODE) { throw 'execstack patch failed' }
Get-Item $app | Select-Object FullName, Length
