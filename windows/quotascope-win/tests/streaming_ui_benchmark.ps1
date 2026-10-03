# Run from windows/ with debug binaries. Uses isolated synthetic history.
param(
  [Parameter(Mandatory=$true)][string]$Executable,
  [Parameter(Mandatory=$true)][ValidateSet('before','after')][string]$Label,
  [ValidateRange(1,2000000)][int]$Records=400000
)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$testId=[guid]::NewGuid().ToString('N')
$fixtureProfile=Join-Path $env:TEMP ('quotascope-stream-bench-'+$testId)
$data=Join-Path $fixtureProfile 'appdata/QuotaScope'
$fixtureHome=Join-Path $fixtureProfile 'home'
$session=Join-Path $fixtureHome '.codex/sessions/fixture.jsonl'
New-Item -ItemType Directory -Path $data,(Split-Path $session) -Force | Out-Null
Set-Content -LiteralPath (Join-Path $data 'settings.json') -Value '{"hasRun":true,"enabledAccounts":[],"language":"zh","readsTokenSpend":true}' -Encoding utf8
$prices=@{fetched_at_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();prices=@{'fixture-model'=@{input=1.0;output=2.0};'fixture-beta'=@{input=1.0;output=2.0}}}
[IO.File]::WriteAllText((Join-Path $data 'model-prices-4.json'),($prices|ConvertTo-Json -Depth 4))
$timestamp=[DateTime]::UtcNow.ToString('o')
$header='{"type":"turn_context","payload":{"model":"fixture-model"}}'
function Counter([int]$n) {
  @{timestamp=$timestamp;payload=@{type='token_count';info=@{total_token_usage=@{input_tokens=100*$n;cached_input_tokens=20*$n;output_tokens=50*$n}}}} | ConvertTo-Json -Depth 6 -Compress
}
$counter=Counter 1
$writer=[IO.StreamWriter]::new($session,$false,[Text.UTF8Encoding]::new($false))
try {
  $writer.WriteLine($header)
  $chunk=($counter+"`n")*1000
  $whole=[Math]::Floor($Records/1000)
  for($i=0;$i -lt $whole;$i++){ $writer.Write($chunk) }
  for($i=0;$i -lt ($Records%1000);$i++){ $writer.WriteLine($counter) }
} finally { $writer.Dispose() }
$fixtureBytes=(Get-Item -LiteralPath $session).Length
$info=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings')
$info.UseShellExecute=$false
$info.Environment['APPDATA']=Join-Path $fixtureProfile 'appdata'
$info.Environment['LOCALAPPDATA']=Join-Path $fixtureProfile 'local'
$info.Environment['USERPROFILE']=$fixtureHome
$info.Environment['QUOTASCOPE_TEST_INSTANCE']=$testId
foreach($key in @('GEMINI_CLI_HOME','JCODE_HOME','XDG_DATA_HOME','REASONIX_STATE_HOME','REASONIX_HOME','LM_STUDIO_HOME','SENPI_CODING_AGENT_DIR','SENPI_CODING_AGENT_SESSION_DIR','KIMCHI_CODING_AGENT_DIR','GJC_CODING_AGENT_DIR','GJC_CONFIG_DIR','PI_CONFIG_DIR','CODEBUFF_DATA_DIR')) { [void]$info.Environment.Remove($key) }
$testApp=[Diagnostics.Process]::Start($info)
$samples=[Collections.Generic.List[object]]::new()
function Nodes {
  if($testApp.HasExited){ throw 'Application exited unexpectedly' }
  $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
}
function Sample([string]$phase) {
  $testApp.Refresh()
  $sample=[pscustomobject]@{Phase=$phase;ProcessId=$testApp.Id;Time=[DateTime]::UtcNow.ToString('o');PrivateBytes=$testApp.PrivateMemorySize64;WorkingSetBytes=$testApp.WorkingSet64;Handles=$testApp.HandleCount}
  $samples.Add($sample)
  $sample
}
function Wait-Complete([string]$phase,[int]$expected,[Diagnostics.Stopwatch]$watch,[long]$baseline) {
  $until=[DateTime]::UtcNow.AddSeconds(60)
  $nextUi=0
  $complete=$false
  while([DateTime]::UtcNow -lt $until){
    [void](Sample $phase)
    if($watch.ElapsedMilliseconds -ge $nextUi){
      $nodes=@(Nodes)
      $names=@($nodes|ForEach-Object{$_.Current.Name})
      $refresh=$nodes|Where-Object{$_.Current.Name -eq '刷新' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1
      if('Token 总量' -in $names -and $expected.ToString() -in $names -and $refresh.Current.IsEnabled){ $complete=$true;break }
      $nextUi=$watch.ElapsedMilliseconds+200
    }
    Start-Sleep -Milliseconds 20
  }
  $watch.Stop()
  if(-not $complete){ throw "Expected $expected tokens after $phase" }
  $cache=Get-Content -LiteralPath (Join-Path $data 'ledger-4-codex.json') -Raw | ConvertFrom-Json
  [long]$cachedTokens=0
  foreach($entry in $cache.files.PSObject.Properties){
    foreach($day in $entry.Value.days.PSObject.Properties){
      foreach($model in $day.Value.PSObject.Properties){
        $tally=$model.Value
        $cachedTokens+=$tally.input+$tally.cache_write+$tally.cache_read+$tally.output
      }
    }
  }
  if($cachedTokens -ne $expected){throw "Cached counters disagree after $phase : $cachedTokens"}
  $phaseSamples=@($samples|Where-Object Phase -eq $phase)
  $peak=($phaseSamples|Measure-Object PrivateBytes -Maximum).Maximum
  [pscustomobject]@{ExpectedTokens=$expected;ElapsedMilliseconds=$watch.ElapsedMilliseconds;BaselinePrivateBytes=$baseline;PeakPrivateBytes=[long]$peak;PeakIncreaseBytes=[long]$peak-$baseline;Samples=$phaseSamples.Count}
}
function Refresh-Button {
  Nodes|Where-Object{$_.Current.Name -eq '刷新' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1
}
function Wait-Window {
  $root=[Windows.Automation.AutomationElement]::RootElement
  $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
  $until=[DateTime]::UtcNow.AddSeconds(15)
  do {
    Start-Sleep -Milliseconds 200
    if($testApp.HasExited){throw 'Settings unavailable'}
    $script:window=$root.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object{$_.Current.Name -like 'QuotaScope *'}|Select-Object -First 1
  } until($window -or [DateTime]::UtcNow -gt $until)
  if(-not $window){throw 'Settings unavailable'}
}
try {
  Wait-Window
  Start-Sleep -Seconds 2
  $initialBaseline=(Sample 'baseline').PrivateBytes
  $spend=Nodes|Where-Object{$_.Current.Name -eq 'Token 消耗' -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
  $watch=[Diagnostics.Stopwatch]::StartNew()
  $spend.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
  $full=Wait-Complete 'full' 150 $watch $initialBaseline
  $tail=(Counter 2)+"`n"+(Counter 2)+"`n"+'{"payload":{"model":"fixture-beta"}}'+"`n"+(Counter 3)+"`n"
  [IO.File]::AppendAllText($session,$tail,[Text.UTF8Encoding]::new($false))
  $appendBaseline=(Sample 'append-baseline').PrivateBytes
  $watch=[Diagnostics.Stopwatch]::StartNew()
  (Refresh-Button).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
  $append=Wait-Complete 'append' 450 $watch $appendBaseline
  # Recreate the process and append again: parser state must survive on disk.
  $firstProcessId=$testApp.Id
  $testApp.Kill()
  $testApp.WaitForExit()
  $testApp.Dispose()
  [IO.File]::AppendAllText($session,(Counter 4)+"`n",[Text.UTF8Encoding]::new($false))
  $testApp=[Diagnostics.Process]::Start($info)
  Wait-Window
  Start-Sleep -Seconds 2
  $restartBaseline=(Sample 'restart-baseline').PrivateBytes
  $spend=Nodes|Where-Object{$_.Current.Name -eq 'Token 消耗' -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
  $watch=[Diagnostics.Stopwatch]::StartNew()
  $spend.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
  $restart=Wait-Complete 'restart' 600 $watch $restartBaseline
  # A rewrite that changes the old prefix must discard its previous totals.
  [IO.File]::WriteAllText($session,$header+"`n"+(Counter 5)+"`n",[Text.UTF8Encoding]::new($false))
  $rewriteBaseline=(Sample 'rewrite-baseline').PrivateBytes
  $watch=[Diagnostics.Stopwatch]::StartNew()
  (Refresh-Button).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
  $rewrite=Wait-Complete 'rewrite' 750 $watch $rewriteBaseline
  $result=[pscustomobject]@{Passed=$true;Label=$Label;ExecutableSHA256=(Get-FileHash -LiteralPath $info.FileName -Algorithm SHA256).Hash;FixtureBytes=$fixtureBytes;Records=$Records;Full=$full;Append=$append;Restart=$restart;RestartFromDisk=$true;ProcessIds=@($firstProcessId,$testApp.Id);Rewrite=$rewrite}
  $samples|ConvertTo-Json|Set-Content -LiteralPath ('target/stream-'+$Label+'-samples.json') -Encoding utf8
  $result|ConvertTo-Json -Depth 5|Tee-Object -FilePath ('target/stream-'+$Label+'-result.json')
} finally {
  if(-not $testApp.HasExited){ $testApp.Kill();$testApp.WaitForExit() }
  $testApp.Dispose()
  $resolved=[IO.Path]::GetFullPath($fixtureProfile)
  if((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')){throw 'Unexpected profile location'}
  Remove-Item -LiteralPath $resolved -Recurse -Force
}
