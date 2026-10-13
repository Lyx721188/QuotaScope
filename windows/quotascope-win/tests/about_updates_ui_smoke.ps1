# Isolated About update opt-in and retry. Loopback proxy rejects every connection.
param([Parameter(Mandatory=$true)][string]$Executable,[string]$OutputDirectory='target/about-updates-ui-validation')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$id=[guid]::NewGuid().ToString('N')
$profile=Join-Path $env:TEMP ('qs-about-updates-'+$id)
$data=Join-Path $profile 'appdata/QuotaScope'
New-Item -ItemType Directory -Path $data,(Join-Path $profile 'home'),$OutputDirectory -Force|Out-Null
$proxy=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$proxy.Start()
$seed=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$settings=@{hasRun=$true;enabledAccounts=@();language='zh';readsTokenSpend=$false;checksForUpdates=$false;lastUpdateCheckAt=$seed;proxyMode='manual';proxyUrl="http://127.0.0.1:$($proxy.LocalEndpoint.Port)"}
$settingsPath=Join-Path $data 'settings.json'
[IO.File]::WriteAllText($settingsPath,($settings|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$info=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings');$info.UseShellExecute=$false
$info.Environment['APPDATA']=Join-Path $profile 'appdata'
$info.Environment['LOCALAPPDATA']=Join-Path $profile 'local'
$info.Environment['USERPROFILE']=Join-Path $profile 'home'
$info.Environment['HERMES_HOME']=Join-Path $profile 'hermes'
$info.Environment['QUOTASCOPE_TEST_INSTANCE']=$id
$testApp=[Diagnostics.Process]::Start($info)
$condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
$script:connections=0
function Reject-Connections {while($proxy.Pending()){$client=$proxy.AcceptTcpClient();$client.Dispose();$script:connections++}}
function Nodes {
    Reject-Connections
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do {
        if($testApp.HasExited){throw 'Isolated update app exited'}
        try {
            $script:window=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {$_.Current.Name -like 'QuotaScope *' -and -not $_.Current.IsOffscreen}|Select-Object -First 1
            if($window){return $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)}
        }catch [Windows.Automation.ElementNotAvailableException] {}
        Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $until)
    throw 'Update window unavailable'
}
function Wait-Text([string]$name,[switch]$Absent) {
    $until=[DateTime]::UtcNow.AddSeconds(25)
    do {$found=@(Nodes|Where-Object {$_.Current.Name -eq $name}).Count -gt 0;if($found -ne $Absent.IsPresent){return};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
    throw "Update text condition failed: $name (absent=$Absent)"
}
function Page([string]$name) {
    $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
    if(-not $node){throw "Missing update navigation: $name"}
    $node.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
function Automatic([bool]$enabled) {
    $node=Nodes|Where-Object {$_.Current.Name -eq '自动检查更新' -and $_.Current.ClassName -eq 'ToggleSwitch'}|Select-Object -First 1
    if(-not $node){throw 'Named update toggle missing'}
    $toggle=$node.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
    if(($toggle.Current.ToggleState -eq [Windows.Automation.ToggleState]::On) -ne $enabled){$toggle.Toggle()}
    $until=[DateTime]::UtcNow.AddSeconds(5)
    do {if((Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json).checksForUpdates -eq $enabled){return};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
    throw 'Update choice not persisted'
}
function Settle {$until=[DateTime]::UtcNow.AddSeconds(1);do{Reject-Connections;Start-Sleep -Milliseconds 50}while([DateTime]::UtcNow -lt $until)}
$failure='检查更新失败。请稍后重试，或打开发布页面。'
try {
    Page '关于';Wait-Text '检查更新';Settle
    if($connections -ne 0){throw 'Disabled About requested an update'}
    if((Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json).lastUpdateCheckAt -ne $seed){throw 'Disabled About changed attempt time'}
    Automatic $true;Wait-Text $failure
    if($connections -lt 1){throw 'Enabled About did not use the configured proxy'}
    if((Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json).lastUpdateCheckAt -le $seed){throw 'Persisted daily time blocked cold About result'}
    Wait-Text '当前已是最新 Windows 正式版。' -Absent
    $after=$connections
    Page '常规';Page '关于';Settle
    if($connections -ne $after){throw 'Recent failure was requested again'}
    $window.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close();Settle
    if($testApp.HasExited){throw 'Update app exited on Settings close'}
    $reopen=[Diagnostics.Process]::Start($info)
    if(-not $reopen.WaitForExit(15000) -or $reopen.ExitCode -ne 0){throw 'Settings reopen request failed'}
    Page '关于';Wait-Text $failure;Settle
    if($connections -ne $after){throw 'Reopened About ignored recent failure'}
    Automatic $false
    $check=Nodes|Where-Object {$_.Current.Name -eq '检查更新' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1
    $check.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    $until=[DateTime]::UtcNow.AddSeconds(25)
    do{Reject-Connections;if($connections -gt $after){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
    if($connections -le $after){throw 'Explicit manual retry was suppressed'}
    Wait-Text $failure
    if((Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json).checksForUpdates){throw 'Manual retry enabled automatic checks'}
    [pscustomobject]@{Passed=$true;Sha256=(Get-FileHash -LiteralPath $Executable).Hash;DefaultsToManual=$true;AboutProbeWithOptIn=$true;PersistedDailyTimeDoesNotImplyResult=$true;ConfiguredProxyUsed=$true;FailedReadNotUpToDate=$true;RecentFailureReused=$true;CloseAndReopen=$true;ExplicitManualRetry=$true;TogglePersisted=$true;ExternalRequestsBlocked=$true;RealAccountVerified=$false}|ConvertTo-Json|Tee-Object -FilePath (Join-Path $OutputDirectory 'about-updates-ui-result.json')
} catch {
    $originalError=$_
    try{if($window -and -not $testApp.HasExited){Nodes|ForEach-Object {[pscustomobject]@{Name=$_.Current.Name;Class=$_.Current.ClassName}}|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $OutputDirectory 'about-updates-ui-failure.json')}}catch{}
    throw $originalError
} finally {
    if(-not $testApp.HasExited){$testApp.Kill();$testApp.WaitForExit()};$proxy.Stop()
    $resolved=[IO.Path]::GetFullPath($profile)
    if((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') -or (Split-Path $resolved -Leaf) -ne ('qs-about-updates-'+$id)){throw 'Unexpected update profile'}
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
