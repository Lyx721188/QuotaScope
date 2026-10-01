[CmdletBinding()]
param(
    [string]$CompilerPath,
    [string]$OutputDirectory,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repo 'windows/target/installer' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $CompilerPath) {
    $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($compiler) { $CompilerPath = $compiler.Source }
    else {
        $candidates = @(
            (Join-Path $env:LOCALAPPDATA 'Programs/Inno Setup 6/ISCC.exe'),
            (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6/ISCC.exe'),
            (Join-Path $env:ProgramFiles 'Inno Setup 6/ISCC.exe')
        )
        $CompilerPath = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    }
}
if (-not $CompilerPath -or -not (Test-Path -LiteralPath $CompilerPath)) {
    throw 'Install Inno Setup 6, or supply -CompilerPath with the path to ISCC.exe.'
}
if (-not $SkipBuild) {
    & cargo build --release -p quotascope-win --manifest-path (Join-Path $repo 'windows/Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw "Release build failed: $LASTEXITCODE" }
}
$release = Join-Path $repo 'windows/target/release'
$exe = Get-Item -LiteralPath (Join-Path $release 'quotascope.exe')
$manifest = Get-Content -LiteralPath (Join-Path $repo 'windows/Cargo.toml') -Raw
$version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^\"]+)"').Groups[1].Value
if (-not $version -or $exe.VersionInfo.ProductVersion -ne $version) {
    throw 'Executable version does not match the workspace version. Rebuild without -SkipBuild.'
}
$payload = Join-Path $OutputDirectory ('payload-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $payload -Force | Out-Null
Copy-Item -LiteralPath $exe.FullName -Destination $payload
Get-ChildItem -LiteralPath $release -File |
    Where-Object { $_.Extension -in '.dll', '.pri' } |
    Copy-Item -Destination $payload
Get-ChildItem -LiteralPath $release -Directory |
    Where-Object { $_.Name -notin '.fingerprint', 'build', 'deps', 'examples', 'incremental' } |
    Copy-Item -Destination $payload -Recurse
Copy-Item -LiteralPath (Join-Path $repo 'LICENSE'), (Join-Path $repo 'THIRD_PARTY_NOTICES.md') -Destination $payload
foreach ($required in @('Microsoft.UI.Xaml.dll', 'resources.pri', 'Fonts/HarmonyOS_Sans_SC_Regular.ttf')) {
    if (-not (Test-Path -LiteralPath (Join-Path $payload $required))) { throw "Missing runtime asset: $required" }
}
& $CompilerPath "/DAppVersion=$version" "/DPayloadDir=$payload" "/DOutputDir=$OutputDirectory" (Join-Path $PSScriptRoot 'QuotaScope.iss')
if ($LASTEXITCODE -ne 0) { throw "Installer compilation failed: $LASTEXITCODE" }
$setup = Join-Path $OutputDirectory "QuotaScope-$version-windows-x64-Setup.exe"
$hash = Get-FileHash -LiteralPath $setup -Algorithm SHA256
"$($hash.Hash)  $([IO.Path]::GetFileName($setup))" | Set-Content -LiteralPath "$setup.sha256" -Encoding ascii
Write-Output "Installer: $setup"
Write-Output "Payload: $payload"
Write-Output "SHA256: $($hash.Hash)"
