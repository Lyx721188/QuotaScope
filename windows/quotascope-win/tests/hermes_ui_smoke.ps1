# Isolated native Hermes cumulative/profile counts and coverage repair on Token page.
param([Parameter(Mandatory=$true)][string]$Executable,[string]$Python='python',[string]$OutputDirectory='target/hermes-ui-validation')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
$id=[guid]::NewGuid().ToString('N')
$profile=Join-Path $env:TEMP ('qs-hermes-ui-'+$id)
$fixtureHome=Join-Path $profile 'home'
$data=Join-Path $profile 'appdata/QuotaScope'
$hermes=Join-Path $profile 'hermes'
New-Item -ItemType Directory -Path $data,$fixtureHome,$hermes,$OutputDirectory -Force | Out-Null
$reserve=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
$reserve.Start()
$settings=@{hasRun=$true;enabledAccounts=@();language='zh';readsTokenSpend=$true;checksForUpdates=$false;proxyMode='manual';proxyUrl="http://127.0.0.1:$($reserve.LocalEndpoint.Port)"}
[IO.File]::WriteAllText((Join-Path $data 'settings.json'),($settings|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$prices=@{fetched_at_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();prices=@{'fixture-a'=@{input=1.0;output=2.0};'fixture-b'=@{input=1.0;output=2.0};'fixture-c'=@{input=1.0;output=2.0}}}
[IO.File]::WriteAllText((Join-Path $data 'model-prices-4.json'),($prices|ConvertTo-Json -Depth 4))
function Write-Fixture([string]$mode) {
    Write-Host "Synthetic Hermes fixture: $mode"
    $script=@'
import os, pathlib, sqlite3, sys, time
from contextlib import closing
root=pathlib.Path(sys.argv[1]).resolve(); mode=sys.argv[2]
temporary=pathlib.Path(os.environ['TEMP']).resolve()
assert root.parent.parent==temporary and root.parent.name.startswith('qs-hermes-ui-') and root.name=='hermes'
path=root/'state.db'; profile=root/'profiles/team/state.db'; at=time.time()
schema='CREATE TABLE sessions(id TEXT PRIMARY KEY, model TEXT, started_at REAL, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER); CREATE TABLE session_model_usage(session_id TEXT, model TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER);'
def main():
    with sqlite3.connect(path) as db:
        db.executescript('DROP TABLE IF EXISTS sessions;DROP TABLE IF EXISTS session_model_usage;'+schema)
        db.executemany('INSERT INTO sessions VALUES(?,?,?,?,?,?,?,?)',[('s','old-session-model',at,1000,1000,0,0,0),('solo','fixture-b',at,20,5,0,0,0)])
        db.execute('INSERT INTO session_model_usage VALUES(?,?,?,?,?,?,?)',('s','fixture-a',100,20,50,10,7))
def named():
    profile.parent.mkdir(parents=True,exist_ok=True)
    with sqlite3.connect(profile) as db:
        db.executescript('DROP TABLE IF EXISTS sessions;DROP TABLE IF EXISTS session_model_usage;'+schema)
        db.execute('INSERT INTO sessions VALUES(?,?,?,?,?,?,?,?)',('s','fixture-c',at,30,10,0,0,0))
if mode=='base': main();named()
elif mode=='profile-delete': profile.unlink()
elif mode=='profile-add': named()
elif mode=='delete': path.unlink()
elif mode=='recreate': main()
elif mode=='replace':
    stamp=path.stat();replacement=path.with_suffix('.replacement')
    with closing(sqlite3.connect(path)) as old,closing(sqlite3.connect(replacement)) as new:
        old.backup(new);new.execute('UPDATE session_model_usage SET input_tokens=200,cache_read_tokens=0,cache_write_tokens=0,reasoning_tokens=0');new.commit()
    assert replacement.stat().st_size==stamp.st_size
    os.utime(replacement,ns=(stamp.st_atime_ns,stamp.st_mtime_ns));os.replace(replacement,path)
else:
    updates={'repair':'cache_read_tokens=0,cache_write_tokens=0,reasoning_tokens=0','grow':'input_tokens=150','missing':'cache_read_tokens=NULL','negative':'input_tokens=-1','fix':'input_tokens=150','zero':'input_tokens=0,output_tokens=0,cache_read_tokens=0,cache_write_tokens=0,reasoning_tokens=0'}
    with sqlite3.connect(path) as db: db.execute('UPDATE session_model_usage SET '+updates[mode])
'@
    $script | & $Python - $hermes $mode
    if($LASTEXITCODE -ne 0){throw 'Synthetic Hermes fixture failed'}
}
Write-Fixture 'base'
$info=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings');$info.UseShellExecute=$false
$info.Environment['APPDATA']=Join-Path $profile 'appdata'
$info.Environment['LOCALAPPDATA']=Join-Path $profile 'local'
$info.Environment['USERPROFILE']=$fixtureHome
$info.Environment['HERMES_HOME']=$hermes
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

$warning='这些来源的本机记录不完整：Hermes。总量只覆盖可读取的计数。'
try {
    $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
    $until=[DateTime]::UtcNow.AddSeconds(20)
    do {
        $window=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {$_.Current.Name -like 'QuotaScope *' -and -not $_.Current.IsOffscreen}|Select-Object -First 1
        if($window){break};Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $until)
    if(-not $window){throw 'Settings window missing'}
    $page=Nodes|Where-Object {$_.Current.Name -eq 'Token 消耗' -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
    $page.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Agent 'Hermes';Wait-Total 185;Wait-Text $warning
    Wait-Text '缺少类型明细的 Token： 100';Wait-Text '未能计价的 Token： 100'
    Choose-Filter 2 'fixture-a';Wait-Total 120;Wait-Text '缺少类型明细的 Token： 100';Wait-Text '未能计价的 Token： 100'
    Choose-Filter 2 '全部模型';Wait-Total 185
    Write-Fixture 'repair';Refresh;Wait-Total 185;Wait-Text $warning -Absent;Wait-Text '缺少类型明细的 Token： 0'
    Write-Fixture 'grow';Refresh;Wait-Total 235
    Write-Fixture 'missing';Refresh;Wait-Total 235;Wait-Text $warning;Wait-Text '缺少类型明细的 Token： 150'
    Write-Fixture 'negative';Refresh;Wait-Total 85;Wait-Text $warning
    Write-Fixture 'fix';Write-Fixture 'repair';Refresh;Wait-Total 235;Wait-Text $warning -Absent
    Write-Fixture 'profile-delete';Refresh;Wait-Total 195
    Write-Fixture 'profile-add';Refresh;Wait-Total 235
    Write-Fixture 'zero';Refresh;Wait-Total 65
    Write-Fixture 'delete';Refresh;Wait-Total 40
    Write-Fixture 'recreate';Refresh;Wait-Total 185;Wait-Text $warning
    Write-Fixture 'replace';Refresh;Wait-Total 285;Wait-Text $warning -Absent
    $folder=Join-Path $data 'UsageImports/hermes'
    New-Item -ItemType Directory -Force -Path $folder|Out-Null
    $row=@{schema='quotascope.usage.v1';source='hermes';id='import-fixture';timestamp=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();model='fixture-a';usage=@{inputTokens=3;outputTokens=4}}
    [IO.File]::WriteAllText((Join-Path $folder 'usage.json'),($row|ConvertTo-Json -Depth 4),[Text.UTF8Encoding]::new($false))
    Refresh;Wait-Total 292
    Choose-Filter 2 'fixture-a';Wait-Total 227;Choose-Filter 2 '全部模型';Wait-Total 292
    $close=Nodes|Where-Object {$_.Current.Name -eq '关闭' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1
    $close.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 200
    if($testApp.HasExited){throw 'App exited on Settings close'}
    $reopen=[Diagnostics.Process]::Start($info)
    if(-not $reopen.WaitForExit(15000) -or $reopen.ExitCode -ne 0){throw 'Second settings request failed'}
    $page=Nodes|Where-Object {$_.Current.Name -eq 'Token 消耗' -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
    $page.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Agent 'Hermes';Wait-Total 292;Wait-Text $warning -Absent
    [pscustomobject]@{Passed=$true;Sha256=(Get-FileHash -LiteralPath $Executable).Hash;NativeSqlite=$true;ModelCoverageNoDoubleCount=$true;NamedProfiles=$true;ModelFiltersRetainUnknownKind=$true;CacheAndReasoningNotAdded=$true;PartialRepair=$true;Growth=$true;ProfileAddAndDelete=$true;ZeroReplacesOldSession=$true;SameSizeSameMtimeReplacement=$true;StandardImport=$true;SettingsCloseAndReopen=$true;OfflineSyntheticProfile=$true;RealAccountVerified=$false}|ConvertTo-Json|Tee-Object -FilePath (Join-Path $OutputDirectory 'hermes-ui-result.json')
} catch {
    $originalError=$_
    try {if($window -and -not $testApp.HasExited){Nodes|ForEach-Object {[pscustomobject]@{Name=$_.Current.Name;Class=$_.Current.ClassName}}|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $OutputDirectory 'hermes-ui-failure.json')}}catch{}
    throw $originalError
} finally {
    if(-not $testApp.HasExited){$testApp.Kill();$testApp.WaitForExit()}
    $reserve.Stop()
    $resolved=[IO.Path]::GetFullPath($profile)
    if((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') -or (Split-Path $resolved -Leaf) -ne ('qs-hermes-ui-'+$id)){throw 'Unexpected Hermes test profile'}
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
