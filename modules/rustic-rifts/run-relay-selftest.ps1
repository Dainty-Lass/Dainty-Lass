param(
    # The relay keys (P2P_CLOUDFLARE_TURN_*) are the backend's, in rustic-isles since the split (DD14).
    [Parameter(Mandatory = $true)]
    [string]$EnvFile,
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release"
)

# Plan step 3 (SteamDocs/HOSTING_TIERS_AND_REALMS_PLAN_2026_10_13.md §6.1): two ICE links on this PC, relay only,
# through Cloudflare. Mints 10-minute credentials from -EnvFile, runs the self-test and always deletes
# the credential file. Prints no secret.
$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$Exe = Join-Path $ProjectRoot "build\$Configuration\ParadoxTransportTest.exe"
if (-not (Test-Path -LiteralPath $Exe)) { throw "Build first: .\build.ps1 -Configuration $Configuration" }
$RelayFile = Join-Path $ProjectRoot "build\$Configuration\relay-test.conf"
try {
    node (Join-Path $ProjectRoot "tools\mint-relay-test-file.cjs") $RelayFile $EnvFile
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $Exe ice --relay-file $RelayFile
    $Result = $LASTEXITCODE
}
finally {
    if (Test-Path -LiteralPath $RelayFile) { Remove-Item -LiteralPath $RelayFile -Force }
}
exit $Result
