# Isolated Debug Token page: actual zstd frames, append, failure and repair.
param([Parameter(Mandatory=$true)][string]$Executable,[string]$Python='C:/Python314/python.exe')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
$id=[guid]::NewGuid().ToString('N')
$profile=Join-Path $env:TEMP ('qs-dsh-ui-'+$id)
$fixtureHome=Join-Path $profile 'home'
$data=Join-Path $profile 'appdata/QuotaScope'
$transcript=Join-Path $fixtureHome '.dsh/sessions/fixture/session.jsonl.zstd'
New-Item -ItemType Directory -Path $data,(Split-Path $transcript) -Force | Out-Null
$reserve=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$reserve.Start()
$settings=@{hasRun=$true;enabledAccounts=@();language='zh';readsTokenSpend=$true;checksForUpdates=$false;proxyMode='manual';proxyUrl="http://127.0.0.1:$($reserve.LocalEndpoint.Port)"}
[IO.File]::WriteAllText((Join-Path $data 'settings.json'),($settings|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$prices=@{fetched_at_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();prices=@{'fixture-dsh'=@{input=1.0;output=2.0}}}
[IO.File]::WriteAllText((Join-Path $data 'model-prices-4.json'),($prices|ConvertTo-Json -Depth 4))
function Write-Fixture([string]$mode) {
    $script=@'
import compression.zstd, json, pathlib, sys, time
path=pathlib.Path(sys.argv[1]);mode=sys.argv[2];at=int(time.time()*1000)
def event(seq,id,input,output,cache=0):
    return {'type':'assistant/message','seq':seq,'time':at,'data':{'message':{'id':id},'usage':{'inputTokens':input,'outputTokens':output,'cacheReadTokens':cache,'reasoningTokens':3}}}
def encode(rows):
    return compression.zstd.compress(('\n'.join(json.dumps(row,separators=(',',':')) for row in rows)+'\n').encode())
if mode=='base':
    base=event(2,'one',11,7,2)
    path.write_bytes(encode([{'type':'session','seedLength':2},{'type':'request/header','data':{'header':{'config':{'model':'fixture-dsh'}}}},event(1,'inherited',999,999),base,base]))
elif mode=='append':
    with path.open('ab') as stream: stream.write(encode([event(3,'two',4,6)]))
elif mode=='corrupt':
    with path.open('ab') as stream: stream.write(bytes.fromhex('28b52ffdffff'))
'@
    $script | & $Python - $transcript $mode
    if($LASTEXITCODE -ne 0){throw 'Synthetic zstd fixture failed'}
}
Write-Fixture 'base'
$info=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings');$info.UseShellExecute=$false
$info.Environment['APPDATA']=Join-Path $profile 'appdata'
$info.Environment['LOCALAPPDATA']=Join-Path $profile 'local'
$info.Environment['USERPROFILE']=$fixtureHome
$info.Environment['QUOTASCOPE_TEST_INSTANCE']=$id
$testApp=[Diagnostics.Process]::Start($info)
function Nodes {
    if($testApp.HasExited){throw 'Isolated app exited'}
    $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
}
function Wait-Total([int]$total) {
    $until=[DateTime]::UtcNow.AddSeconds(30)
    do {
        $nodes=@(Nodes)
        $label=$nodes|Where-Object {$_.Current.Name -eq 'Token 总量' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Text}|Select-Object -First 1
        $found=$false
        if($label){
            $parent=[Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($label)
            $found=@($parent.FindAll([Windows.Automation.TreeScope]::Children,[Windows.Automation.Condition]::TrueCondition)|Where-Object {$_.Current.Name -eq $total.ToString()}).Count -gt 0
        }
        if($found -and @($nodes|Where-Object {$_.Current.Name -in @('正在计算 Token 统计…','正在读取本机用量…')}).Count -eq 0){return}
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    throw "Expected Token total $total"
}
function Wait-Text([string]$name,[switch]$Absent) {
    $until=[DateTime]::UtcNow.AddSeconds(15)
    do {
        $found=@(Nodes|Where-Object {$_.Current.Name -eq $name}).Count -gt 0
        if($found -ne $Absent.IsPresent){return};Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    throw "Text condition failed: $name (absent=$Absent)"
}
function Refresh {
    $button=Nodes|Where-Object {$_.Current.Name -eq '刷新' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1
    $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}
function Agent([string]$name) {
    $until=[DateTime]::UtcNow.AddSeconds(5)
    do {
        $combos=@(Nodes|Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::ComboBox})
        if($combos.Count -eq 6){break};Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if($combos.Count -ne 6){throw 'Filters not ready'}
    $combos[1].GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    $container=$null
    if($combos[1].TryGetCurrentPattern([Windows.Automation.ItemContainerPattern]::Pattern,[ref]$container)){
        $target=$container.FindItemByProperty($null,[Windows.Automation.AutomationElement]::NameProperty,$name)
        if($target){
            $virtual=$null
            if($target.TryGetCurrentPattern([Windows.Automation.VirtualizedItemPattern]::Pattern,[ref]$virtual)){$virtual.Realize()}
            Start-Sleep -Milliseconds 150
            try {$target.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select();return}catch{}
        }
    }
    $combos[1].SetFocus()
    [Windows.Forms.SendKeys]::SendWait(($name.Split(' ')[0]))
    [Windows.Forms.SendKeys]::SendWait('{ENTER}')
    Start-Sleep -Milliseconds 100
    $selection=$combos[1].GetCurrentPattern([Windows.Automation.SelectionPattern]::Pattern).GetSelection()
    if(@($selection|Where-Object {$_.Current.Name -eq $name}).Count){return}
    $combos[1].GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do {
        $items=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id))
        $item=$items|Where-Object {$_.Current.Name -eq $name -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::ListItem}|Select-Object -First 1
        if($item){break}
        # WinUI virtualizes the 54 source choices. Advance using an observed
        # realized item, rather than assuming every item exists in the tree.
        $last=$items|Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::ListItem -and $_.Current.ClassName -eq 'ComboBoxItem'}|Select-Object -Last 1
        if($last){
            $scroll=$null
            if($last.TryGetCurrentPattern([Windows.Automation.ScrollItemPattern]::Pattern,[ref]$scroll)){$scroll.ScrollIntoView()}
            else {
                $ancestor=$last
                for($step=0;$step -lt 6;$step++){
                    $ancestor=[Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($ancestor)
                    if(-not $ancestor){break}
                    $scroll=$null
                    if($ancestor.TryGetCurrentPattern([Windows.Automation.ScrollPattern]::Pattern,[ref]$scroll) -and $scroll.Current.VerticallyScrollable){
                        $scroll.Scroll([Windows.Automation.ScrollAmount]::NoAmount,[Windows.Automation.ScrollAmount]::LargeIncrement);break
                    }
                }
            }
        }
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if(-not $item){throw "Missing agent filter: $name"}
    $item.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
$warning='这些来源的本机记录不完整：DeepSeek Harness。总量只覆盖可读取的计数。'
try {
    $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
    $until=[DateTime]::UtcNow.AddSeconds(15)
    do {
        $window=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {$_.Current.Name -like 'QuotaScope *'}|Select-Object -First 1
        if($window){break};Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if(-not $window){throw 'Settings window missing'}
    $page=Nodes|Where-Object {$_.Current.Name -eq 'Token 消耗' -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
    $page.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Wait-Total 20
    Agent 'DeepSeek Harness'
    Wait-Total 20
    Wait-Text $warning -Absent
    Write-Fixture 'append'
    Wait-Total 20
    Refresh
    Wait-Total 30
    Write-Fixture 'corrupt'
    Refresh
    Wait-Text $warning
    Wait-Text 'DeepSeek Harness · 部分本机记录无法读取'
    Agent 'Codex'
    Wait-Total 0
    Wait-Text $warning -Absent
    Agent 'DeepSeek Harness'
    Wait-Text $warning
    Write-Fixture 'base';Write-Fixture 'append'
    Refresh
    Wait-Total 30
    Wait-Text $warning -Absent
    [pscustomobject]@{Passed=$true;Executable=[IO.Path]::GetFullPath($Executable);Sha256=(Get-FileHash -LiteralPath $Executable).Hash;ActualZstdFrames=$true;InheritedAndDuplicateRecordsSkipped=$true;ReasoningNotAddedTwice=$true;ConcatenatedAppend=$true;SourceFilter=$true;PartialWarningAndRepair=$true;OfflineSyntheticProfile=$true;RealAccountVerified=$false}|ConvertTo-Json|Tee-Object -FilePath 'target/upstream-followthrough-20261004/n4a-ui-result.json'
} catch {
    $originalError=$_
    try {if($window -and -not $testApp.HasExited){Nodes|ForEach-Object {[pscustomobject]@{Name=$_.Current.Name;Class=$_.Current.ClassName}}|ConvertTo-Json|Set-Content -LiteralPath 'target/upstream-followthrough-20261004/n4a-ui-failure.json'}}catch{}
    throw $originalError
} finally {
    if(-not $testApp.HasExited){$testApp.Kill();$testApp.WaitForExit()}
    $reserve.Stop()
    $resolved=[IO.Path]::GetFullPath($profile)
    if((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')){throw 'Unexpected test profile'}
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
