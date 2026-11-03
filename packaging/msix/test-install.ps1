# Checks the Store package on a real Windows machine (store.yml): signs a COPY
# of the bundle (or of one CPU's .msix) with a throwaway self-signed certificate (the Store signs the
# real one), installs it, runs the `instafind.exe` alias and starts the app
# like the Start menu does, then checks the log the packaged program writes:
# package identity found, data under the package's
# private AppData folder, startup task present but off (the user has not
# turned it on). Removes the package and the certificate afterwards.
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
# The native signtool on Windows on ARM, else the x64 one (emulated there).
$hostArch = if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq 'Arm64') { 'arm64' } else { 'x64' }
$signtool = Get-ChildItem $root -Directory -Filter '10.*' |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName "$hostArch\signtool.exe"; Join-Path $_.FullName 'x64\signtool.exe' } |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $signtool) { throw "signtool.exe not found under $root" }

$tmp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$signed = Join-Path $tmp ('instafind-test-signed' + [IO.Path]::GetExtension($Bundle))
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

    # Start it as the Start menu does and wait for its first log line; if
    # that shows no sign of life, start it through the alias (also inside
    # the package) to tell a launch problem from a program problem.
    $log = Join-Path $private 'log.txt'
    function Wait-StoreLine([int]$Seconds) {
        for ($i = 0; $i -lt $Seconds; $i++) {
            Start-Sleep -Seconds 1
            foreach ($f in @($log, (Join-Path $real 'log.txt'))) {
                if (Test-Path $f) {
                    $l = Get-Content $f | Where-Object { $_ -match 'store: packaged as' } | Select-Object -Last 1
                    if ($l) { return @{ Line = $l; Where = $f } }
                }
            }
        }
        return $null
    }
    $started = Get-Date
    $appId = 'InstaFind'
    Write-Host "starting shell:AppsFolder\$pfn!$appId"
    Start-Process "shell:AppsFolder\$pfn!$appId"
    $hit = Wait-StoreLine 60
    if (-not $hit) {
        Write-Host "::warning::no log line 60 s after the Start-menu style launch; processes: $((Get-Process bahth -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }) -join ', ')"
        if (Test-Path $alias) {
            Write-Host "starting $alias (no arguments)"
            $viaAlias = Start-Process -FilePath $alias -PassThru
            $hit = Wait-StoreLine 60
            if (-not $hit) {
                $viaAlias.Refresh()
                $code = if ($viaAlias.HasExited) { '0x{0:x8}' -f $viaAlias.ExitCode } else { 'still running' }
                Write-Host "alias launch: $code"
            } else {
                Write-Host "::warning::the program started through the alias but not through shell:AppsFolder"
            }
        }
    }
    $line = if ($hit) { $hit.Line } else { $null }
    $where = if ($hit) { $hit.Where } else { $null }
    if (-not $line) {
        Fail "the packaged program did not log 'store: packaged as' within 2 minutes"
        Write-Host '--- diagnostics ---'
        Get-Process | Where-Object { $_.Path -like '*WindowsApps*' -or $_.Name -eq 'bahth' } | Format-Table Id, Name, Path -AutoSize | Out-String | Write-Host
        Write-Host "package folder $($pkg.InstallLocation):"
        Get-ChildItem $pkg.InstallLocation -ErrorAction SilentlyContinue | Format-Table Name, Length | Out-String | Write-Host
        $pkgData = Join-Path $env:LOCALAPPDATA "Packages\$pfn"
        Write-Host "files under $pkgData`:"
        Get-ChildItem $pkgData -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object { "  $($_.FullName) ($($_.Length))" } | Write-Host
        if (Test-Path $real) { Write-Host "files under $real`:"; Get-ChildItem $real -File | ForEach-Object { "  $($_.Name) ($($_.Length))" } | Write-Host }
        foreach ($f in @((Join-Path $real 'log.txt'))) { if (Test-Path $f) { Write-Host "--- $f ---"; Get-Content $f -Tail 20 | Write-Host } }
        foreach ($logName in @('Application', 'Microsoft-Windows-AppModel-Runtime/Admin', 'Microsoft-Windows-AppXDeploymentServer/Operational', 'Microsoft-Windows-TWinUI/Operational')) {
            $events = Get-WinEvent -FilterHashtable @{ LogName = $logName; StartTime = $started.AddSeconds(-10) } -ErrorAction SilentlyContinue |
                Where-Object { $_.Level -le 3 -or $_.Message -match 'bahth|InstaFind' } | Select-Object -First 15
            foreach ($e in $events) {
                $msg = "$($e.Message)" -replace '\s+', ' '
                Write-Host ("[{0}] {1} {2} {3}: {4}" -f $logName, $e.TimeCreated.ToString('HH:mm:ss'), $e.ProviderName, $e.Id, $msg.Substring(0, [Math]::Min(400, $msg.Length)))
            }
        }
    } else {
        Write-Host "log ($where): $line"
        if ($where -ne $log) { Fail "data was not written under the package's private folder ($private)" }
        if ($line -notmatch [regex]::Escape($pkg.PackageFullName)) { Fail "the logged package name is not $($pkg.PackageFullName)" }
        if ($line -match 'startup task None') { Fail "the startup task (InstaFindStartup) was not found" }
        # Off until the user turns it on (store::ENABLE_STARTUP_ON_FIRST_RUN,
        # Store policy 10.2.8): the first run must not enable it.
        if ($line -notmatch 'startup task Some\(Disabled\)') { Fail "the startup task is not plainly off after the first run (the user did not consent): $line" }
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
