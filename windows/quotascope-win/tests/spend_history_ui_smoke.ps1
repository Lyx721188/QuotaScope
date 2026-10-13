# Isolated two-month history verification on the Token page: activity chart
# modes, day inspect, model filter, monthly recap, month switch, subscription
# comparison, unpriced and partial notes. Synthetic counters only; a loopback
# proxy refuses every outbound connection.
# Run from windows/ with a debug build on an unlocked desktop.
param([Parameter(Mandatory=$true)][string]$Executable,[string]$OutputDirectory='target/history-ui-validation')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$id=[guid]::NewGuid().ToString('N')
$profile=Join-Path $env:TEMP ('qs-history-'+$id)
$data=Join-Path $profile 'appdata/QuotaScope'
$fixtureHome=Join-Path $profile 'home'
$session=Join-Path $fixtureHome '.codex/sessions/fixture.jsonl'
New-Item -ItemType Directory -Path $data,(Split-Path $session),$OutputDirectory -Force|Out-Null
$proxy=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$proxy.Start()
$settings=@{hasRun=$true;enabledAccounts=@();language='en';readsTokenSpend=$true;checksForUpdates=$false;proxyMode='manual';proxyUrl="http://127.0.0.1:$($proxy.LocalEndpoint.Port)"}
$settingsPath=Join-Path $data 'settings.json'
[IO.File]::WriteAllText($settingsPath,($settings|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$prices=@{fetched_at_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();prices=@{'fixture-priced'=@{input=10.0;output=20.0}}}
[IO.File]::WriteAllText((Join-Path $data 'model-prices-4.json'),($prices|ConvertTo-Json -Depth 4),[Text.UTF8Encoding]::new($false))
# Two calendar months. The older month holds a priced 1500-token day; the
# current month holds 1500 priced plus 200 unpriced tokens; a final backwards
# reading makes the Codex source partial without adding tokens.
$today=(Get-Date).Date
$olderDay=$today.AddMonths(-1)
$olderStamp=[DateTimeOffset]::new($olderDay.Year,$olderDay.Month,10,9,0,0,[DateTimeOffset]::Now.Offset).ToString('o')
$recentStamp=[DateTimeOffset]::new($today.Year,$today.Month,$today.Day,9,0,0,[DateTimeOffset]::Now.Offset).ToString('o')
$olderDate=[DateTime]::new($olderDay.Year,$olderDay.Month,10)
$yesterday=$today.AddDays(-1)
$lines=@(
  ('{"timestamp":"'+$olderStamp+'","payload":{"model":"fixture-priced","cwd":"C:/fixture"}}')
  ('{"timestamp":"'+$olderStamp+'","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"cache_write_input_tokens":0,"output_tokens":500}}}}')
  ('{"timestamp":"'+$recentStamp+'","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2000,"cached_input_tokens":400,"cache_write_input_tokens":0,"output_tokens":1000}}}}')
  ('{"timestamp":"'+$recentStamp+'","payload":{"model":"fixture-unpriced"}}')
  ('{"timestamp":"'+$recentStamp+'","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2100,"cached_input_tokens":400,"cache_write_input_tokens":0,"output_tokens":1100}}}}')
  ('{"timestamp":"'+$recentStamp+'","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":50}}}}')
)
[IO.File]::WriteAllText($session,(($lines -join "`n")+"`n"),[Text.UTF8Encoding]::new($false))
$info=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings');$info.UseShellExecute=$false
$info.Environment['APPDATA']=Join-Path $profile 'appdata'
$info.Environment['LOCALAPPDATA']=Join-Path $profile 'local'
$info.Environment['USERPROFILE']=$fixtureHome
$info.Environment['HERMES_HOME']=Join-Path $profile 'hermes'
$info.Environment['QUOTASCOPE_TEST_INSTANCE']=$id
foreach($key in @('GEMINI_CLI_HOME','JCODE_HOME','XDG_DATA_HOME','REASONIX_STATE_HOME','REASONIX_HOME','LM_STUDIO_HOME','SENPI_CODING_AGENT_DIR','SENPI_CODING_AGENT_SESSION_DIR','KIMCHI_CODING_AGENT_DIR','GJC_CODING_AGENT_DIR','GJC_CONFIG_DIR','PI_CONFIG_DIR','CODEBUFF_DATA_DIR')){[void]$info.Environment.Remove($key)}
$testApp=[Diagnostics.Process]::Start($info)
$condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
$root=[Windows.Automation.AutomationElement]::RootElement
$script:connections=0
function Reject-Connections{while($proxy.Pending()){$client=$proxy.AcceptTcpClient();$client.Dispose();$script:connections++}}
function Nodes{
    Reject-Connections
    $until=[DateTime]::UtcNow.AddSeconds(30)
    do{
        if($testApp.HasExited){throw 'History app exited'}
        try{
            $script:window=$root.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {$_.Current.Name -like 'QuotaScope *' -and -not $_.Current.IsOffscreen}|Select-Object -First 1
            if($window){return $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)}
        }catch [Windows.Automation.ElementNotAvailableException]{}catch [System.Runtime.InteropServices.COMException]{}
        Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $until)
    throw 'History window unavailable'
}
function Wait-Name([string]$name){
    $until=[DateTime]::UtcNow.AddSeconds(30)
    do{if(@(Nodes|Where-Object {$_.Current.Name -eq $name}).Count -gt 0){return};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
    throw "History name missing: $name"
}
function Wait-Match([string]$pattern){
    $until=[DateTime]::UtcNow.AddSeconds(30)
    do{if(@(Nodes|Where-Object {$_.Current.Name -match $pattern}).Count -gt 0){return};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
    throw "History pattern missing: $pattern"
}
function Wait-Absent-Match([string]$pattern){
    $until=[DateTime]::UtcNow.AddSeconds(30)
    do{if(@(Nodes|Where-Object {$_.Current.Name -match $pattern}).Count -eq 0){return};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
    throw "History pattern still present: $pattern"
}
function Text-Of($node){
    $pattern=$node.GetCurrentPattern([Windows.Automation.TextPattern]::Pattern)
    $pattern.DocumentRange.GetText(-1)
}
function Wait-Text([string]$name,[string]$expected){
    $until=[DateTime]::UtcNow.AddSeconds(30)
    do{
        $node=Nodes|Where-Object {$_.Current.Name -eq $name}|Select-Object -First 1
        if($node){try{if((Text-Of $node) -like "*$expected*"){return}}catch [Windows.Automation.ElementNotAvailableException]{}}
        Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $until)
    throw "History text missing: $name -> $expected"
}
function Page([string]$name){
    $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
    if(-not $node){throw "Missing history navigation: $name"}
    $node.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
function Combo([string]$name){
    $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::ComboBox}|Select-Object -First 1
    if(-not $node){throw "Missing history combo: $name"}
    $node
}
function Unnamed-Combo([int]$index){
    $combos=@(Nodes|Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::ComboBox -and $_.Current.Name -eq ''})
    if($combos.Count -le $index){throw "History combo #$index unavailable"}
    $combos[$index]
}
function Select-ComboItem($combo,[string]$item){
    $combo.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do{
        $target=$root.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)|Where-Object {$_.Current.Name -eq $item -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::ListItem}|Select-Object -First 1
        if(-not $target){Start-Sleep -Milliseconds 100}
    }while(-not $target -and [DateTime]::UtcNow -lt $until)
    if(-not $target){throw "History combo item missing: $item"}
    $target.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
function Press([string]$name){
    $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1
    if(-not $node){throw "Missing history button: $name"}
    $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}
function Press-Refresh{
    $until=[DateTime]::UtcNow.AddSeconds(20)
    do{
        $node=Nodes|Where-Object {$_.Current.Name -eq 'Refresh' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1
        if($node -and $node.Current.IsEnabled){$node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke();return}
        Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $until)
    throw 'History refresh unavailable'
}
function Wait-Idle{
    $until=[DateTime]::UtcNow.AddSeconds(60)
    do{
        $nodes=@(Nodes)
        $scanning=@($nodes|Where-Object {$_.Current.Name -match '^Scanning .*files read \d+$'}).Count -gt 0
        $refresh=$nodes|Where-Object {$_.Current.Name -eq 'Refresh' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1
        if(-not $scanning -and $refresh -and $refresh.Current.IsEnabled){return}
        Start-Sleep -Milliseconds 150
    }while([DateTime]::UtcNow -lt $until)
    throw 'History scan did not settle'
}
function Settle{$until=[DateTime]::UtcNow.AddSeconds(1);do{Reject-Connections;Start-Sleep -Milliseconds 50}while([DateTime]::UtcNow -lt $until)}
try{
    Page 'Token spend'
    # Activity is independent of the span picker: the whole year is present
    # while the card still shows the default one-week window.
    Wait-Text 'activity-total' '3200 tokens'
    Wait-Text 'activity-total' '2 active days'
    Wait-Name 'Unpriced tokens: 200'
    Wait-Name 'Incomplete records: Codex'
    Wait-Name 'Incomplete local records: Codex. Totals cover readable counters only.'
    # All time brings the older month into the card.
    Select-ComboItem (Unnamed-Combo 0) 'All time'
    Wait-Name '3200'
    # Three chart modes on the same data.
    Select-ComboItem (Combo 'Activity chart') 'Weekly bars'
    Wait-Match '\d{4}-\d{2}-\d{2} — \d{4}-\d{2}-\d{2} · 1500$'
    Wait-Match '\d{4}-\d{2}-\d{2} — \d{4}-\d{2}-\d{2} · 1700$'
    Select-ComboItem (Combo 'Activity chart') 'Running total'
    Wait-Absent-Match '· 1700$'
    Select-ComboItem (Combo 'Activity chart') 'Calendar grid'
    Wait-Name ("$($olderDate.ToString('yyyy-MM-dd')) · 1500")
    Wait-Name ("$($today.ToString('yyyy-MM-dd')) · 1700")
    # The model filter isolates the unpriced tool. Model is the third unnamed
    # combo (time range, agent, model).
    Select-ComboItem (Unnamed-Combo 2) 'fixture-unpriced'
    Wait-Text 'activity-total' '200 tokens'
    Wait-Text 'activity-total' '1 active days'
    Select-ComboItem (Unnamed-Combo 2) 'All models'
    Wait-Text 'activity-total' '3200 tokens'
    # Day inspect follows the chosen date.
    Wait-Name ("$($today.ToString('yyyy-MM-dd')) · 1700 tokens")
    Select-ComboItem (Combo 'Activity day') $yesterday.ToString('yyyy-MM-dd')
    Wait-Name ("$($yesterday.ToString('yyyy-MM-dd')) · 0 tokens")
    # Monthly recap: current month first, then the completed one.
    Press 'Show monthly recap'
    Wait-Text 'recap-total' '1700 tokens'
    Wait-Name ("recap-day-$($today.ToString('yyyy-MM-dd'))")
    Wait-Text ("recap-day-$($today.ToString('yyyy-MM-dd'))") '1700'
    Wait-Match 'Month in progress'
    Wait-Match 'Unpriced tokens: 200'
    # A partial source is excluded from the four-hour rhythm, not averaged in.
    Wait-Match 'Excluded from timing:'
    Wait-Match 'No reliable hourly records in this month.'
    Select-ComboItem (Combo 'Recap month') $olderDay.ToString('yyyy-MM')
    Wait-Text 'recap-total' '1500 tokens'
    Wait-Name ("recap-day-$($olderDate.ToString('yyyy-MM-dd'))")
    Wait-Text ("recap-day-$($olderDate.ToString('yyyy-MM-dd'))") '1500'
    Wait-Match 'Completed calendar month'
    # A subscription price is compared, never assumed.
    $budget=Nodes|Where-Object {$_.Current.Name -eq 'Monthly subscription price (USD)' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit}|Select-Object -First 1
    if(-not $budget){throw 'Missing history subscription input'}
    $budget.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('10')
    Wait-Match 'Prorated price'
    if($connections -ne 0){throw 'History page opened an outbound connection'}
    # Deleting the transcript and clearing the rebuildable parse cache must not
    # erase the measured history.
    Remove-Item -LiteralPath $session -Force
    Remove-Item -LiteralPath (Join-Path $data 'ledger-5-codex.json') -Force -ErrorAction SilentlyContinue
    Press-Refresh
    Start-Sleep -Milliseconds 400
    Wait-Idle
    Wait-Text 'activity-total' '3200 tokens'
    Wait-Name '3200'
    # The same path reappearing is one replacement observation, not an addition.
    $reappear=@(
      ('{"timestamp":"'+$olderStamp+'","payload":{"model":"fixture-priced"}}')
      ('{"timestamp":"'+$olderStamp+'","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":500,"cached_input_tokens":100,"cache_write_input_tokens":0,"output_tokens":100}}}}')
    )
    [IO.File]::WriteAllText($session,(($reappear -join "`n")+"`n"),[Text.UTF8Encoding]::new($false))
    Press-Refresh
    Start-Sleep -Milliseconds 400
    Wait-Idle
    Wait-Text 'activity-total' '600 tokens'
    Wait-Name ("$($olderDate.ToString('yyyy-MM-dd')) · 600")
    Wait-Name ("$($today.ToString('yyyy-MM-dd')) · 0")
    Settle
    if($connections -ne 0){throw 'History rescan opened an outbound connection'}
    $firstHash=(Get-FileHash -LiteralPath (Join-Path $data 'ledger-5-codex.json') -Algorithm SHA256).Hash
    $window.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close();Settle
    if($testApp.HasExited){throw 'History app exited on Settings close'}
    $reopen=[Diagnostics.Process]::Start($info)
    if(-not $reopen.WaitForExit(15000) -or $reopen.ExitCode -ne 0){throw 'History reopen request failed'}
    Page 'Token spend'
    Wait-Text 'activity-total' '600 tokens'
    Wait-Name 'Total tokens'
    if((Get-FileHash -LiteralPath (Join-Path $data 'ledger-5-codex.json') -Algorithm SHA256).Hash -ne $firstHash){throw 'Reopen rescan rewrote an unchanged parse cache'}
    [pscustomobject]@{Passed=$true;Sha256=(Get-FileHash -LiteralPath $Executable).Hash;TwoCalendarMonths=$true;ActivityIndependentOfSpan=$true;AllTimeCardTotal=3200;ChartModesChecked=@('Calendar grid','Weekly bars','Running total');ModelFilterIsolated=$true;DayInspectQuietDay=$true;CurrentMonthRecap=1700;CompletedMonthRecap=1500;MonthSwitch=$true;SubscriptionCompared=$true;UnpricedTokens=200;PartialSourceShown=$true;DeletedTranscriptKept=3200;ParseCacheCleared=$true;ReappearedTranscriptReplaced=600;CloseAndReopen=$true;WarmCacheUnchanged=$true;ExternalRequestsBlocked=$true}|ConvertTo-Json|Tee-Object -FilePath (Join-Path $OutputDirectory 'history-ui-result.json')
}catch{
    $originalError=$_
    try{if($window -and -not $testApp.HasExited){Nodes|ForEach-Object {[pscustomobject]@{Name=$_.Current.Name;Class=$_.Current.ClassName;Type=$_.Current.ControlType.ProgrammaticName}}|ConvertTo-Json -Depth 3|Set-Content -LiteralPath (Join-Path $OutputDirectory 'history-ui-failure.json')}}catch{}
    throw $originalError
}finally{
    if(-not $testApp.HasExited){$testApp.Kill();$testApp.WaitForExit()};$proxy.Stop()
    $resolved=[IO.Path]::GetFullPath($profile)
    if((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') -or (Split-Path $resolved -Leaf) -ne ('qs-history-'+$id)){throw 'Unexpected history profile'}
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
