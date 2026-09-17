# Build VulpiCast and create the Windows installer.
$ErrorActionPreference = "Stop"

$root = $PSScriptRoot
$cargoHome = Join-Path $root ".tools\cargo"
$rustupHome = Join-Path $root ".tools\rustup"
$mingwBin = Join-Path $root ".tools\mingw\mingw64\bin"
$cargo = Join-Path $cargoHome "bin\cargo.exe"

if (-not (Test-Path -LiteralPath $cargo) -or -not (Test-Path -LiteralPath (Join-Path $mingwBin "gcc.exe"))) {
    throw "Local Rust build tools are missing. Run .\scripts\bootstrap-toolchain.ps1 first."
}

$isccCandidates = @(
    (Join-Path $env:ProgramFiles "Inno Setup 7\ISCC.exe"),
    (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 7\ISCC.exe"),
    (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 7\ISCC.exe")
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }
$iscc = $isccCandidates | Select-Object -First 1
if (-not $iscc) {
    throw "Inno Setup 7 is missing. Run .\scripts\bootstrap-installer.ps1 first."
}

$manifest = Get-Content -LiteralPath (Join-Path $root "Cargo.toml") -Raw
if ($manifest -notmatch '(?ms)\[workspace\.package\].*?version\s*=\s*"([^"]+)"') {
    throw "Projektversion konnte nicht aus Cargo.toml gelesen werden."
}
$version = $Matches[1]

$env:CARGO_HOME = $cargoHome
$env:RUSTUP_HOME = $rustupHome
$env:PATH = "$($cargoHome)\bin;$mingwBin;$env:PATH"

Push-Location $root
try {
    & $cargo build -p vulpicast --release --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }

    $dist = Join-Path $root "dist"
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    foreach ($obsolete in @("VulpiCast.exe", "Add-FirewallRule.ps1", "LICENSE")) {
        $obsoletePath = Join-Path $dist $obsolete
        if (Test-Path -LiteralPath $obsoletePath -PathType Leaf) {
            [System.IO.File]::Delete($obsoletePath)
        }
    }
    & $iscc "/DMyAppVersion=$version" (Join-Path $root "installer\VulpiCast.iss")
    if ($LASTEXITCODE -ne 0) { throw "Installer build failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
}

$installer = Join-Path $root "dist\VulpiCast-Setup.exe"
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $installer).Hash
Write-Host "Fertig: $installer"
Write-Host "SHA256: $hash"
