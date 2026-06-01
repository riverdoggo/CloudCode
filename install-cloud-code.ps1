$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $MyInvocation.MyCommand.Path

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "Rust cargo was not found in PATH."
}

Push-Location (Join-Path $root "rust")
try {
    cargo install --path crates/claw-cli --force
}
finally {
    Pop-Location
}

Write-Host ""
Write-Host "Cloud Code installed."
Write-Host "Open a NEW terminal and run:"
Write-Host "  cloud-code --version"
Write-Host "  cloud-code"
