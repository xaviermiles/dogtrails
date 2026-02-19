# Development script: runs trunk watch (frontend) and cargo watch (backend) in parallel.
# Install prerequisites once:
#   cargo install trunk cargo-watch

$ErrorActionPreference = "Stop"

$frontend = Start-Process -PassThru -NoNewWindow -FilePath "trunk" `
    -ArgumentList "watch" `
    -WorkingDirectory "$PSScriptRoot\frontend"

Start-Sleep -Seconds 1

$backend = Start-Process -PassThru -NoNewWindow -FilePath "cargo" `
    -ArgumentList "watch -x run -w src -w Cargo.toml -d 2" `
    -WorkingDirectory $PSScriptRoot

Write-Host ""
Write-Host "Dev servers running (Ctrl+C to stop)"
Write-Host "  Frontend:  trunk watch  (rebuilds on frontend/ changes)"
Write-Host "  Backend:   cargo watch  (restarts on src/ changes)"
Write-Host "  Open:      http://127.0.0.1:3000"
Write-Host ""

try {
    while (!$frontend.HasExited -and !$backend.HasExited) {
        Start-Sleep -Milliseconds 500
    }
} finally {
    if (!$frontend.HasExited) { Stop-Process $frontend -Force }
    if (!$backend.HasExited) { Stop-Process $backend -Force }
}
