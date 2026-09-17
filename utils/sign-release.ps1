#Requires -Version 5.1
<#
.SYNOPSIS
    Baut und Authenticode-signiert die VulpiCast-Binaries lokal.

.DESCRIPTION
    Signiert ohne Windows SDK: verwendet das in Windows eingebaute
    Set-AuthenticodeSignature anstelle von signtool.exe.

    Zertifikatsquelle, in dieser Reihenfolge:
      1. -PfxPath        echtes Code-Signing-Zertifikat als .pfx
      2. -CertThumbprint Zertifikat aus Cert:\CurrentUser\My
      3. automatisch     vorhandenes selbstsigniertes VulpiCast-Zertifikat
      4. -CreateCert     erzeugt ein neues selbstsigniertes Zertifikat

    WICHTIG: Ein selbstsigniertes Zertifikat erzeugt eine technisch gueltige
    Signatur - Manipulation an der EXE wird erkannt. Es erzeugt aber KEIN
    oeffentliches Vertrauen. Windows zeigt weiterhin "Unbekannter Herausgeber"
    und SmartScreen warnt. Dafuer braucht es ein Zertifikat einer oeffentlichen CA.

.EXAMPLE
    # Erstmalig: Zertifikat anlegen, lokal vertrauen, bauen, signieren, exportieren
    .\utils\sign-release.ps1 -CreateCert -TrustLocally -ExportCert

.EXAMPLE
    # Danach: nur bauen und signieren, das Zertifikat wird wiedergefunden
    .\utils\sign-release.ps1

.EXAMPLE
    # Mit einem echten Zertifikat einer CA
    .\utils\sign-release.ps1 -PfxPath C:\keys\vulpicast.pfx
#>
[CmdletBinding()]
param(
    # Welche Crates gebaut und signiert werden. Paketname = Binaryname.
    [string[]] $Crates = @('vulpicast'),

    # Zielverzeichnis fuer die signierten Artefakte.
    [string] $OutDir = 'dist',

    # Pfad zu einem .pfx mit Code-Signing-Zertifikat und privatem Schluessel.
    [string] $PfxPath,

    # Passwort zum .pfx. Ohne Angabe wird interaktiv gefragt.
    [securestring] $PfxPassword,

    # Thumbprint eines Zertifikats aus Cert:\CurrentUser\My.
    [string] $CertThumbprint,

    # Legt ein neues selbstsigniertes Code-Signing-Zertifikat an.
    [switch] $CreateCert,

    # Subject fuer ein neu erzeugtes Zertifikat.
    [string] $Subject = 'CN=VulpiCast, O=VulpiCast, C=DE',

    # Traegt das Zertifikat in die Vertrauensspeicher des aktuellen Nutzers ein,
    # damit die Signatur auf DIESEM Rechner als gueltig gilt.
    [switch] $TrustLocally,

    # Exportiert den oeffentlichen Teil als .cer neben die Binaries.
    [switch] $ExportCert,

    # Ueberspringt cargo build und signiert bereits gebaute Binaries.
    [switch] $SkipBuild,

    # Optionales cargo --target Tripel.
    [string] $Target,

    # Timestamp-Server, werden der Reihe nach durchprobiert.
    [string[]] $TimestampUrls = @(
        'http://timestamp.digicert.com',
        'http://timestamp.sectigo.com',
        'http://time.certum.pl'
    )
)

$ErrorActionPreference = 'Stop'

$RepoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $RepoRoot

function Write-Step { param([string] $Message) Write-Host "`n==> $Message" -ForegroundColor Cyan }
function Write-Ok   { param([string] $Message) Write-Host "    OK  $Message" -ForegroundColor Green }
function Write-Warn { param([string] $Message) Write-Host "    !   $Message" -ForegroundColor Yellow }

