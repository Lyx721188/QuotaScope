# Run from windows/ with a debug build; all accounts, logs and caches are synthetic.
param([Parameter(Mandatory=$true)][string]$Executable)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$testId=[guid]::NewGuid().ToString('N')
$fixtureProfile=Join-Path $env:TEMP ('quotascope-maintenance-'+$testId)
$data=Join-Path $fixtureProfile 'appdata/QuotaScope'
$fixtureHome=Join-Path $fixtureProfile 'home'
$session=Join-Path $fixtureHome '.codex/sessions/fixture.jsonl'
$settingsPath=Join-Path $data 'settings.json'
$reportPath=Join-Path $data 'diagnostics/QuotaScope-diagnostics.json'
New-Item -ItemType Directory -Path $data,(Split-Path $session) -Force|Out-Null
$prefs=@{hasRun=$true;enabledAccounts=@();language='zh';readsTokenSpend=$true;serverAddresses=@{privateLabel='https://private-endpoint?key=private-secret'};display='private-device-path';skippedUpdateVersion='private-version'}
[IO.File]::WriteAllText($settingsPath,($prefs|ConvertTo-Json -Depth 4))
$prices=@{fetched_at_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds();prices=@{'fixture-model'=@{input=1.0;output=2.0}}}
[IO.File]::WriteAllText((Join-Path $data 'model-prices-4.json'),($prices|ConvertTo-Json -Depth 4))
foreach($name in @('keys.dat','last-readings.json','ledger-4-unknown.json')){[IO.File]::WriteAllText((Join-Path $data $name),'private-storage-sentinel')}
foreach($name in @('ledger-4-claudeCode.json','ledger-4-codex.json')){
  $file=[IO.File]::OpenWrite((Join-Path $data $name))
  try{$file.SetLength(12*1024*1024);$bytes=[Text.Encoding]::UTF8.GetBytes('private-transcript-sentinel');$file.Write($bytes,0,$bytes.Length)}finally{$file.Dispose()}
}
[IO.File]::WriteAllText((Join-Path $data 'ledger-4-claudeCode.json.tmp'),'interrupted-cache-write')
(Get-Item -LiteralPath (Join-Path $data 'ledger-4-claudeCode.json')).LastWriteTimeUtc=[DateTime]::UtcNow.AddDays(-2)
(Get-Item -LiteralPath (Join-Path $data 'ledger-4-codex.json')).LastWriteTimeUtc=[DateTime]::UtcNow.AddDays(-1)
$timestamp=[DateTime]::UtcNow.ToString('o')
function Write-Session([int]$n){
  $header='{"type":"turn_context","payload":{"model":"fixture-model"}}'
  $count=@{timestamp=$timestamp;payload=@{type='token_count';info=@{total_token_usage=@{input_tokens=100*$n;cached_input_tokens=20*$n;output_tokens=50*$n}}}}|ConvertTo-Json -Depth 6 -Compress
  [IO.File]::WriteAllText($session,$header+"`n"+$count+"`n",[Text.UTF8Encoding]::new($false))
}
Write-Session 1
$priceHash=(Get-FileHash -LiteralPath (Join-Path $data 'model-prices-4.json')).Hash
$sourceHash=(Get-FileHash -LiteralPath $session).Hash
$info=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings');$info.UseShellExecute=$false
$info.Environment['APPDATA']=Join-Path $fixtureProfile 'appdata'
$info.Environment['LOCALAPPDATA']=Join-Path $fixtureProfile 'local'
$info.Environment['USERPROFILE']=$fixtureHome
$info.Environment['QUOTASCOPE_TEST_INSTANCE']=$testId
foreach($key in @('GEMINI_CLI_HOME','JCODE_HOME','XDG_DATA_HOME','REASONIX_STATE_HOME','REASONIX_HOME','LM_STUDIO_HOME','SENPI_CODING_AGENT_DIR','SENPI_CODING_AGENT_SESSION_DIR','KIMCHI_CODING_AGENT_DIR','GJC_CODING_AGENT_DIR','GJC_CONFIG_DIR','PI_CONFIG_DIR','CODEBUFF_DATA_DIR')){[void]$info.Environment.Remove($key)}
$testApp=$null
function Nodes {
  for($attempt=0;$attempt -lt 4;$attempt++){
    if($testApp.HasExited){throw 'Application exited unexpectedly'}
    try{return $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)}catch{
      if($attempt -eq 3){throw}
      Start-Sleep -Milliseconds 150
    }
  }
}
function Wait-For([scriptblock]$predicate,[string]$description){
  $until=[DateTime]::UtcNow.AddSeconds(20)
  do{if(& $predicate){return};Start-Sleep -Milliseconds 200}until([DateTime]::UtcNow -gt $until)
  throw "Timed out: $description"
}
function Start-App {
  $script:testApp=[Diagnostics.Process]::Start($info)
  $root=[Windows.Automation.AutomationElement]::RootElement
  $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
  Wait-For {
    if($testApp.HasExited){throw 'Settings unavailable'}
    $script:window=$root.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object{$_.Current.Name -like 'QuotaScope *'}|Select-Object -First 1
    [bool]$window
  } 'settings window'
}
function Select-Page([string]$name){
  $node=Nodes|Where-Object{$_.Current.Name -eq $name -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
  if(-not $node){throw "Missing navigation: $name"}
  $node.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
  Start-Sleep -Milliseconds 350
}
function Button([string]$name){Nodes|Where-Object{$_.Current.Name -eq $name -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1}
function Click([string]$name){
  $button=Button $name
  if(-not $button -or -not $button.Current.IsEnabled){throw "Unavailable button: $name"}
  $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}
function Wait-Maintenance {
  Start-Sleep -Milliseconds 350
  Wait-For { $button=Button '刷新缓存信息';$button -and $button.Current.IsEnabled -and @(Nodes|Where-Object{$_.Current.Name -like '统计缓存：*'}).Count -gt 0 } 'maintenance completion'
}
function Select-Budget([string]$name,[int]$expected){
  $combo=Nodes|Where-Object{$_.Current.ControlType -eq [Windows.Automation.ControlType]::ComboBox}|Select-Object -First 1
  $combo.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
  Start-Sleep -Milliseconds 200
  $option=Nodes|Where-Object{$_.Current.Name -eq $name -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::ListItem}|Select-Object -First 1
  if(-not $option){throw "Missing budget option: $name"}
  $option.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
  Wait-For { (Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json).statisticsCacheLimitMb -eq $expected } 'budget persistence'
  Wait-Maintenance
}
function Export-Report {
  Click '导出诊断信息'
  Start-Sleep -Milliseconds 350
  Wait-For { @(Nodes|Where-Object{$_.Current.Name -eq '诊断信息已导出到本地。'}).Count -gt 0 } 'diagnostics export'
  $text=Get-Content -LiteralPath $reportPath -Raw
  foreach($marker in @('privateLabel','private-endpoint','private-secret','private-device-path','private-version','private-storage-sentinel','private-transcript-sentinel',$fixtureHome,$fixtureProfile)){
    if($text.Contains($marker)){throw "Sensitive field leaked: $marker"}
  }
  $report=$text|ConvertFrom-Json
  if($report.schema_version -ne 1 -or $report.runtime.process.private_bytes -le 0 -or $report.runtime.process.handles -le 0){throw 'Diagnostic resource fields missing'}
  if((Get-Item -LiteralPath $reportPath).Length -gt 8192){throw 'Unexpectedly large report'}
  $report
}
function Wait-Tokens([int]$expected){
  Wait-For { $names=@(Nodes|ForEach-Object{$_.Current.Name});$refresh=Button '刷新';'Token 总量' -in $names -and $expected.ToString() -in $names -and $refresh.Current.IsEnabled } "tokens $expected"
}
try {
  Start-App
  Select-Page '存储与诊断';Wait-Maintenance
  $first=Export-Report
  if($first.preferences.statistics_cache_limit_mb -ne 64 -or $first.statistics_cache.bytes -lt 24*1024*1024){throw 'Initial inventory/default incorrect'}
  Select-Budget '16 MiB' 16
  if(Test-Path -LiteralPath (Join-Path $data 'ledger-4-claudeCode.json')){throw 'Oldest cache not removed'}
  if(-not (Test-Path -LiteralPath (Join-Path $data 'ledger-4-codex.json'))){throw 'Recent cache removed unnecessarily'}
  $second=Export-Report
  if($second.statistics_cache.bytes -gt 16*1024*1024){throw 'Budget exceeded'}
  if(@(Get-ChildItem -LiteralPath (Split-Path $reportPath) -File).Count -ne 1){throw 'Repeated export grew the report folder'}
  $firstProcess=$testApp.Id;$testApp.Kill();$testApp.WaitForExit();$testApp.Dispose()
  Start-App;Select-Page '存储与诊断';Wait-Maintenance
  $restart=Export-Report
  if($restart.preferences.statistics_cache_limit_mb -ne 16){throw 'Budget lost on restart'}
  Click '清理统计缓存';Wait-Maintenance
  foreach($name in @('ledger-4-codex.json','ledger-5-codex.json','ledger-4-claudeCode.json.tmp')){if(Test-Path -LiteralPath (Join-Path $data $name)){throw "Cache retained: $name"}}
  foreach($name in @('keys.dat','last-readings.json','ledger-4-unknown.json')){if((Get-Content -LiteralPath (Join-Path $data $name) -Raw) -ne 'private-storage-sentinel'){throw "Unrelated file changed: $name"}}
  if((Get-FileHash -LiteralPath $session).Hash -ne $sourceHash -or (Get-FileHash -LiteralPath (Join-Path $data 'model-prices-4.json')).Hash -ne $priceHash){throw 'Source or prices changed'}
  Select-Budget '禁用磁盘缓存' 0
  Select-Page 'Token 消耗';Wait-Tokens 150
  if(Test-Path -LiteralPath (Join-Path $data 'ledger-5-codex.json')){throw 'Disabled disk cache was written'}
  Select-Page '存储与诊断';Wait-Maintenance
  Select-Budget '64 MiB（默认）' 64
  Click '清理统计缓存';Wait-Maintenance
  Write-Session 2
  Select-Page 'Token 消耗';Wait-Tokens 300
  if(-not (Test-Path -LiteralPath (Join-Path $data 'ledger-5-codex.json'))){throw 'Cache not rebuilt after enabling'}
  Select-Page '存储与诊断';Wait-Maintenance
  $final=Export-Report
  $reportHash=(Get-FileHash -LiteralPath $reportPath).Hash
  $blockedTemporary=$reportPath+'.tmp'
  New-Item -ItemType Directory -Path $blockedTemporary|Out-Null
  Click '导出诊断信息'
  Wait-For { @(Nodes|Where-Object{$_.Current.Name -like '操作未能完成。*'}).Count -gt 0 } 'export failure message'
  if((Get-FileHash -LiteralPath $reportPath).Hash -ne $reportHash){throw 'Failed export replaced the valid report'}
  Remove-Item -LiteralPath $blockedTemporary
  $final=Export-Report
  $final|ConvertTo-Json -Depth 8|Set-Content -LiteralPath 'target/maintenance-diagnostics.json' -Encoding utf8
  Nodes|ForEach-Object{[pscustomobject]@{Name=$_.Current.Name;Class=$_.Current.ClassName;Enabled=$_.Current.IsEnabled}}|ConvertTo-Json|Set-Content -LiteralPath 'target/maintenance-ui-nodes.json' -Encoding utf8
  [pscustomobject]@{Passed=$true;DefaultsTo64MiB=$true;BudgetPersistedOnRestart=$true;OldestEvicted=$true;SensitiveFieldsExcluded=$true;FixedExportFile=$true;FailedExportPreservedReport=$true;UnrelatedFilesPreserved=$true;NoDiskCacheWorks=$true;RebuiltTokens=300;ProcessIds=@($firstProcess,$testApp.Id);ExecutableSHA256=(Get-FileHash -LiteralPath $info.FileName).Hash}|ConvertTo-Json|Tee-Object -FilePath 'target/maintenance-ui-result.json'
} finally {
  if($testApp){if(-not $testApp.HasExited){$testApp.Kill();$testApp.WaitForExit()};$testApp.Dispose()}
  $resolved=[IO.Path]::GetFullPath($fixtureProfile)
  if((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')){throw 'Unexpected profile location'}
  Remove-Item -LiteralPath $resolved -Recurse -Force
}
