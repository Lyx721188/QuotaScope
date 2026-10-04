# Isolated Token page: native SQLite, retries, missing usage, replacement and reopen.
param([Parameter(Mandatory=$true)][string]$Executable,[string]$Python='python',[string]$OutputDirectory='target/release-acceptance-20261004')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
$id=[guid]::NewGuid().ToString('N')
$profile=Join-Path $env:TEMP ('qs-zcode-ui-'+$id)
$fixtureHome=Join-Path $profile 'home'
$data=Join-Path $profile 'appdata/QuotaScope'
$transcript=Join-Path $fixtureHome '.zcode/cli/db/db.sqlite'
New-Item -ItemType Directory -Path $data,(Split-Path $transcript),$OutputDirectory -Force | Out-Null
$reserve=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$reserve.Start()
$settings=@{hasRun=$true;enabledAccounts=@();language='zh';readsTokenSpend=$true;checksForUpdates=$false;proxyMode='manual';proxyUrl="http://127.0.0.1:$($reserve.LocalEndpoint.Port)"}
[IO.File]::WriteAllText((Join-Path $data 'settings.json'),($settings|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$prices=@{fetched_at_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();prices=@{'fixture-zcode'=@{input=1.0;output=2.0}}}
[IO.File]::WriteAllText((Join-Path $data 'model-prices-4.json'),($prices|ConvertTo-Json -Depth 4))
function Write-Fixture([string]$mode) {
    Write-Host "Synthetic SQLite fixture: $mode"
    $script=@'
import json, os, pathlib, sqlite3, sys, time
from contextlib import closing
path=pathlib.Path(sys.argv[1]);mode=sys.argv[2];at=int(time.time()*1000)
schema="""CREATE TABLE model_usage (
 id TEXT PRIMARY KEY, logical_request_id TEXT, attempt_index INTEGER, model_id TEXT,
 status TEXT, started_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
 reasoning_tokens INTEGER, cache_creation_input_tokens INTEGER, cache_read_input_tokens INTEGER,
 computed_total_tokens INTEGER, provider_total_tokens INTEGER, raw_usage_json TEXT);
 CREATE INDEX model_usage_started_model_idx ON model_usage(started_at,model_id);"""
def raw(i,o,w=0,r=0):
    return json.dumps(dict(inputTokens=i,outputTokens=o,cacheWriteTokens=w,cacheReadTokens=r,reasoningTokens=3,totalTokens=i+o),separators=(',',':'))
def row(id,attempt,i,o,w=0,r=0):
    return (id,'request',attempt,'fixture-zcode','completed',at,i,o,3,w,r,i+o,i+o,raw(i,o,w,r))
if mode=='delete':
    path.unlink()
elif mode=='replace':
    stamp=path.stat();replacement=path.with_suffix('.replacement')
    with closing(sqlite3.connect(path)) as old, closing(sqlite3.connect(replacement)) as new:
        old.backup(new)
        new.execute('UPDATE model_usage SET input_tokens=200,computed_total_tokens=220,provider_total_tokens=220,raw_usage_json=? WHERE id=?',(raw(200,20,10,30),'one'))
        new.commit()
    assert replacement.stat().st_size==stamp.st_size
    os.utime(replacement,ns=(stamp.st_atime_ns,stamp.st_mtime_ns))
    os.replace(replacement,path)
else:
    with sqlite3.connect(path) as db:
        if mode=='base':
            db.executescript('DROP TABLE IF EXISTS model_usage;'+schema)
            db.execute('INSERT INTO model_usage VALUES ('+','.join('?'*14)+')',row('one',0,100,20,10,30))
        elif mode=='append':
            db.execute('INSERT INTO model_usage VALUES ('+','.join('?'*14)+')',row('retry',1,40,10))
        elif mode=='missing':
            db.execute('UPDATE model_usage SET raw_usage_json=NULL WHERE id=?',('retry',))
        elif mode=='unpriced':
            db.execute('UPDATE model_usage SET model_id=? WHERE id=?',('unpublished-fixture-7142','retry'))
        elif mode=='repair':
            db.execute('UPDATE model_usage SET raw_usage_json=? WHERE id=?',(raw(40,10),'retry'))
        else: raise ValueError('Unknown fixture mode')
'@
    $script | & $Python - $transcript $mode
    if($LASTEXITCODE -ne 0){throw 'Synthetic SQLite fixture failed'}
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
    # WinUI can replace its automation provider when a filter rebuilds the
    # page. Reacquire the live process window rather than retaining that tree.
    $until=[DateTime]::UtcNow.AddSeconds(5)
    do {
        if($testApp.HasExited){throw 'Isolated app exited'}
        try {
            $script:window=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {$_.Current.Name -like 'QuotaScope *' -and -not $_.Current.IsOffscreen}|Select-Object -First 1
            if($window){return $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)}
        } catch {if($testApp.HasExited){throw}}
        Start-Sleep -Milliseconds 50
    } while([DateTime]::UtcNow -lt $until)
    throw 'Live settings automation tree unavailable'
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
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do {
        $button=Nodes|Where-Object {$_.Current.Name -eq '刷新' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button -and $_.Current.IsEnabled}|Select-Object -First 1
        if($button){
            try {$button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke();return}
            catch [Windows.Automation.ElementNotAvailableException] {}
        }
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    throw 'Enabled refresh button unavailable'
}
function Choose-Filter([int]$index,[string]$name) {
    $until=[DateTime]::UtcNow.AddSeconds(5)
    do {
        $combos=@(Nodes|Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::ComboBox})
        if($combos.Count -eq 6){break};Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if($combos.Count -ne 6){throw 'Filters not ready'}
    $combos[$index].GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    $container=$null
    if($combos[$index].TryGetCurrentPattern([Windows.Automation.ItemContainerPattern]::Pattern,[ref]$container)){
        $target=$container.FindItemByProperty($null,[Windows.Automation.AutomationElement]::NameProperty,$name)
        if($target){
            $virtual=$null
            if($target.TryGetCurrentPattern([Windows.Automation.VirtualizedItemPattern]::Pattern,[ref]$virtual)){$virtual.Realize()}
            Start-Sleep -Milliseconds 150
            try {$target.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select();return}catch{}
        }
    }
    $combos[$index].SetFocus()
    [Windows.Forms.SendKeys]::SendWait(($name.Split(' ')[0]))
    [Windows.Forms.SendKeys]::SendWait('{ENTER}')
    Start-Sleep -Milliseconds 100
    $selection=$combos[$index].GetCurrentPattern([Windows.Automation.SelectionPattern]::Pattern).GetSelection()
    if(@($selection|Where-Object {$_.Current.Name -eq $name}).Count){return}
    $combos[$index].GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
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
function Agent([string]$name) { Choose-Filter 1 $name }
function Write-Imports([string]$mode) {
    $folder=Join-Path $data 'UsageImports/zcode'
    $path=Join-Path $folder 'usage.jsonl'
    $jsonPath=Join-Path $folder 'usage.json'
    if($mode -eq 'clear') { Remove-Item -LiteralPath $path,$jsonPath -ErrorAction SilentlyContinue; return }
    New-Item -ItemType Directory -Path $folder -Force | Out-Null
    $row=@{schema='quotascope.usage.v1';source='zcode';id='import-fixture';timestamp=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();model='fixture-zcode';usage=@{inputTokens=10;outputTokens=7;cacheReadTokens=5;cacheWriteTokens=3;reasoningTokens=4;totalTokens=25}}
    $lines=@($row|ConvertTo-Json -Depth 4 -Compress)
    if($mode -in @('array','torn-array')) {
        if(Test-Path -LiteralPath $path){Remove-Item -LiteralPath $path}
        $text='['+$lines[0]+',{'
        if($mode -eq 'array') {
            $row.usage=@{inputTokens=2;outputTokens=3}
            $text='['+$lines[0]+','+($row|ConvertTo-Json -Depth 4 -Compress)+']'
        }
        [IO.File]::WriteAllText($jsonPath,$text,[Text.UTF8Encoding]::new($false))
        return
    }
    if(Test-Path -LiteralPath $jsonPath){Remove-Item -LiteralPath $jsonPath}
    if($mode -eq 'wide-calendar') {
        $row.id='old-calendar';$row.timestamp='1970-01-01T12:00:00Z';$row.usage=@{inputTokens=11;outputTokens=0}
        $lines=@($row|ConvertTo-Json -Depth 4 -Compress)
        $row.id='future-calendar';$row.timestamp='9999-01-01T12:00:00Z';$row.usage=@{inputTokens=29;outputTokens=0}
        $lines+=($row|ConvertTo-Json -Depth 4 -Compress)
    } elseif($mode -eq 'invalid') {
        $row.id='missing-output';$row.usage=@{inputTokens=999}
        $lines+=($row|ConvertTo-Json -Depth 4 -Compress)
    } elseif($mode -eq 'zero') {
        $row.usage=@{inputTokens=0;outputTokens=0}
        $lines+=($row|ConvertTo-Json -Depth 4 -Compress)
    } elseif($mode -eq 'unknown') {
        $row.id='unknown-model';$row.Remove('model');$row.usage=@{inputTokens=2;outputTokens=3}
        $lines+=($row|ConvertTo-Json -Depth 4 -Compress)
    }
    [IO.File]::WriteAllText($path,($lines -join "`n")+"`n",[Text.UTF8Encoding]::new($false))
}
$warning='这些来源的本机记录不完整：ZCode。总量只覆盖可读取的计数。'
try {
    $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
    $until=[DateTime]::UtcNow.AddSeconds(15)
    do {
        $window=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {$_.Current.Name -like 'QuotaScope *' -and -not $_.Current.IsOffscreen}|Select-Object -First 1
        if($window){break};Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if(-not $window){throw 'Settings window missing'}
    $page=Nodes|Where-Object {$_.Current.Name -eq 'Token 消耗' -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
    $page.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Wait-Total 120
    Agent 'ZCode'
    Wait-Total 120
    Wait-Text $warning -Absent
    Choose-Filter 0 '今日'
    Wait-Total 120
    Choose-Filter 2 'fixture-zcode'
    Wait-Total 120
    Choose-Filter 3 '模型'
    Choose-Filter 4 '输入'
    Choose-Filter 5 '升序'
    Wait-Total 120
    Write-Fixture 'append'
    Wait-Total 120
    Refresh
    Wait-Total 170
    Write-Fixture 'missing'
    Refresh
    Wait-Total 120
    Wait-Text $warning
    Wait-Text 'ZCode · 部分本机记录无法读取'
    Agent 'Codex'
    Wait-Total 0
    Wait-Text $warning -Absent
    Agent 'ZCode'
    Wait-Total 120
    Wait-Text $warning
    Write-Fixture 'repair'
    Refresh
    Wait-Total 170
    Wait-Text $warning -Absent
    Write-Fixture 'unpriced'
    Choose-Filter 2 '全部模型'
    Refresh
    Wait-Total 170
    Wait-Text '未公开价格的 Token： 50'
    Choose-Filter 2 'unpublished-fixture-7142'
    Wait-Total 50
    Choose-Filter 2 '全部模型'
    Wait-Total 170
    Write-Fixture 'replace'
    Refresh
    Wait-Total 270
    Write-Fixture 'delete'
    Refresh
    Wait-Total 0
    Write-Fixture 'base'
    Refresh
    Wait-Total 120
    Write-Imports 'valid'
    Refresh
    Wait-Total 145
    Wait-Text $warning -Absent
    Write-Imports 'invalid'
    Refresh
    Wait-Total 145
    Wait-Text $warning
    Write-Imports 'zero'
    Refresh
    Wait-Total 120
    Wait-Text $warning -Absent
    Write-Imports 'unknown'
    Refresh
    Wait-Total 150
    Wait-Text $warning
    Wait-Text '未公开价格的 Token： 5'
    Write-Imports 'clear'
    Refresh
    Wait-Total 120
    Wait-Text $warning -Absent
    Write-Imports 'torn-array'
    Refresh
    Wait-Total 145
    Wait-Text $warning
    Write-Imports 'array'
    Refresh
    Wait-Total 125
    Wait-Text $warning -Absent
    Write-Imports 'clear'
    Refresh
    Wait-Total 120
    Wait-Text $warning -Absent
    Write-Imports 'wide-calendar'
    Refresh
    Wait-Total 120
    Wait-Text $warning -Absent
    Choose-Filter 0 '全部时间'
    Wait-Total 131
    Choose-Filter 0 '今日'
    Wait-Total 120
    Write-Imports 'clear'
    Refresh
    Wait-Total 120
    $window.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close()
    Start-Sleep -Milliseconds 200
    if($testApp.HasExited){throw 'App exited on Settings close'}
    $reopen=[Diagnostics.Process]::Start($info)
    if(-not $reopen.WaitForExit(15000)){throw 'Second settings request did not exit'}
    if($reopen.ExitCode -ne 0){throw 'Second settings request failed'}
    $until=[DateTime]::UtcNow.AddSeconds(15)
    do {
        $window=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {$_.Current.Name -like 'QuotaScope *' -and -not $_.Current.IsOffscreen}|Select-Object -First 1
        if($window){break};Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if(-not $window){throw 'Reopened settings window missing'}
    $page=Nodes|Where-Object {$_.Current.Name -eq 'Token 消耗' -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
    $page.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Agent 'ZCode'
    Wait-Total 120
    Wait-Text $warning -Absent
    [pscustomobject]@{Passed=$true;Executable=[IO.Path]::GetFullPath($Executable);Sha256=(Get-FileHash -LiteralPath $Executable).Hash;NativeSqlite=$true;RetriesCounted=$true;ReasoningNotAddedTwice=$true;RefreshAppend=$true;SourceFilter=$true;TimeModelAndSortFilters=$true;UnpricedModel=$true;PartialWarningAndRepair=$true;SameSizeSameMtimeReplacement=$true;Deletion=$true;StandardImportCounters=$true;ZeroSnapshotReplacesUsage=$true;UnknownImportModelUnpriced=$true;StreamedJsonArrayAndRepair=$true;WideCalendarAndFutureFiltering=$true;SettingsCloseAndReopen=$true;OfflineSyntheticProfile=$true;RealAccountVerified=$false}|ConvertTo-Json|Tee-Object -FilePath (Join-Path $OutputDirectory 'zcode-ui-result.json')
} catch {
    $originalError=$_
    try {if($window -and -not $testApp.HasExited){Nodes|ForEach-Object {[pscustomobject]@{Name=$_.Current.Name;Class=$_.Current.ClassName}}|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $OutputDirectory 'zcode-ui-failure.json')}}catch{}
    throw $originalError
} finally {
    if(-not $testApp.HasExited){$testApp.Kill();$testApp.WaitForExit()}
    $reserve.Stop()
    $resolved=[IO.Path]::GetFullPath($profile)
    if((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')){throw 'Unexpected test profile'}
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
