@echo off
setlocal
REM Builds the winmm.dll proxy that loads Rustic-Isles.dll (restricted\native\runtime-loader), for the
REM Windows target dainty-host publish reads it from.
pushd "%~dp0..\..\..\..\native\runtime-loader"
if errorlevel 1 exit /b 1
cargo build --release --target x86_64-pc-windows-msvc
if errorlevel 1 (
  popd
  exit /b 1
)
if not exist "target\x86_64-pc-windows-msvc\release\winmm.dll" (
  echo target\x86_64-pc-windows-msvc\release\winmm.dll was not produced.
  popd
  exit /b 1
)
popd
exit /b 0
