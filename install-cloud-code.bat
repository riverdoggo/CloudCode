@echo off
setlocal

set ROOT=%~dp0

where cargo >nul 2>nul
if errorlevel 1 (
  echo Rust cargo was not found in PATH.
  exit /b 1
)

echo Installing Cloud Code CLI to cargo bin...
pushd "%ROOT%rust"
cargo install --path crates/claw-cli --force
set EXIT_CODE=%errorlevel%
popd

if not "%EXIT_CODE%"=="0" (
  echo Installation failed.
  exit /b %EXIT_CODE%
)

echo.
echo Cloud Code installed.
echo Open a NEW terminal and run:
echo   cloud-code --version
echo   cloud-code
exit /b 0
