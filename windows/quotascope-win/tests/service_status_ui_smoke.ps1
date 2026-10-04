# Isolated public-status failure handling and independent notification switch.
# The loopback proxy closes connections: no official status or account request leaves the profile.
param([Parameter(Mandatory=$true)][string]$Executable,[string]$OutputDirectory='target/service-status-ui-validation',[switch]$ProbePublicPages)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class StatusWindow {
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wparam, IntPtr lparam);
}
'@
$evidence=[IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $evidence | Out-Null
$id=[guid]::NewGuid().ToString('N')
$profile=Join-Path $env:TEMP ('qs-service-status-'+$id)
$data=Join-Path $profile 'appdata/QuotaScope'
New-Item -ItemType Directory -Force -Path $data,(Join-Path $profile 'home') | Out-Null
$proxy=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$proxy.Start()
$settings=@{hasRun=$true;enabledAccounts=@();language='zh';readsTokenSpend=$false;checksForUpdates=$false;wantsAlerts=$false;alertsOnOutage=$false;proxyMode='manual';proxyUrl="http://127.0.0.1:$($proxy.LocalEndpoint.Port)"}
if($ProbePublicPages){$settings.proxyMode='disabled';$settings.proxyUrl=''}
[IO.File]::WriteAllText((Join-Path $data 'settings.json'),($settings|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$info=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings')
$info.UseShellExecute=$false
$info.Environment['APPDATA']=Join-Path $profile 'appdata'
$info.Environment['LOCALAPPDATA']=Join-Path $profile 'local'
$info.Environment['USERPROFILE']=Join-Path $profile 'home'
$info.Environment['HERMES_HOME']=Join-Path ($info.Environment['USERPROFILE']) '.hermes'
$info.Environment['QUOTASCOPE_TEST_INSTANCE']=$id
$testApp=[Diagnostics.Process]::Start($info)
$script:connections=0
function Reject-Connections {
    while($proxy.Pending()) {
        $client=$proxy.AcceptTcpClient()
        $client.Dispose()
        $script:connections++
    }
}
function Nodes {
    Reject-Connections
    if($testApp.HasExited){throw 'Isolated status app exited'}
    $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
}
function Window {
    $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
    $until=[DateTime]::UtcNow.AddSeconds(25)
    do {
        Reject-Connections
        if($testApp.HasExited){throw 'Settings process exited'}
        $script:window=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {$_.Current.Name -like 'QuotaScope *' -and -not $_.Current.IsOffscreen}|Select-Object -First 1
        if($window){return}
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    throw 'Status settings window missing'
}
function Wait-Text([string]$name,[switch]$Absent,[switch]$Contains) {
    $until=[DateTime]::UtcNow.AddSeconds(25)
    do {
        $found=@(Nodes|Where-Object {if($Contains){$_.Current.Name.Contains($name)}else{$_.Current.Name -eq $name}}).Count -gt 0
        if($found -ne $Absent.IsPresent){return}
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    throw "Status text condition failed: $name (absent=$Absent)"
}
function Click([string]$name) {
    Wait-Text $name
    $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1
    $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}
function Page([string]$name) {
    $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
    if(-not $node){throw "Missing navigation: $name"}
    $node.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
function Configure([string]$provider) {
    Page '账户'
    $until=[DateTime]::UtcNow.AddSeconds(15)
    do {
        $search=Nodes|Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit -and $_.Current.ClassName -eq 'TextBox'}|Select-Object -First 1
        if($search){break}; Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if(-not $search){throw 'Status search field missing'}
    $search.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($provider)
    do {
        $controls=@(Nodes|Where-Object {$_.Current.Name -in @('配置','收起') -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button})
        if($controls.Count -eq 1){break}; Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if($controls.Count -ne 1){throw 'Search did not isolate status provider'}
    if($controls[0].Current.Name -eq '配置'){$controls[0].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()}
    Wait-Text '服务状态'
    $company=switch($provider){'Codex'{'OpenAI'};'Claude Code'{'Anthropic'};'DeepSeek'{'DeepSeek'}}
    Wait-Text "由 $company 发布" -Contains
}
function Toggle([string]$name,[bool]$value) {
    Wait-Text $name
    $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ClassName -eq 'ToggleSwitch'}|Select-Object -First 1
    if(-not $node){throw "Toggle missing: $name"}
    $pattern=$node.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
    $wanted=if($value){[Windows.Automation.ToggleState]::On}else{[Windows.Automation.ToggleState]::Off}
    if($pattern.Current.ToggleState -ne $wanted){$pattern.Toggle()}
}
function Wait-RetryFailed([int]$before) {
    $until=[DateTime]::UtcNow.AddSeconds(25)
    do {
        $names=@(Nodes|ForEach-Object {$_.Current.Name})
        if($connections -gt $before -and $names -contains $failure -and $names -notcontains '正在检查服务状态…'){return}
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    throw 'Manual status retry did not complete with an explicit failure'
}
$failure='无法读取状态页。下方如有此前读数，它已经过时，尚未确认恢复。'
try {
    $phase='off-by-default'
    Window
    if($ProbePublicPages) {
        $current=@();$history=@();$uptimePages=@();$unreadable=@();$partialHistory=@()
        foreach($provider in @('Codex','Claude Code','DeepSeek')) {
            $phase='public-page-'+$provider
            Configure $provider
            $until=[DateTime]::UtcNow.AddSeconds(45)
            do {
                $names=@(Nodes|ForEach-Object {$_.Current.Name})
                $busy=$names -contains '正在检查服务状态…'
                $hasCurrent=@($names|Where-Object {$_ -like '状态检查时间：*'}).Count -gt 0
                $failed=$names -contains $failure
                $bars=@($names|Where-Object {$_ -match '^\d{4}-\d{2}-\d{2} · '})
                $uptimes=@($names|Where-Object {$_ -match '官方可用率：'})
                $historyGap=$names -contains '部分状态历史无法读取。保留可确认的当前状态，不估算可用率。'
                $noHistory=$names -contains '暂无状态历史数据。'
                if(-not $busy -and ($failed -or ($hasCurrent -and ($bars.Count -gt 0 -or $historyGap -or $noHistory)))){break}
                Start-Sleep -Milliseconds 100
            } while([DateTime]::UtcNow -lt $until)
            if($busy -or (-not $hasCurrent -and -not $failed)){throw 'Public status request did not finish'}
            if($hasCurrent -and -not $failed) {
                $current+=$provider
                if($bars.Count -gt 0){$history+=$provider}
                elseif($historyGap -or $noHistory){$partialHistory+=$provider}
                else{throw 'Readable public page displayed neither history nor an explicit history gap'}
                if($uptimes.Count -gt 0){$uptimePages+=$provider}
            } else {
                $unreadable+=$provider
                Wait-Text '运行正常' -Absent
            }
            Write-Output "Public page checked: $provider (current=$hasCurrent)"
        }
        [pscustomobject]@{Passed=$true;Sha256=(Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash;CurrentReadablePages=$current;HistoryReadablePages=$history;OfficialUptimePages=$uptimePages;UnreadablePages=$unreadable;PartialHistoryPages=$partialHistory;LiveStatusVerified=($current.Count -eq 3);AllHistoryVerified=($history.Count -eq 3);RealAccountVerified=$false;ExternalRequestsBlocked=$false}|ConvertTo-Json|Tee-Object -FilePath (Join-Path $evidence 'service-status-public-ui-result.json')
        return
    }
    Start-Sleep -Milliseconds 600
    Reject-Connections
    if($connections -ne 0){throw 'Status pages requested with every feature switched off'}
    $phase='independent-switch'
    Page '通知'
    Toggle '服务商发生故障或恢复时通知' $true
    $until=[DateTime]::UtcNow.AddSeconds(5)
    do {
        $saved=Get-Content -LiteralPath (Join-Path $data 'settings.json') -Raw|ConvertFrom-Json
        if($saved.alertsOnOutage){break}; Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if(-not $saved.alertsOnOutage -or $saved.wantsAlerts -or $saved.alertsOnFailure){throw 'Outage switch enabled quota alerts'}
    Reject-Connections
    if($connections -ne 0){throw 'Disabled providers were monitored'}
    $phase='codex-failed-read'
    Configure 'Codex'
    Wait-Text $failure
    Wait-Text '运行正常' -Absent
    Wait-Text '打开官方状态页'
    $before=$connections
    $phase='manual-retry'
    Click '刷新服务状态'
    Wait-RetryFailed $before
    if($connections -le $before){throw 'Manual refresh reused failed reading'}
    $phase='leave-during-request'
    Click '刷新服务状态'
    Page '常规'
    Wait-Text '服务状态' -Absent
    Configure 'Codex'
    Wait-Text $failure
    $phase='other-providers'
    Configure 'Claude Code'
    Wait-Text $failure
    Wait-Text '由 Anthropic 发布' -Contains
    Configure 'DeepSeek'
    Wait-Text $failure
    Wait-Text '由 DeepSeek 发布' -Contains
    $phase='close-reopen'
    Click '刷新服务状态'
    [void][StatusWindow]::PostMessage([IntPtr]$window.Current.NativeWindowHandle,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    $reopen=[Diagnostics.Process]::Start($info)
    if(-not $reopen.WaitForExit(15000)){throw 'Status reopen did not hand off'}
    Window
    Configure 'Codex'
    Wait-Text $failure
    Page '通知'
    Toggle '服务商发生故障或恢复时通知' $false
    Start-Sleep -Milliseconds 200
    $saved=Get-Content -LiteralPath (Join-Path $data 'settings.json') -Raw|ConvertFrom-Json
    if($saved.alertsOnOutage -or $saved.wantsAlerts){throw 'Notification switches did not persist independently'}
    [pscustomobject]@{Passed=$true;Sha256=(Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash;IndependentOutageSwitch=$true;DisabledProvidersNotFetched=$true;FailedReadNotOperational=$true;ManualRetry=$true;ProviderSections=$true;LeaveAndReopen=$true;CloseAndReopen=$true;ExternalRequestsBlocked=$true;LiveStatusVerified=$false;RejectedConnections=$connections}|ConvertTo-Json|Tee-Object -FilePath (Join-Path $evidence 'service-status-ui-result.json')
} catch {
    $originalError=$_
    Write-Output "Failed phase: $phase"
    try {Nodes|ForEach-Object {[pscustomobject]@{Name=$_.Current.Name;Class=$_.Current.ClassName;Enabled=$_.Current.IsEnabled}}|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $evidence 'service-status-ui-failure.json')} catch {}
    throw $originalError
} finally {
    if(-not $testApp.HasExited){$testApp.Kill();$testApp.WaitForExit()}
    $proxy.Stop()
    $resolved=[IO.Path]::GetFullPath($profile)
    if((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')){throw 'Unexpected status profile path'}
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
