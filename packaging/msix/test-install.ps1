# Checks the Store package on a real Windows machine (store.yml): signs a COPY
# of the bundle with a throwaway self-signed certificate (the Store signs the
# real one), installs it, runs the `instafind.exe` alias and starts the app
# like the Start menu does, then checks the log the packaged program writes:
# package identity found, startup task present, data under the package's
# private AppData folder. Removes the package and the certificate afterwards.
# Run with Windows PowerShell 5.1 (the Appx module is unreliable in pwsh 7).
param(
    [Parameter(Mandatory)][string]$Bundle,
    [Parameter(Mandatory)][string]$Name,
    [Parameter(Mandatory)][string]$Publisher
)
$ErrorActionPreference = 'Stop'
$failed = $false
function Fail($msg) { Write-Host "::error::$msg"; $script:failed = $true }

$root = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$signtool = Get-ChildItem $root -Directory -Filter '10.*' |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $signtool) { throw "signtool.exe not found under $root" }

$tmp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$signed = Join-Path $tmp 'instafind-test-signed.msixbundle'
Copy-Item $Bundle $signed -Force

# The certificate subject must equal the manifest's Publisher.
$cert = New-SelfSignedCertificate -Type Custom -Subject $Publisher -KeyUsage DigitalSignature `
    -FriendlyName 'InstaFind store.yml test' -CertStoreLocation 'Cert:\CurrentUser\My' `
    -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
$cer = Join-Path $tmp 'instafind-test.cer'
Export-Certificate -Cert $cert -FilePath $cer | Out-Null
Import-Certificate -FilePath $cer -CertStoreLocation 'Cert:\LocalMachine\TrustedPeople' | Out-Null
& $signtool sign /fd SHA256 /sha1 $cert.Thumbprint /s My $signed
if ($LASTEXITCODE -ne 0) { throw "signtool failed ($LASTEXITCODE)" }

try {
    Add-AppxPackage -Path $signed
    $pkg = Get-AppxPackage -Name $Name
    if (-not $pkg) { throw "$Name is not installed after Add-AppxPackage" }
    Write-Host "installed $($pkg.PackageFullName)"
    $pfn = $pkg.PackageFamilyName
    $private = Join-Path $env:LOCALAPPDATA "Packages\$pfn\LocalCache\Local\BahthSari"
    $real = Join-Path $env:LOCALAPPDATA 'BahthSari'

    # The command-line alias runs inside the package.
    $alias = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\instafind.exe'
    if (-not (Test-Path $alias)) {
        Fail "the instafind.exe alias is missing"
    } else {
        $out = Join-Path $tmp 'alias-out.txt'
        $p = Start-Process -FilePath $alias -ArgumentList '--version' -NoNewWindow -Wait -PassThru -RedirectStandardOutput $out
        $text = [IO.File]::ReadAllText($out)
        Write-Host "instafind.exe --version -> exit $($p.ExitCode): $text"
        if ($p.ExitCode -ne 0 -or $text -notmatch '\d+\.\d+\.\d+') { Fail "instafind.exe --version failed" }
    }

    # Start it as the Start menu does and wait for its first log line.
    Start-Process "shell:AppsFolder\$pfn!InstaFind"
    $log = Join-Path $private 'log.txt'
    $line = $null
    for ($i = 0; $i -lt 120 -and -not $line; $i++) {
        Start-Sleep -Seconds 1
        foreach ($f in @($log, (Join-Path $real 'log.txt'))) {
            if (Test-Path $f) {
                $line = Get-Content $f | Where-Object { $_ -match 'store: packaged as' } | Select-Object -Last 1
                if ($line) { $where = $f; break }
            }
        }
    }
    if (-not $line) {
        Fail "the packaged program did not log 'store: packaged as' within 2 minutes"
    } else {
        Write-Host "log ($where): $line"
        if ($where -ne $log) { Fail "data was not written under the package's private folder ($private)" }
        if ($line -notmatch [regex]::Escape($pkg.PackageFullName)) { Fail "the logged package name is not $($pkg.PackageFullName)" }
        if ($line -match 'startup task None') { Fail "the startup task (InstaFindStartup) was not found" }
        if ($line -notmatch 'startup task Some\(Enabled\)') { Write-Host "::warning::startup task not enabled after the first run: $line" }
    }
    Start-Sleep -Seconds 5
    $proc = Get-Process bahth -ErrorAction SilentlyContinue
    if (-not $proc) { Fail "the program exited right after starting" }
    foreach ($d in @($private, $real)) {
        $crash = Join-Path $d 'crash.txt'
        if (Test-Path $crash) { Get-Content $crash | Write-Host; Fail "crash report in $d" }
    }
    if (Test-Path $log) { Write-Host '--- log.txt ---'; Get-Content $log -Tail 30 | Write-Host }
} finally {
    Get-Process bahth -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 2
    Get-AppxPackage -Name $Name | Remove-AppxPackage -ErrorAction SilentlyContinue
    Get-ChildItem 'Cert:\LocalMachine\TrustedPeople' | Where-Object Thumbprint -eq $cert.Thumbprint | Remove-Item
    Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)"
}
if ($failed) { exit 1 }
Write-Host 'Store package: installed, started in Store mode, removed.'
