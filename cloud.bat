@echo off
setlocal

set ROOT=%~dp0
set CLOUD_CODE_WORKSPACE_ROOT=%ROOT%

where cloud-code >nul 2>nul
if not errorlevel 1 (
  cloud-code %*
  exit /b %errorlevel%
)

where cargo >nul 2>nul
if errorlevel 1 (
  echo cloud-code is not installed and Rust cargo was not found.
  echo Run install-cloud-code.bat first.
  exit /b 1
)

echo Launching Cloud Code terminal from local source...
pushd "%ROOT%rust"
cargo run -p claw-cli --bin cloud-code -- %*
set EXIT_CODE=%errorlevel%
popd
exit /b %EXIT_CODE%
