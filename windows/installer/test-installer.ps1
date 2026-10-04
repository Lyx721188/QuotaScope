# Installs the shipped payload with independent identity, paths and shortcuts.
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$PayloadDirectory,
    [Parameter(Mandatory=$true)][string]$CompilerPath,
    [int]$IdleSeconds=2
)
$ErrorActionPreference='Stop'
$repo=Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$targetRoot=[IO.Path]::GetFullPath((Join-Path $repo 'windows/target'))
$id=[guid]::NewGuid().ToString('N')
$validationRoot=Join-Path $targetRoot ('installer-validation-'+$id)
$installDirectory=Join-Path $validationRoot 'installed'
if(-not [IO.Path]::GetFullPath($installDirectory).StartsWith($targetRoot+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Unexpected validation directory'}
New-Item -ItemType Directory -Path $validationRoot -Force|Out-Null
$manifest=Get-Content -LiteralPath (Join-Path $repo 'windows/Cargo.toml') -Raw
$version=[regex]::Match($manifest,'(?m)^version\s*=\s*"([^\"]+)"').Groups[1].Value
$realExe=Join-Path $env:LOCALAPPDATA 'Programs/QuotaScope/quotascope.exe'
$realHash=if(Test-Path -LiteralPath $realExe){(Get-FileHash -LiteralPath $realExe).Hash}else{$null}
$appId='QuotaScope.Windows.Validation.'+$id
& $CompilerPath "/DAppVersion=$version" "/DPayloadDir=$([IO.Path]::GetFullPath($PayloadDirectory))" "/DOutputDir=$validationRoot" '/DOutputName=installer-validation' "/DInstallerAppId=$appId" "/DInstallerGroupName=QuotaScope Validation $id" "/DInstallerDefaultDir=$installDirectory" (Join-Path $PSScriptRoot 'QuotaScope.iss') 1> (Join-Path $validationRoot 'compiler.log') 2>&1
if($LASTEXITCODE -ne 0){throw 'Validation installer compile failed'}
function Run-Setup([string]$path,[string[]]$parameters){
    $info=[Diagnostics.ProcessStartInfo]::new($path)
    $info.UseShellExecute=$false;$info.CreateNoWindow=$true
    foreach($parameter in $parameters){$info.ArgumentList.Add($parameter)}
    $process=[Diagnostics.Process]::Start($info)
    if(-not $process.WaitForExit(60000)){throw 'Installer timed out'}
    $exit=$process.ExitCode;$process.Dispose()
    if($exit -ne 0){throw "Installer exit code $exit"}
}
$testExe=Join-Path $installDirectory 'quotascope.exe'
$installed=$false
$previousExe=$env:QUOTASCOPE_TEST_EXE
$previousIdle=$env:QUOTASCOPE_TEST_IDLE_SECONDS
try{
    Run-Setup (Join-Path $validationRoot 'installer-validation.exe') @('/VERYSILENT','/SUPPRESSMSGBOXES','/SP-','/NORESTART','/NOCLOSEAPPLICATIONS','/NOICONS','/TASKS=',"/DIR=$installDirectory","/LOG=$(Join-Path $validationRoot 'install.log')")
    $installed=$true
    $actual=Get-Item -LiteralPath $testExe
    if($actual.VersionInfo.ProductVersion -ne $version){throw 'Installed version mismatch'}
    $expectedHash=(Get-FileHash -LiteralPath (Join-Path $PayloadDirectory 'quotascope.exe')).Hash
    if((Get-FileHash -LiteralPath $testExe).Hash -ne $expectedHash){throw 'Installed executable differs from payload'}
    foreach($required in @('Microsoft.UI.Xaml.dll','resources.pri','Fonts/HarmonyOS_Sans_SC_Regular.ttf')){if(-not(Test-Path -LiteralPath (Join-Path $installDirectory $required))){throw "Missing installed resource: $required"}}
    foreach($notice in @('LICENSE','THIRD_PARTY_NOTICES.md')){
        $sourceHash=(Get-FileHash -LiteralPath (Join-Path $repo $notice)).Hash
        if((Get-FileHash -LiteralPath (Join-Path $installDirectory $notice)).Hash -ne $sourceHash){throw "Installed notice missing or stale: $notice"}
    }
    $env:QUOTASCOPE_TEST_EXE=$testExe
    $env:QUOTASCOPE_TEST_IDLE_SECONDS=$IdleSeconds.ToString()
    Push-Location (Join-Path $repo 'windows')
    try{
        cargo test -p quotascope-win --test settings_lifecycle --locked -- --ignored --nocapture --test-threads=1 1> (Join-Path $validationRoot 'lifecycle.log') 2>&1
        if($LASTEXITCODE -ne 0){throw 'Installed Settings lifecycle failed'}
    }finally{Pop-Location}
    Run-Setup (Join-Path $installDirectory 'unins000.exe') @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',"/LOG=$(Join-Path $validationRoot 'uninstall.log')")
    $installed=$false
    if(Test-Path -LiteralPath $testExe){throw 'Uninstaller retained the installed executable'}
    $key="HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${appId}_is1"
    if(Test-Path -LiteralPath $key){throw 'Uninstaller retained its registry entry'}
    if($realHash -and (Get-FileHash -LiteralPath $realExe).Hash -ne $realHash){throw 'Existing installation was modified'}
    $result=[pscustomobject]@{Passed=$true;Version=$version;InstalledExecutableSHA256=$expectedHash;IndependentIdentity=$true;TrayLifecycle=$true;LicenseNoticesVerified=$true;Uninstalled=$true;ExistingInstallationPreserved=$true;IdleSeconds=$IdleSeconds}
    $result|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $validationRoot 'result.json') -Encoding utf8
    $result|ConvertTo-Json
    Write-Output "Validation logs: $validationRoot"
}finally{
    $env:QUOTASCOPE_TEST_EXE=$previousExe
    $env:QUOTASCOPE_TEST_IDLE_SECONDS=$previousIdle
    if($installed -and (Test-Path -LiteralPath (Join-Path $installDirectory 'unins000.exe'))){Run-Setup (Join-Path $installDirectory 'unins000.exe') @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART')}
}
