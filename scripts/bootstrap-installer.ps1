# Install the official Inno Setup compiler used to build VulpiCast-Setup.exe.
$ErrorActionPreference = "Stop"

$isccCandidates = @(
    (Join-Path $env:ProgramFiles "Inno Setup 7\ISCC.exe"),
    (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 7\ISCC.exe"),
    (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 7\ISCC.exe")
)
$iscc = $isccCandidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
if ($iscc) {
    Write-Host "Inno Setup ist bereits installiert: $iscc"
    exit 0
}

$winget = Get-Command winget.exe -ErrorAction SilentlyContinue
if (-not $winget) {
    throw "winget wurde nicht gefunden. Inno Setup 7 bitte von https://jrsoftware.org/isdl.php installieren."
}

& $winget.Source install --id JRSoftware.InnoSetup.7 -e --silent `
    --accept-package-agreements --accept-source-agreements
if ($LASTEXITCODE -ne 0) {
    throw "Inno Setup konnte nicht installiert werden ($LASTEXITCODE)."
}

$iscc = $isccCandidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
if (-not $iscc) {
    throw "ISCC.exe wurde nach der Installation nicht gefunden."
}
Write-Host "Inno Setup wurde installiert: $iscc"
