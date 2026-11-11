param(
    # The relay keys are the backend's: P2P_CLOUDFLARE_TURN_* for Cloudflare, P2P_TURN_REST_* for turn-rest.
    [Parameter(Mandatory = $true)]
    [string]$EnvFile,
    [ValidateSet("cloudflare", "turn-rest")]
    [string]$Provider = "cloudflare"
)

# Two ICE links on this PC, relay only, through a real relay, run by the MIT transport's ignored `relay_file` test
# (spec-paradox-transport C4). Always deletes the credential file. Prints no secret.
$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = (Resolve-Path (Join-Path $ProjectRoot "..\..\..")).Path
$EnvFile = (Resolve-Path -LiteralPath $EnvFile).Path
$RelayFile = Join-Path ([System.IO.Path]::GetTempPath()) "rifts-relay-test-$PID.conf"
Push-Location $RepoRoot
try {
    if ($Provider -eq "cloudflare") {
        node (Join-Path $ProjectRoot "tools\mint-relay-test-file.cjs") $RelayFile $EnvFile
    }
    else {
        cargo run --quiet --locked -p rustic-rifts-relay --example mint-relay-file -- $RelayFile $EnvFile
    }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $env:RIFTS_RELAY_FILE = $RelayFile
    cargo test --locked -p rustic-rifts-transport --test ice_link -- --ignored --exact relay_file
    $Result = $LASTEXITCODE
}
finally {
    Remove-Item Env:RIFTS_RELAY_FILE -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $RelayFile) { Remove-Item -LiteralPath $RelayFile -Force }
    Pop-Location
}
exit $Result
