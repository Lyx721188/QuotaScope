# Run from windows/ with a complete payload and an unlocked desktop.
# Synthetic rollout facts only; no credentials or real account requests.
param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [string]$OutputDirectory='target/codex-account-ui-validation'
)
$ErrorActionPreference='Stop'
$evidence=[IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SignalWindow {
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wparam, IntPtr lparam);
}
'@
$testId=[guid]::NewGuid().ToString('N')
$profile=Join-Path $env:TEMP ('qs-codex-signals-'+$testId)
$data=Join-Path $profile 'appdata/QuotaScope'
$fixtureHome=Join-Path $profile 'home'
$sessionRoot=Join-Path $fixtureHome '.codex/sessions'
$archiveRoot=Join-Path $fixtureHome '.codex/archived_sessions'
New-Item -ItemType Directory -Path $data,$sessionRoot,$archiveRoot -Force | Out-Null
$reserve=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$reserve.Start()
$port=$reserve.LocalEndpoint.Port
$settings=@{hasRun=$true;enabledAccounts=@();language='zh';readsTokenSpend=$true;checksForUpdates=$false;proxyMode='manual';proxyUrl="http://127.0.0.1:$port"}
[IO.File]::WriteAllText((Join-Path $data 'settings.json'),($settings|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$prices=@{fetched_at_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();prices=@{'fixture-alpha'=@{input=1.0;output=2.0};'fixture-beta'=@{input=1.0;output=2.0};'fixture-gamma'=@{input=1.0;output=2.0}}}
[IO.File]::WriteAllText((Join-Path $data 'model-prices-4.json'),($prices|ConvertTo-Json -Depth 4))
function Write-Rollout([string]$path,[int]$age,[string]$model,[int]$responses,[int]$hits,[string]$asked=$model) {
    $stamp=[DateTime]::UtcNow.Date.AddDays(-$age).AddHours(12).ToString('o')
    $writer=[IO.StreamWriter]::new($path,$false,[Text.UTF8Encoding]::new($false))
    try {
        $writer.WriteLine((@{timestamp=$stamp;type='session_meta';payload=@{cli_version='0.146.0'}}|ConvertTo-Json -Depth 6 -Compress))
        $writer.WriteLine((@{timestamp=$stamp;type='event_msg';payload=@{type='thread_settings_applied';thread_settings=@{model=$asked;reasoning_effort='high'}}}|ConvertTo-Json -Depth 6 -Compress))
        $writer.WriteLine((@{timestamp=$stamp;type='event_msg';payload=@{type='task_started';turn_id='root';root_turn_id='root'}}|ConvertTo-Json -Depth 6 -Compress))
        $writer.WriteLine((@{timestamp=$stamp;type='turn_context';payload=@{turn_id='root';model=$model;effort='high'}}|ConvertTo-Json -Depth 6 -Compress))
        for($n=0;$n -lt $responses;$n++) {
            $reasoning=if($n -lt $hits){516}else{700}
            $writer.WriteLine((@{timestamp=$stamp;type='event_msg';payload=@{type='token_count';info=@{last_token_usage=@{reasoning_output_tokens=$reasoning};total_token_usage=@{input_tokens=100*($n+1);output_tokens=50*($n+1);reasoning_output_tokens=$reasoning*($n+1);total_tokens=150*($n+1)}}}}|ConvertTo-Json -Depth 7 -Compress))
        }
    } finally {$writer.Dispose()}
}
$young=Join-Path $sessionRoot 'rollout-alpha.jsonl'
Write-Rollout $young 0 'fixture-alpha' 20 6
Write-Rollout (Join-Path $archiveRoot 'rollout-beta.jsonl') 60 'fixture-beta' 3 3 'fixture-requested-beta'
Write-Rollout (Join-Path $archiveRoot 'rollout-gamma.jsonl') 120 'fixture-gamma' 1 0
$timingDamage=Join-Path $sessionRoot 'damaged-timings.jsonl'
[IO.File]::WriteAllText($timingDamage,"{`n",[Text.UTF8Encoding]::new($false))
function Write-FirstTokenTimings([string]$path,[DateTime]$at,[int]$milliseconds) {
    $writer=[IO.StreamWriter]::new($path,$false,[Text.UTF8Encoding]::new($false))
    try {
        $writer.WriteLine('{"type":"turn_context","payload":{"model":"fixture-alpha"}}')
        for($n=0;$n -lt 3;$n++) {
            $writer.WriteLine((@{timestamp=$at.ToString('o');type='event_msg';payload=@{type='task_complete';turn_id=('timing-'+$path+'-'+$n);time_to_first_token_ms=$milliseconds}}|ConvertTo-Json -Depth 5 -Compress))
        }
    } finally {$writer.Dispose()}
}
$futureTimings=Join-Path $sessionRoot 'future-timings.jsonl'
Write-FirstTokenTimings $futureTimings ([DateTime]::UtcNow.AddDays(1)) 10000
$info=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings')
$info.UseShellExecute=$false
$info.Environment['APPDATA']=Join-Path $profile 'appdata'
$info.Environment['LOCALAPPDATA']=Join-Path $profile 'local'
$info.Environment['USERPROFILE']=$fixtureHome
$info.Environment['HERMES_HOME']=Join-Path ($info.Environment['USERPROFILE']) '.hermes'
$info.Environment['QUOTASCOPE_TEST_INSTANCE']=$testId
$testApp=[Diagnostics.Process]::Start($info)
function Nodes {
    if($testApp.HasExited){throw 'Isolated app exited'}
    try { $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition) }
    catch {
        Window
        $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
    }
}
function Wait-Text([string]$text,[switch]$Absent,[switch]$Contains) {
    $until=[DateTime]::UtcNow.AddSeconds(20)
    do {
        $found=@(Nodes|Where-Object {if($Contains){$_.Current.Name.Contains($text)}else{$_.Current.Name -eq $text}}).Count -gt 0
        if($found -ne $Absent.IsPresent){return}
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    throw "UI text condition failed: $text (absent=$Absent)"
}
function Node([string]$name) {
    Wait-Text $name
    Nodes|Where-Object {$_.Current.Name -eq $name}|Select-Object -First 1
}
function Click([string]$name) {
    $node=Node $name
    $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}
function Page([string]$name) {
    $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
    if(-not $node){throw "Missing navigation: $name"}
    $node.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
function Window {
    $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
    $until=[DateTime]::UtcNow.AddSeconds(20)
    do {
        if($testApp.HasExited){throw 'Settings process exited'}
        $script:window=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {$_.Current.Name -like 'QuotaScope *' -and -not $_.Current.IsOffscreen}|Select-Object -First 1
        if($window){return}
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    throw 'Settings window missing'
}
function Configure {
    Page '账户'
    $until=[DateTime]::UtcNow.AddSeconds(15)
    do {
        $search=Nodes|Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit -and $_.Current.ClassName -eq 'TextBox'}|Select-Object -First 1
        if($search){break}; Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if(-not $search){throw 'Search field missing'}
    $search.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('Codex')
    do {
        $controls=@(Nodes|Where-Object {$_.Current.Name -in @('配置','收起') -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button})
        if($controls.Count -eq 1){break}; Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if($controls.Count -ne 1){throw 'Search did not isolate Codex'}
    if($controls[0].Current.Name -eq '配置'){$controls[0].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()}
    Wait-Text '本地异常线索'
}
function Refresh-Timings {
    # ControlView flattens the layout panels. The live tree places this
    # section's Refresh immediately after its heading, before model rows.
    Wait-Text '按模型查看用量（最近 30 天）'
    $seenHeading=$false
    foreach($node in @(Nodes)) {
        if($node.Current.Name -eq '按模型查看用量（最近 30 天）'){$seenHeading=$true;continue}
        if($seenHeading -and $node.Current.Name -eq '刷新' -and $node.Current.ControlType -eq [Windows.Automation.ControlType]::Button){
            $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
            return
        }
    }
    throw 'Model timing refresh button missing'
}
function Period([string]$label) {
    $until=[DateTime]::UtcNow.AddSeconds(5)
    do {
        $combo=Nodes|Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::ComboBox}|Select-Object -Last 1
        if($combo){break}; Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if(-not $combo){throw 'Period selector not ready'}
    $combo.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    $until=[DateTime]::UtcNow.AddSeconds(5)
    do {
        $items=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id))
        $item=$items|Where-Object {$_.Current.Name -eq $label -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::ListItem}|Select-Object -First 1
        if($item){break}; Start-Sleep -Milliseconds 50
    } while([DateTime]::UtcNow -lt $until)
    if(-not $item){throw "Period missing: $label"}
    $item.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
$alpha20='fixture-alpha · 正推理回复 20 · 格点 6/20（30.0%） · 格点集中'
$alpha21='fixture-alpha · 正推理回复 21 · 格点 6/21（28.6%） · 格点集中'
$beta='fixture-beta · 正推理回复 3 · 格点 3/3（100.0%） · 样本不足'
$gamma='fixture-gamma · 正推理回复 1 · 格点 0/1（0.0%） · 样本不足'
try {
    $phase='initial'
    Window
    Configure
    Wait-Text $alpha20
    Wait-Text $beta -Absent
    $phase='timing-partial-and-repair'
    $timingWarning='部分本机时序记录无法读取；速度只覆盖可读取的回复。'
    Wait-Text $timingWarning
    Wait-Text 'fixture-alpha · 占比 100.0% · 3000 tokens' -Contains
    Wait-Text '计数可能不完整。' -Absent
    Remove-Item -LiteralPath $timingDamage -Force
    Refresh-Timings
    Start-Sleep -Milliseconds 200
    Wait-Text 'fixture-alpha · 占比 100.0% · 3000 tokens' -Contains
    Wait-Text $timingWarning -Absent
    $phase='native-counter-gap-and-repair'
    $nativeGap=Join-Path $sessionRoot 'native-counter-gap.jsonl'
    $gapRecord=@{timestamp=[DateTime]::UtcNow.ToString('o');type='event_msg';payload=@{type='token_count';info=@{total_token_usage=@{input_tokens=-1;output_tokens=0}}}}|ConvertTo-Json -Depth 6 -Compress
    [IO.File]::WriteAllText($nativeGap,('{"type":"turn_context","payload":{"model":"fixture-alpha"}}'+"`n"+$gapRecord+"`n"),[Text.UTF8Encoding]::new($false))
    Refresh-Timings
    Wait-Text '计数可能不完整。'
    Wait-Text 'fixture-alpha · 占比 100.0% · 3000 tokens' -Contains
    Wait-Text $timingWarning -Absent
    Remove-Item -LiteralPath $nativeGap -Force
    Refresh-Timings
    Wait-Text '计数可能不完整。' -Absent
    Wait-Text 'fixture-alpha · 占比 100.0% · 3000 tokens' -Contains
    Wait-Text '首字 — 秒' -Contains
    $phase='native-discovery-gap-and-repair'
    $deepRoot=Join-Path $sessionRoot 'discovery-gap'
    $deep=$deepRoot
    for($n=0;$n -lt 64;$n++) {$deep=Join-Path $deep 'a'}
    New-Item -ItemType Directory -Path $deep -Force | Out-Null
    Copy-Item -LiteralPath $young -Destination (Join-Path $deep 'native-depth.jsonl')
    Refresh-Timings
    Wait-Text '计数可能不完整。'
    Wait-Text 'fixture-alpha · 占比 100.0% · 3000 tokens' -Contains
    $resolvedDeep=[IO.Path]::GetFullPath($deepRoot)
    $ownedSession=[IO.Path]::GetFullPath($sessionRoot).TrimEnd([IO.Path]::DirectorySeparatorChar)+[IO.Path]::DirectorySeparatorChar
    if(-not $resolvedDeep.StartsWith($ownedSession,[StringComparison]::OrdinalIgnoreCase)) {throw 'Discovery fixture escaped the owned session directory'}
    Remove-Item -LiteralPath $resolvedDeep -Recurse -Force
    Refresh-Timings
    Wait-Text '计数可能不完整。' -Absent
    Wait-Text 'fixture-alpha · 占比 100.0% · 3000 tokens' -Contains
    Wait-Text $timingWarning -Absent
    $phase='valid-and-future-timings'
    $currentTimings=Join-Path $sessionRoot 'current-timings.jsonl'
    Write-FirstTokenTimings $currentTimings ([DateTime]::UtcNow.AddMinutes(-2)) 2000
    Refresh-Timings
    Wait-Text '首字 2.00 秒' -Contains
    Wait-Text 'fixture-alpha · 占比 100.0% · 3000 tokens' -Contains
    Wait-Text $timingWarning -Absent
    $phase='90-days'
    Period '最近 90 个自然日'
    Wait-Text $beta
    Wait-Text '所选模型 fixture-requested-beta → 记录模型 fixture-beta' -Contains
    $phase='all-records'
    Period '全部本机记录'
    Wait-Text $gamma
    $phase='30-days'
    Period '最近 30 个自然日'
    Wait-Text $alpha20
    Wait-Text $beta -Absent
    $phase='manual-refresh'
    Write-Rollout $young 0 'fixture-alpha' 21 6
    Wait-Text $alpha20
    Click '刷新本地线索'
    Wait-Text $alpha21
    $large=Join-Path $sessionRoot 'rollout-large.jsonl'
    $stamp=[DateTime]::UtcNow.ToString('o')
    $writer=[IO.StreamWriter]::new($large,$false,[Text.UTF8Encoding]::new($false))
    try {
        $writer.WriteLine('{"type":"session_meta","payload":{"cli_version":"0.146.0"}}')
        $writer.WriteLine('{"type":"turn_context","payload":{"model":"fixture-large"}}')
        $line=(@{timestamp=$stamp;type='event_msg';payload=@{type='token_count';info=@{last_token_usage=@{reasoning_output_tokens=516};total_token_usage=@{total_tokens=100}}}}|ConvertTo-Json -Depth 6 -Compress)+"`n"
        $chunk=$line*1000
        for($n=0;$n -lt 400;$n++){$writer.Write($chunk)}
    } finally {$writer.Dispose()}
    $phase='cancel-long-read'
    Click '刷新本地线索'
    Wait-Text '正在读取本地线索…'
    $cancelWatch=[Diagnostics.Stopwatch]::StartNew()
    Click '取消本地读取'
    Wait-Text '已取消本地读取；保留此前完整快照。'
    $cancelWatch.Stop()
    Wait-Text $alpha21
    $phase='leave-page'
    Click '刷新本地线索'
    Wait-Text '正在读取本地线索…'
    Page '常规'
    Remove-Item -LiteralPath $large -Force
    Configure
    Wait-Text $alpha21
    $phase='close-reopen'
    Click '刷新本地线索'
    [void][SignalWindow]::PostMessage([IntPtr]$window.Current.NativeWindowHandle,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    $reopen=[Diagnostics.Process]::Start($info)
    if(-not $reopen.WaitForExit(15000)){throw 'Second settings launch did not hand off'}
    Window
    Configure
    Wait-Text $alpha21
    $forbidden=@(Nodes|Where-Object {$_.Current.Name -match '服务端.*降级|实际模型.*fixture'})
    if($forbidden.Count){throw 'UI asserts an actual server model'}
    [pscustomobject]@{Passed=$true;Executable=[IO.Path]::GetFullPath($Executable);Sha256=(Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash;SyntheticRecords=$true;CalendarPeriods=$true;SamplesAndDenominator=$true;RecordedSettingsDifference=$true;ManualRefresh=$true;PartialTimingWarningAndRepair=$true;FutureTimingsExcluded=$true;MeasuredFirstTokenSeconds=2.0;NativeCounterWarningAndRepair=$true;NativeDiscoveryWarningAndRepair=$true;TokenCountsUnaffected=$true;CancellationSnapshotPreserved=$true;CancelUiMilliseconds=$cancelWatch.ElapsedMilliseconds;LeaveAndReopen=$true;CloseAndReopen=$true;RealAccountVerified=$false;ExternalRequestsBlocked=$true}|ConvertTo-Json|Tee-Object -FilePath (Join-Path $evidence 'codex-account-ui-result.json')
} catch {
    $originalError=$_
    Write-Output "Failed phase: $phase"
    try {
        if($window -and -not $testApp.HasExited){Nodes|ForEach-Object {[pscustomobject]@{Name=$_.Current.Name;Class=$_.Current.ClassName;Enabled=$_.Current.IsEnabled}}|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $evidence 'codex-account-ui-failure.json')}
    } catch {Write-Output 'UI tree unavailable during failure capture'}
    throw $originalError
} finally {
    if(-not $testApp.HasExited){$testApp.Kill();$testApp.WaitForExit()}
    $reserve.Stop()
    $resolved=[IO.Path]::GetFullPath($profile)
    if((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')){throw 'Unexpected profile path'}
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
