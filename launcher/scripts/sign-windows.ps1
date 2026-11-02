# Authenticode-signs one file for tauri's bundle.windows.signCommand (RA14, docs/launcher-release.md).
# The certificate and its password come from the release workflow's environment, never from the command
# line tauri prints.
param([Parameter(Mandatory)][string]$File)
$ErrorActionPreference = 'Stop'

$certificate = $env:WINDOWS_CERTIFICATE_PATH
if (-not $certificate -or -not (Test-Path -LiteralPath $certificate)) {
    throw 'WINDOWS_CERTIFICATE_PATH is not set to the certificate file'
}
# The newest Windows SDK's signtool
$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
    Sort-Object { try { [version]$_.Directory.Parent.Name } catch { [version]'0.0' } } -Descending |
    Select-Object -First 1
if (-not $signtool) { throw 'signtool.exe not found: install a Windows 10/11 SDK' }
$timestamp = if ($env:WINDOWS_TIMESTAMP_URL) { $env:WINDOWS_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }

& $signtool.FullName sign /q /fd sha256 /td sha256 /tr $timestamp /f $certificate /p $env:WINDOWS_CERTIFICATE_PASSWORD $File
if ($LASTEXITCODE -ne 0) { throw "signtool failed on $File (exit $LASTEXITCODE)" }
