param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$AgentArgs
)

$ErrorActionPreference = "Stop"

Write-Host "Starting agent development bootstrap..."

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "Rust toolchain is required. Install rustup first."
}

if ((-not (Test-Path ".agent/config.toml")) -and (Test-Path ".agent/config.toml.example")) {
    Copy-Item ".agent/config.toml.example" ".agent/config.toml"
    Write-Host "Created .agent/config.toml from example."
}

Write-Host "Building Rust workspace..."
Push-Location rust
cargo build --workspace
Pop-Location

Write-Host "Launching CLI..."
cargo run -p claw-cli --bin cloud-code -- @AgentArgs
