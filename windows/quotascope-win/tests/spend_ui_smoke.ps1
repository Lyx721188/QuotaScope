# Run from windows/ with a debug build on an unlocked desktop.
# The isolated home contains synthetic counters, never user history or keys.
param([Parameter(Mandatory = $true)][string]$Executable)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class SpendSmokeNative {
    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
}
"@
$testId = [guid]::NewGuid().ToString('N')
$profile = Join-Path $env:TEMP ('quotascope-spend-' + $testId)
$data = Join-Path $profile 'appdata/QuotaScope'
$fixtureHome = Join-Path $profile 'home'
$session = Join-Path $fixtureHome '.codex/sessions/fixture.jsonl'
New-Item -ItemType Directory -Path $data, (Split-Path $session) -Force | Out-Null
Set-Content -LiteralPath (Join-Path $data 'settings.json') -Value '{"hasRun":true,"enabledAccounts":[],"language":"zh","readsTokenSpend":true}' -Encoding utf8
$prices = @{fetched_at_ms=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds(); prices=@{'fixture-model'=@{input=1.0; output=2.0}}}
[IO.File]::WriteAllText((Join-Path $data 'model-prices-4.json'), ($prices | ConvertTo-Json -Depth 4))
$stamp = [DateTime]::UtcNow.ToString('o')
$header = '{"type":"turn_context","payload":{"model":"fixture-model"}}'
$counter = '{"timestamp":"' + $stamp + '","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":50}}}}'
function Write-LargeFixture {
  $writer = [IO.StreamWriter]::new($session, $false, [Text.UTF8Encoding]::new($false))
  try {
    $writer.WriteLine($header)
    $chunk = ($counter + "`n") * 1000
    for ($i = 0; $i -lt 400; $i++) { $writer.Write($chunk) }
  } finally { $writer.Dispose() }
}
Write-LargeFixture
$fixtureBytes = (Get-Item -LiteralPath $session).Length
$info = [Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings')
$info.UseShellExecute = $false
$info.Environment['APPDATA'] = Join-Path $profile 'appdata'
$info.Environment['LOCALAPPDATA'] = Join-Path $profile 'local'
$info.Environment['USERPROFILE'] = $fixtureHome
$info.Environment['QUOTASCOPE_TEST_INSTANCE'] = $testId
# Remove inherited source-directory overrides so this run stays isolated.
foreach ($key in @('GEMINI_CLI_HOME','JCODE_HOME','XDG_DATA_HOME','REASONIX_STATE_HOME','REASONIX_HOME','LM_STUDIO_HOME','SENPI_CODING_AGENT_DIR','SENPI_CODING_AGENT_SESSION_DIR','KIMCHI_CODING_AGENT_DIR','GJC_CODING_AGENT_DIR','GJC_CONFIG_DIR','PI_CONFIG_DIR','CODEBUFF_DATA_DIR')) { [void]$info.Environment.Remove($key) }
$testApp = [Diagnostics.Process]::Start($info)
function Get-Nodes {
  if ($testApp.HasExited) { throw 'Test application exited' }
  $window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
}
function Select-Page([string]$name) {
  $node = Get-Nodes | Where-Object { $_.Current.Name -eq $name -and $_.Current.ClassName -like '*NavigationViewItem' } | Select-Object -First 1
  if (-not $node) { throw "Navigation unavailable: $name" }
  $node.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
function Wait-Progress {
  $until = [DateTime]::UtcNow.AddSeconds(15)
  do {
    Start-Sleep -Milliseconds 100
    $nodes = @(Get-Nodes)
    $progress = $nodes | Where-Object { $_.Current.Name -like '正在扫描 Codex*' } | Select-Object -First 1
    $cancel = $nodes | Where-Object { $_.Current.Name -eq '取消扫描' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button } | Select-Object -First 1
  } until (($progress -and $cancel -and $cancel.Current.IsEnabled) -or [DateTime]::UtcNow -gt $until)
  if (-not $progress -or -not $cancel.Current.IsEnabled) { throw 'Scan progress or cancel button unavailable' }
  # OpenCode, Kilo CLI and Claude precede Codex in the current catalogue.
  if ($progress.Current.Name -notmatch '来源 3/54 · 已读取 \d+ 个文件') { throw ('Progress counters incorrect: ' + $progress.Current.Name) }
  [pscustomobject]@{Progress=$progress.Current.Name; Cancel=$cancel}
}
try {
  $root = [Windows.Automation.AutomationElement]::RootElement
  $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, $testApp.Id)
  $until = [DateTime]::UtcNow.AddSeconds(15)
  do {
    Start-Sleep -Milliseconds 200
    if ($testApp.HasExited) { throw 'Test application exited' }
    $window = $root.FindAll([Windows.Automation.TreeScope]::Children, $condition) | Where-Object { $_.Current.Name -like 'QuotaScope *' } | Select-Object -First 1
  } until ($window -or [DateTime]::UtcNow -gt $until)
  if (-not $window) { throw 'Settings window unavailable' }
  Select-Page 'Token 消耗'
  $active = Wait-Progress
  $watch = [Diagnostics.Stopwatch]::StartNew()
  $active.Cancel.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
  $until = [DateTime]::UtcNow.AddSeconds(6)
  do {
    Start-Sleep -Milliseconds 100
    $nodes = @(Get-Nodes)
    $cancelled = $nodes | Where-Object { $_.Current.Name -eq '已取消本地扫描。' } | Select-Object -First 1
    $refresh = $nodes | Where-Object { $_.Current.Name -eq '刷新' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button } | Select-Object -First 1
  } until (($cancelled -and $refresh.Current.IsEnabled) -or [DateTime]::UtcNow -gt $until)
  $watch.Stop()
  if (-not $cancelled -or -not $refresh.Current.IsEnabled) { throw 'Worker did not stop after cancellation' }
  if (@($nodes | Where-Object { $_.Current.Name -eq 'Token 总量' }).Count -ne 0) { throw 'Cancelled scan published partial statistics' }
  $codexCache = Join-Path $data 'ledger-4-codex.json'
  if (Test-Path -LiteralPath $codexCache) { throw 'Cancelled scan saved a partial Codex cache' }
  $cancelMilliseconds = $watch.ElapsedMilliseconds
  $refresh.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
  $navigationProgress = Wait-Progress
  Select-Page '关于'
  Start-Sleep -Milliseconds 1000
  if (Test-Path -LiteralPath $codexCache) { throw 'Leaving the page did not cancel the scan' }
  Select-Page 'Token 消耗'
  $closedProgress = Wait-Progress
  if (-not [SpendSmokeNative]::PostMessage([IntPtr]$window.Current.NativeWindowHandle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'Settings close request failed' }
  Start-Sleep -Milliseconds 1000
  if ($testApp.HasExited -or -not $window.Current.IsOffscreen) { throw 'Closing Settings did not hide the existing host' }
  if (Test-Path -LiteralPath $codexCache) { throw 'Closing Settings did not cancel the scan' }
  $second = [Diagnostics.Process]::Start($info)
  try {
    if (-not $second.WaitForExit(5000) -or $second.ExitCode -ne 0) { throw 'Second launch did not reopen the existing Settings host' }
  } finally { if (-not $second.HasExited) { $second.Kill(); $second.WaitForExit() }; $second.Dispose() }
  $until = [DateTime]::UtcNow.AddSeconds(10)
  while ($window.Current.IsOffscreen -and [DateTime]::UtcNow -lt $until) { Start-Sleep -Milliseconds 100 }
  if ($window.Current.IsOffscreen) { throw 'Settings did not become visible after reopening' }
  # A new complete scan must work after all cancellation paths.
  [IO.File]::WriteAllText($session, ($header + "`n" + $counter + "`n"), [Text.UTF8Encoding]::new($false))
  Select-Page 'Token 消耗'
  $until = [DateTime]::UtcNow.AddSeconds(15)
  do {
    Start-Sleep -Milliseconds 200
    $nodes = @(Get-Nodes)
    $names = @($nodes | ForEach-Object { $_.Current.Name })
  } until (('Token 总量' -in $names) -or [DateTime]::UtcNow -gt $until)
  if ('Token 总量' -notin $names -or '150' -notin $names) { throw 'Rescan did not publish the expected complete token total' }
  if (-not (Test-Path -LiteralPath $codexCache)) { throw 'Complete scan did not persist its cache' }
  $nodes | ForEach-Object { [pscustomobject]@{Name=$_.Current.Name; Class=$_.Current.ClassName; Enabled=$_.Current.IsEnabled} } | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath 'target/spend-ui-complete.json' -Encoding utf8
  # Manual refresh must re-read changed files even within the cache lifetime.
  # Cancelling that refresh keeps the preceding complete result and disk cache.
  $previousHash = (Get-FileHash -LiteralPath $codexCache -Algorithm SHA256).Hash
  Write-LargeFixture
  $refresh = $nodes | Where-Object { $_.Current.Name -eq '刷新' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button } | Select-Object -First 1
  $refresh.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
  $preservedProgress = Wait-Progress
  $preservedProgress.Cancel.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
  $until = [DateTime]::UtcNow.AddSeconds(6)
  do {
    Start-Sleep -Milliseconds 100
    $nodes = @(Get-Nodes)
    $names = @($nodes | ForEach-Object { $_.Current.Name })
    $refresh = $nodes | Where-Object { $_.Current.Name -eq '刷新' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button } | Select-Object -First 1
  } until (('已取消本地扫描。' -in $names -and $refresh.Current.IsEnabled) -or [DateTime]::UtcNow -gt $until)
  if ('150' -notin $names -or '当前仍显示上次完成的结果。' -notin $names -or -not $refresh.Current.IsEnabled) { throw 'Cancelling refresh discarded the complete snapshot' }
  if ((Get-FileHash -LiteralPath $codexCache -Algorithm SHA256).Hash -ne $previousHash) { throw 'Cancelling refresh overwrote the complete disk cache' }
  [pscustomobject]@{Passed=$true; FixtureBytes=$fixtureBytes; Records=400000; Progress=$active.Progress; CancelMilliseconds=$cancelMilliseconds; NavigationCancelled=$true; ClosingCancelled=$true; ReopenedSameHost=$true; CompleteRescanTokens=150; NoPartialCache=$true; ManualRefreshRescans=$true; PreviousResultPreserved=$true} | ConvertTo-Json | Tee-Object -FilePath 'target/spend-ui-result.json'
} finally {
  if (-not $testApp.HasExited) { $testApp.Kill(); $testApp.WaitForExit() }
  $resolved = [IO.Path]::GetFullPath($profile)
  if ((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')) { throw 'Unexpected test profile location' }
  Remove-Item -LiteralPath $resolved -Recurse -Force
}
