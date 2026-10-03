# Run from windows/ with a debug build. Tests an isolated empty profile and
# debug-only instance namespace; it does not install an update or use keys.
param([Parameter(Mandatory = $true)][string]$Executable)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$testId = [guid]::NewGuid().ToString('N')
$profile = Join-Path $env:TEMP ('quotascope-ui-' + $testId)
$settingsPath = Join-Path $profile 'QuotaScope/settings.json'
New-Item -ItemType Directory -Path (Split-Path $settingsPath) -Force | Out-Null
Set-Content -LiteralPath $settingsPath -Value '{"hasRun":true,"enabledAccounts":[],"language":"zh","readsTokenSpend":false}' -Encoding utf8
$info = [Diagnostics.ProcessStartInfo]::new($Executable)
$info.ArgumentList.Add('--settings')
$info.UseShellExecute = $false
$info.Environment['APPDATA'] = $profile
$info.Environment['QUOTASCOPE_TEST_INSTANCE'] = $testId
$testApp = [Diagnostics.Process]::Start($info)
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
  $nodes = $window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
  $nodes | ForEach-Object { [pscustomobject]@{Name=$_.Current.Name; Class=$_.Current.ClassName; Patterns=(($_.GetSupportedPatterns() | ForEach-Object ProgrammaticName) -join ',')} } | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath 'target/updates-ui-initial.json' -Encoding utf8
  $about = $nodes | Where-Object { $_.Current.Name -eq '关于' } | Select-Object -First 1
  if (-not $about) { throw 'About navigation unavailable' }
  $about.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
  Start-Sleep -Milliseconds 750
  $nodes = $window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
  $nodes | ForEach-Object { [pscustomobject]@{Name=$_.Current.Name; Class=$_.Current.ClassName; Patterns=(($_.GetSupportedPatterns() | ForEach-Object ProgrammaticName) -join ',')} } | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath 'target/updates-ui-about.json' -Encoding utf8
  $names = @($nodes | ForEach-Object { $_.Current.Name })
  if ('自动检查更新' -notin $names -or '检查更新' -notin $names) { throw 'Update controls missing' }
  $before = Get-Content -LiteralPath $settingsPath -Raw | ConvertFrom-Json
  if ($before.checksForUpdates -or $before.lastUpdateCheckAt) { throw 'Default performed an automatic check' }
  $check = $nodes | Where-Object { $_.Current.Name -eq '检查更新' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button } | Select-Object -First 1
  $check.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
  $until = [DateTime]::UtcNow.AddSeconds(25)
  do {
    Start-Sleep -Milliseconds 250
    $nodes = $window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
    $names = @($nodes | ForEach-Object { $_.Current.Name })
    $result = $names | Where-Object { $_ -like '检查更新失败*' -or $_ -like '当前已是最新*' -or $_ -like '可用的 Windows 更新*' } | Select-Object -First 1
  } until ($result -or [DateTime]::UtcNow -gt $until)
  if (-not $result) { throw 'Manual check did not finish' }
  $after = Get-Content -LiteralPath $settingsPath -Raw | ConvertFrom-Json
  if (-not $after.lastUpdateCheckAt -or $after.checksForUpdates) { throw 'Manual check persistence incorrect' }
  $switch = $nodes | Where-Object { $_.Current.ClassName -eq 'ToggleSwitch' } | Select-Object -First 1
  $toggle = $switch.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
  $toggle.Toggle()
  Start-Sleep -Milliseconds 500
  if (-not (Get-Content -LiteralPath $settingsPath -Raw | ConvertFrom-Json).checksForUpdates) { throw 'Update opt-in was not persisted' }
  $toggle.Toggle()
  Start-Sleep -Milliseconds 500
  if ((Get-Content -LiteralPath $settingsPath -Raw | ConvertFrom-Json).checksForUpdates) { throw 'Update opt-out was not persisted' }
  [pscustomobject]@{Passed=$true; Result=$result; DefaultsToManual=$true; TogglePersisted=$true; ProcessId=$testApp.Id} | ConvertTo-Json | Tee-Object -FilePath 'target/updates-ui-result.json'
} finally {
  if (-not $testApp.HasExited) { $testApp.Kill(); $testApp.WaitForExit() }
  $profileResolved = [IO.Path]::GetFullPath($profile)
  if ((Split-Path $profileResolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')) { throw 'Unexpected test profile location' }
  Remove-Item -LiteralPath $profileResolved -Recurse -Force
}
