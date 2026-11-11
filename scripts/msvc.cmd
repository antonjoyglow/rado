@echo off
rem Runs a command inside the VS Build Tools environment. Rust otherwise picks the
rem VS Community install on this machine, which lacks the C++ libraries (msvcrt.lib).
call "C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
%*