try {
    # ------------------------------------------------------------- Zertifikat
    Write-Step 'Zertifikat ermitteln'

    $cert = $null
    $isSelfSigned = $false

    if ($PfxPath) {
        if (-not (Test-Path -LiteralPath $PfxPath)) {
            throw "PFX nicht gefunden: $PfxPath"
        }
        if (-not $PfxPassword) {
            $PfxPassword = Read-Host -AsSecureString "Passwort fuer $([IO.Path]::GetFileName($PfxPath))"
        }
        $cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 @(
            (Resolve-Path -LiteralPath $PfxPath).Path,
            $PfxPassword,
            'Exportable,PersistKeySet'
        )
        Write-Ok "aus PFX geladen: $($cert.Subject)"
    }
    elseif ($CertThumbprint) {
        $clean = $CertThumbprint -replace '[^0-9A-Fa-f]', ''
        $cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Thumbprint -eq $clean }
        if (-not $cert) {
            throw "Kein Zertifikat mit Thumbprint $clean in Cert:\CurrentUser\My"
        }
        Write-Ok "aus Zertifikatsspeicher: $($cert.Subject)"
    }
    else {
        # Ein frueher erzeugtes selbstsigniertes VulpiCast-Zertifikat wiederverwenden.
        $cert = Get-ChildItem Cert:\CurrentUser\My |
            Where-Object {
                $_.Subject -eq $Subject -and
                $_.HasPrivateKey -and
                $_.NotAfter -gt (Get-Date) -and
                ($_.EnhancedKeyUsageList.ObjectId -contains '1.3.6.1.5.5.7.3.3')
            } |
            Sort-Object NotAfter -Descending |
            Select-Object -First 1

        if ($cert) {
            $isSelfSigned = $true
            Write-Ok "vorhandenes Zertifikat: $($cert.Thumbprint)"
            Write-Ok "gueltig bis $($cert.NotAfter.ToString('yyyy-MM-dd'))"
        }
        elseif ($CreateCert) {
            $cert = New-SelfSignedCertificate `
                -Type CodeSigningCert `
                -Subject $Subject `
                -KeyAlgorithm RSA `
                -KeyLength 3072 `
                -HashAlgorithm SHA256 `
                -KeyExportPolicy Exportable `
                -CertStoreLocation Cert:\CurrentUser\My `
                -NotAfter (Get-Date).AddYears(5)
            $isSelfSigned = $true
            Write-Ok "neu erzeugt: $($cert.Thumbprint)"
        }
        else {
            throw @"
Kein Zertifikat gefunden.

  Selbstsigniert anlegen:  .\utils\sign-release.ps1 -CreateCert -TrustLocally
  Echtes Zertifikat:       .\utils\sign-release.ps1 -PfxPath <pfad.pfx>
"@
        }
    }

    if ($isSelfSigned) {
        Write-Warn 'Selbstsigniert: gueltige Signatur, aber kein oeffentliches Vertrauen.'
        Write-Warn 'Windows meldet weiterhin "Unbekannter Herausgeber", SmartScreen warnt.'
    }

    # -------------------------------------------------------- Lokal vertrauen
    if ($TrustLocally) {
        Write-Step 'Zertifikat lokal als vertrauenswuerdig eintragen'

        # Nur der oeffentliche Teil wandert in die Trust-Stores, nie der Schluessel.
        $publicOnly = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 @(
            ,$cert.RawData
        )

        foreach ($storeName in @('Root', 'TrustedPublisher')) {
            $store = New-Object System.Security.Cryptography.X509Certificates.X509Store @(
                $storeName, 'CurrentUser'
            )
            $store.Open('ReadWrite')
            try {
                $found = $store.Certificates.Find('FindByThumbprint', $cert.Thumbprint, $false)
                if ($found.Count -eq 0) {
                    $store.Add($publicOnly)
                    Write-Ok "CurrentUser\$storeName"
                }
                else {
                    Write-Ok "CurrentUser\$storeName (war bereits eingetragen)"
                }
            }
            finally {
                $store.Close()
            }
        }
        Write-Warn 'Gilt nur fuer diesen Windows-Benutzer auf diesem Rechner.'
    }

    # --------------------------------------------------------------- Build
    $profileDir = if ($Target) { "target/$Target/release" } else { 'target/release' }

    if ($SkipBuild) {
        Write-Step 'Build uebersprungen (-SkipBuild)'
    }
    else {
        Write-Step 'Release-Build'
        $cargoArgs = @('build', '--release')
        foreach ($c in $Crates) { $cargoArgs += @('-p', $c) }
        if ($Target) { $cargoArgs += @('--target', $Target) }

        Write-Host "    cargo $($cargoArgs -join ' ')" -ForegroundColor DarkGray
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build fehlgeschlagen (exit $LASTEXITCODE)"
        }
        Write-Ok 'Build abgeschlossen'
    }

    # ---------------------------------------------------------- Artefakte
    Write-Step 'Artefakte einsammeln'

    if (-not (Test-Path -LiteralPath $OutDir)) {
        New-Item -ItemType Directory -Path $OutDir | Out-Null
    }
    $OutDirFull = (Resolve-Path -LiteralPath $OutDir).Path

    $artifacts = @()
    foreach ($c in $Crates) {
        $src = Join-Path $RepoRoot "$profileDir/$c.exe"
        if (-not (Test-Path -LiteralPath $src)) {
            throw "Binary nicht gefunden: $src`nOhne -SkipBuild laufen lassen oder -Crates pruefen."
        }
        $dst = Join-Path $OutDirFull "$c.exe"
        Copy-Item -LiteralPath $src -Destination $dst -Force
        $artifacts += $dst
        $sizeMb = [math]::Round((Get-Item -LiteralPath $dst).Length / 1MB, 2)
        Write-Ok "$c.exe ($sizeMb MB)"
    }

    # --------------------------------------------------------------- Signieren
    Write-Step 'Signieren'

    foreach ($file in $artifacts) {
        $name = Split-Path -Leaf $file
        $timestamped = $false

        foreach ($url in $TimestampUrls) {
            try {
                Set-AuthenticodeSignature `
                    -FilePath $file `
                    -Certificate $cert `
                    -HashAlgorithm SHA256 `
                    -TimestampServer $url `
                    -IncludeChain All `
                    -ErrorAction Stop | Out-Null
                Write-Ok "$name signiert, Zeitstempel von $url"
                $timestamped = $true
                break
            }
            catch {
                Write-Warn "Zeitstempel ueber $url fehlgeschlagen: $($_.Exception.Message)"
            }
        }

        if (-not $timestamped) {
            # Ohne Zeitstempel wird die Signatur ungueltig, sobald das Zertifikat ablaeuft.
            Write-Warn "$name wird OHNE Zeitstempel signiert."
            Set-AuthenticodeSignature `
                -FilePath $file `
                -Certificate $cert `
                -HashAlgorithm SHA256 `
                -IncludeChain All | Out-Null
        }
    }

    # ------------------------------------------------------------ Verifizieren
    Write-Step 'Signaturen pruefen'

    $allTrusted = $true
    foreach ($file in $artifacts) {
        $sig = Get-AuthenticodeSignature -FilePath $file
        $name = Split-Path -Leaf $file

        # Windows bevorzugt bei OS-Binaries eine Katalogsignatur und meldet dann
        # "Valid" fuer einen fremden Signierer. Erst pruefen, WER signiert hat.
        if ($sig.SignatureType -ne 'Authenticode') {
            Write-Warn "$name keine eingebettete Signatur (SignatureType: $($sig.SignatureType))"
            $allTrusted = $false
            continue
        }
        if ($sig.SignerCertificate.Thumbprint -ne $cert.Thumbprint) {
            Write-Warn "$name von fremdem Zertifikat signiert: $($sig.SignerCertificate.Thumbprint)"
            Write-Warn "  erwartet: $($cert.Thumbprint)"
            $allTrusted = $false
            continue
        }

        if ($sig.Status -eq 'Valid') {
            Write-Ok "$name  Valid  $($sig.SignerCertificate.Subject)"
        }
        elseif ($sig.Status -eq 'UnknownError') {
            # Typisch bei selbstsigniert: Signatur intakt, Kette nicht vertraut.
            Write-Warn "$name signiert, Kette nicht vertraut (erwartet bei selbstsigniert)"
            Write-Warn '  -TrustLocally behebt das fuer diesen Rechner.'
            $allTrusted = $false
        }
        else {
            Write-Warn "$name Status: $($sig.Status) - $($sig.StatusMessage)"
            $allTrusted = $false
        }

        if ($sig.TimeStamperCertificate) {
            Write-Ok "  Zeitstempel: $($sig.TimeStamperCertificate.Subject)"
        }
    }

    # ------------------------------------------------------- Zertifikatsexport
    if ($ExportCert) {
        Write-Step 'Oeffentliches Zertifikat exportieren'
        $cerPath = Join-Path $OutDirFull 'VulpiCast-CodeSigning.cer'
        [IO.File]::WriteAllBytes($cerPath, $cert.Export('Cert'))
        Write-Ok 'VulpiCast-CodeSigning.cer'
        Write-Host '    Nutzer koennen dem Zertifikat einmalig vertrauen mit:' -ForegroundColor DarkGray
        Write-Host '      Import-Certificate -FilePath VulpiCast-CodeSigning.cer -CertStoreLocation Cert:\CurrentUser\TrustedPublisher' -ForegroundColor DarkGray
    }

    # ------------------------------------------------------------- Pruefsummen
    Write-Step 'Pruefsummen schreiben'
    $sumPath = Join-Path $OutDirFull 'SHA256SUMS.txt'
    $lines = @()
    foreach ($f in (Get-ChildItem -LiteralPath $OutDirFull -File | Sort-Object Name)) {
        if ($f.Name -eq 'SHA256SUMS.txt') { continue }
        $hash = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLower()
        $lines += ('{0}  {1}' -f $hash, $f.Name)
    }
    Set-Content -LiteralPath $sumPath -Value $lines -Encoding ascii
    foreach ($l in $lines) { Write-Host "    $l" -ForegroundColor DarkGray }

    Write-Host "`nFertig. Artefakte in: $OutDirFull" -ForegroundColor Green
    if (-not $allTrusted) {
        Write-Host 'Hinweis: mindestens eine Signatur ist nicht voll vertrauenswuerdig (siehe oben).' -ForegroundColor Yellow
    }
}
finally {
    Pop-Location
}
