param([Parameter(Mandatory)][string]$Exe, [string]$Arguments = '')
# Runs $Exe through WMI, waits for it and exits with its exit code, or with 99 when WMI can't start it.
# A process started from the Claude desktop app (an MSIX package) sees %LOCALAPPDATA% and %APPDATA%
# redirected into the app's private folder, so an installer started from a Claude session would install
# there instead of into the real %LOCALAPPDATA%\Rado. WMI starts the process outside the package.
$ErrorActionPreference = 'Stop'
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = "`"$Exe`" $Arguments" }
if ($r.ReturnValue -ne 0) {
    [Console]::Error.WriteLine("Could not start $Exe through WMI (Win32_Process.Create returned $($r.ReturnValue))")
    exit 99
}
$p = Get-Process -Id $r.ProcessId
$null = $p.Handle  # opens the process now, so its exit code stays readable after it exits
$p.WaitForExit()
exit $p.ExitCode
