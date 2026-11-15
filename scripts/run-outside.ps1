param([Parameter(Mandatory)][string]$Exe, [string]$Arguments = '')
# Runs $Exe through WMI, waits for it and exits with its exit code. A process started from the Claude
# desktop app (an MSIX package) sees %LOCALAPPDATA% and %APPDATA% redirected into the app's private
# folder, so an installer started from a Claude session would install there instead of into the real
# %LOCALAPPDATA%\Rado. WMI starts the process outside the package. Its exit code comes back through a
# file beside $Exe, which is outside the redirected folders.
$exitFile = "$Exe.exit"
Remove-Item -LiteralPath $exitFile -ErrorAction SilentlyContinue
$cmd = "cmd.exe /v:on /c start `"`" /wait `"$Exe`" $Arguments & echo !errorlevel!> `"$exitFile`""
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = $cmd }
if ($r.ReturnValue -ne 0) { Write-Error "Could not start $Exe through WMI (error $($r.ReturnValue))"; exit 1 }
while (Get-Process -Id $r.ProcessId -ErrorAction SilentlyContinue) { Start-Sleep -Milliseconds 500 }
if (-not (Test-Path -LiteralPath $exitFile)) { Write-Error "$Exe left no exit code"; exit 1 }
$code = [int](Get-Content -LiteralPath $exitFile -Raw).Trim()
Remove-Item -LiteralPath $exitFile -ErrorAction SilentlyContinue
exit $code
