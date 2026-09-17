# Install a self-contained Rust GNU + MinGW toolchain below .tools\.
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$toolsDir = Join-Path $root ".tools"
$downloads = Join-Path $toolsDir "downloads"
$cargoHome = Join-Path $toolsDir "cargo"
$rustupHome = Join-Path $toolsDir "rustup"
$mingwRoot = Join-Path $toolsDir "mingw"
New-Item -ItemType Directory -Force -Path $downloads, $cargoHome, $rustupHome, $mingwRoot | Out-Null

$rustupInit = Join-Path $downloads "rustup-init.exe"
if (-not (Test-Path -LiteralPath $rustupInit)) {
    curl.exe -L --fail --show-error --output $rustupInit `
        "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe"
    if ($LASTEXITCODE -ne 0) { throw "rustup-init download failed" }
}

$env:CARGO_HOME = $cargoHome
$env:RUSTUP_HOME = $rustupHome
if (-not (Test-Path -LiteralPath (Join-Path $cargoHome "bin\cargo.exe"))) {
    & $rustupInit -y --profile minimal --default-host x86_64-pc-windows-gnu --default-toolchain stable
    if ($LASTEXITCODE -ne 0) { throw "Rust installation failed" }
}
& (Join-Path $cargoHome "bin\rustup.exe") component add rustfmt
if ($LASTEXITCODE -ne 0) { throw "rustfmt installation failed" }

$mingwName = "winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r1.7z"
$mingwUrl = "https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-14.0.0-ucrt-r1/$mingwName"
$mingwSha256 = "9714f9e55905000ec2a066ae033abdfe8c93083a925e3fe015d3c7cc4bb1c918"
$archive = Join-Path $downloads $mingwName
if (-not (Test-Path -LiteralPath $archive)) {
    curl.exe -L --fail --show-error --output $archive $mingwUrl
    if ($LASTEXITCODE -ne 0) { throw "MinGW download failed" }
}

$actualSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
if ($actualSha256 -ne $mingwSha256) {
    throw "MinGW SHA256 mismatch: expected $mingwSha256, got $actualSha256"
}

$gcc = Join-Path $mingwRoot "mingw64\bin\gcc.exe"
if (-not (Test-Path -LiteralPath $gcc)) {
    $sevenZip = Get-Command 7z -ErrorAction SilentlyContinue
    if ($sevenZip) {
        & $sevenZip.Source x -y "-o$mingwRoot" $archive
    } else {
        tar -xf $archive -C $mingwRoot
    }
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $gcc)) {
        throw "MinGW extraction failed"
    }
}

Write-Host "Projektlokale Toolchain ist bereit."
Write-Host "Jetzt .\build.ps1 ausführen."
