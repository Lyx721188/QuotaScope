# Verify the normal settings entry points and persistence for the November upstream batch.
# Synthetic profile and cached session account, outbound requests rejected by a loopback proxy.
param([Parameter(Mandatory=$true)][string]$Executable,[string]$OutputDirectory='target/upstream-features-ui-validation')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
$id=[guid]::NewGuid().ToString('N')
$profile=Join-Path $env:TEMP ('qs-upstream-features-'+$id)
$data=Join-Path $profile 'appdata/QuotaScope'
New-Item -ItemType Directory -Path $data,(Join-Path $profile 'home'),$OutputDirectory -Force|Out-Null
$proxy=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0);$proxy.Start()
$seed=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
$settings=@{hasRun=$true;enabledAccounts=@('openDesign');language='en';manualExchangeRates=@{CNY=7.2};readsTokenSpend=$false;checksForUpdates=$false;lastUpdateCheckAt=$seed;proxyMode='manual';proxyUrl="http://127.0.0.1:$($proxy.LocalEndpoint.Port)"}
$fixtureVersion=[regex]::Match([Diagnostics.FileVersionInfo]::GetVersionInfo([IO.Path]::GetFullPath($Executable)).ProductVersion,'\d+\.\d+\.\d+').Value
$settings.notices=@{lastVersion=$fixtureVersion}
$settingsPath=Join-Path $data 'settings.json'
[IO.File]::WriteAllText($settingsPath,($settings|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
# A previous session reading must be checked after releasing the settings lock.
$cachedSession=@{openDesign=@{observed_at=$seed;windows=@();plan='Synthetic';credential_context='synthetic-old-context'}}
[IO.File]::WriteAllText((Join-Path $data 'last-readings.json'),($cachedSession|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
$info=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings');$info.UseShellExecute=$false
$info.Environment['APPDATA']=Join-Path $profile 'appdata'
$info.Environment['LOCALAPPDATA']=Join-Path $profile 'local'
$info.Environment['USERPROFILE']=Join-Path $profile 'home'
$info.Environment['HERMES_HOME']=Join-Path $profile 'hermes'
$info.Environment['QUOTASCOPE_TEST_INSTANCE']=$id
$testApp=[Diagnostics.Process]::Start($info)
$condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
$root=[Windows.Automation.AutomationElement]::RootElement
$script:connections=0
function Reject-Connections {while($proxy.Pending()){$client=$proxy.AcceptTcpClient();$client.Dispose();$script:connections++}}
function Nodes {
    Reject-Connections
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do {
        if($testApp.HasExited){throw 'Isolated update app exited'}
        try {
            $script:window=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {($_.Current.Name -like 'QuotaScope *' -or $_.Current.Name -like '* QuotaScope') -and -not $_.Current.IsOffscreen}|Select-Object -First 1
            if($window){return $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)}
        }catch [Windows.Automation.ElementNotAvailableException] {}
        Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $until)
    throw 'Settings window unavailable'
}
function Wait-Text([string]$name,[switch]$Absent) {
    $until=[DateTime]::UtcNow.AddSeconds(60)
    do {$found=@(Nodes|Where-Object {$_.Current.Name -eq $name}).Count -gt 0;if($found -ne $Absent.IsPresent){return};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
    throw "Settings text condition failed: $name (absent=$Absent)"
}
function Page([string]$name) {
    Wait-Text $name
    $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1
    if(-not $node){throw "Missing update navigation: $name"}
    $node.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
function Combo([string]$name){
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do{
        $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::ComboBox}|Select-Object -First 1
        if($node){break};Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $until)
    if(-not $node){throw "Missing settings combo: $name"}
    $node
}
function Select-ComboItem($combo,[string]$item){
    $combo.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do{
        $target=$root.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)|Where-Object {$_.Current.Name -eq $item -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::ListItem}|Select-Object -First 1
        if(-not $target){Start-Sleep -Milliseconds 100}
    }while(-not $target -and [DateTime]::UtcNow -lt $until)
    if(-not $target){throw "Settings combo item missing: $item"}
    $target.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
function Press([string]$name){
    # Closing a ComboBox rebuilds the page; let the replacement control appear.
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do {
        $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button}|Select-Object -First 1
        if($node){break}
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if(-not $node){throw "Missing history button: $name"}
    $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}

function Set-Toggle([string]$name,[bool]$on){
    Wait-Text $name
    $node=Nodes|Where-Object {$_.Current.Name -eq $name -and $_.Current.ClassName -eq 'ToggleSwitch'}|Select-Object -First 1
    if(-not $node){throw "Toggle missing: $name"}
    $pattern=$node.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
    if(($pattern.Current.ToggleState -eq [Windows.Automation.ToggleState]::On) -ne $on){$pattern.Toggle()}
}
function Saved {Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json}
function Check-Saved([scriptblock]$check){
    $until=[DateTime]::UtcNow.AddSeconds(6)
    do{if(&$check (Saved)){return};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
    throw "Setting did not persist: $check"
}
function Configure([string]$name){
    Page 'Accounts'
    $search=Nodes|Where-Object {$_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit -and $_.Current.ClassName -eq 'TextBox'}|Select-Object -First 1
    $search.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue($name)
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do{
        $buttons=@(Nodes|Where-Object {$_.Current.Name -in @('Configure','Collapse') -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button})
        if($buttons.Count -eq 1){break};Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $until)
    if($buttons.Count -ne 1){throw "Account not isolated: $name"}
    if($buttons[0].Current.Name -eq 'Configure'){$buttons[0].GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()}
}
try{
    $phase='general'
    Page 'General'
    Select-ComboItem (Combo 'Browser for session import') 'Edge'
    Select-ComboItem (Combo 'Progress bars') 'Segmented'
    Set-Toggle 'Snap to screen edges' $false
    Set-Toggle 'Elastic docking' $true
    Check-Saved {param($s) $s.importBrowser -eq 'edge' -and $s.progressBars -eq 'segmented' -and -not $s.snapsToEdges -and $s.liquidDocking}
    $phase='currency'
    Select-ComboItem (Combo 'Estimate currency') 'Chinese yuan (¥)'
    Check-Saved {param($s) $s.displayCurrency -eq 'CNY'}
    # UIA ValuePattern can update the XAML text without raising TextChanged, and
    # a view rebuild then restores the last committed draft. Type into the
    # focused editor so the value always reaches the app before Save.
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do {
      $edit=Nodes|Where-Object {$_.Current.Name -eq 'Manual exchange rate' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit}|Select-Object -First 1
      if($edit -and $edit.Current.IsEnabled){break}
      Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $until)
    if(!$edit -or -not $edit.Current.IsEnabled){throw 'Manual exchange rate editor did not become enabled'}
    $edit.SetFocus()
    Start-Sleep -Milliseconds 200
    [Windows.Forms.SendKeys]::SendWait('^a')
    [Windows.Forms.SendKeys]::SendWait('7.25invalid')
    Start-Sleep -Milliseconds 300
    Press 'Save rate'
    Wait-Text 'Enter a positive exchange rate, or leave it blank for the daily rate.'
    if((Saved).manualExchangeRates.CNY -ne 7.2){throw 'Invalid rate changed the saved rate'}
    $edit=Nodes|Where-Object {$_.Current.Name -eq 'Manual exchange rate' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit}|Select-Object -First 1
    $edit.SetFocus()
    Start-Sleep -Milliseconds 200
    [Windows.Forms.SendKeys]::SendWait('^a')
    [Windows.Forms.SendKeys]::SendWait('7.25')
    Start-Sleep -Milliseconds 300
    if($edit.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).Current.Value -ne '7.25'){throw 'Valid rate text did not reach the editor'}
    Press 'Save rate'
    Check-Saved {param($s) $s.manualExchangeRates.CNY -eq 7.25}
    $phase='open-design'
    Configure 'Open Design'
    Wait-Text 'Sign in at open-design.ai and import its browser session, or sign in to the Open Design desktop app. Model API keys cannot read usage.'
    Select-ComboItem (Combo 'Browser for session import') 'Default browser'
    Check-Saved {param($s) $s.importBrowser -eq 'default'}
    Wait-Text 'Or paste a Cookie header'
    Wait-Text 'Connection diagnostics'
    Press 'Check connection'
    Wait-Text 'Import a browser session in Settings.'
    Select-ComboItem (Combo 'Browser for session import') 'Firefox'
    Press 'Import from browser'
    $importFailure='No readable browser session was found. Sign in to the selected browser. For protected Chrome or Edge cookies, select All browsers and connect OpenCLI Browser Bridge, or paste a Cookie header.'
    Wait-Text $importFailure
    Start-Sleep -Seconds 2
    Wait-Text $importFailure
    Select-ComboItem (Combo 'Browser for session import') 'Default browser'
    $phase='dimagent'
    Configure 'DimAgent'
    Wait-Text "Uses dim usage --json with the CLI's existing login. Sign in with dim auth login first."
    $phase='sites'
    Configure 'MiniMax'
    Select-ComboItem (Combo 'Account site') 'minimaxi.com'
    Check-Saved {param($s) $s.minimaxSite -eq 'china'}
    Select-ComboItem (Combo 'Account site') 'minimax.io'
    Check-Saved {param($s) $s.minimaxSite -eq 'international'}
    Configure 'Qoder'
    Select-ComboItem (Combo 'Account site') 'qoder.com.cn'
    Check-Saved {param($s) $s.qoderSite -eq 'china'}
    Wait-Text 'Usage history'
    $phase='avatar'
    Configure 'Codex'
    Select-ComboItem (Combo 'Ring mark') 'Agent Robot Avatar'
    Select-ComboItem (Combo 'Personality') 'Curious'
    Select-ComboItem (Combo 'Head shape') 'Rounder'
    Check-Saved {param($s) $s.robotAvatars.codex.enabled -and $s.robotAvatars.codex.personality -eq 'curious' -and $s.robotAvatars.codex.shape -eq 'round'}
    $phase='antigravity'
    Configure 'Antigravity'
    Select-ComboItem (Combo 'Read usage from') 'CLI'
    Set-Toggle 'One ring per model group' $true
    Check-Saved {param($s) $s.antigravitySource -eq 'cli' -and $s.splitAccounts -contains 'antigravity'}
    $phase='russian'
    Page 'General'
    Select-ComboItem (Combo 'Language') 'Русский'
    Wait-Text 'Основные'
    Check-Saved {param($s) $s.language -eq 'ru'}
    Select-ComboItem (Combo 'Язык') 'English'
    Wait-Text 'General'
    $phase='reopen'
    $window.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close()
    $reopen=[Diagnostics.Process]::Start($info)
    if(-not $reopen.WaitForExit(15000) -or $reopen.ExitCode -ne 0){throw 'Settings reopen failed'}
    Page 'General'
    Check-Saved {param($s) $s.importBrowser -eq 'default' -and $s.progressBars -eq 'segmented' -and -not $s.snapsToEdges -and $s.manualExchangeRates.CNY -eq 7.25 -and $s.language -eq 'en'}
    [pscustomobject]@{Passed=$true;Sha256=(Get-FileHash -LiteralPath $Executable).Hash;StartupWithCachedSession=$true;OpenDesignEntry=$true;MissingSessionDiagnostic=$true;PersistentImportFailure=$true;DimAgentEntry=$true;BrowserChoicePersisted=$true;SegmentedAndFreePlacement=$true;ElasticDocking=$true;StrictManualRate=$true;SiteChoicePersisted=$true;AvatarChoicePersisted=$true;AntigravityRouteAndSplit=$true;RussianAndEnglish=$true;CloseAndReopen=$true;RealAccountVerified=$false;ExternalRequestsBlocked=$true}|ConvertTo-Json|Tee-Object -FilePath (Join-Path $OutputDirectory 'upstream-features-ui-result.json')
}catch{
    $failure=$_
    Write-Output "Failed phase: $phase"
    try{Nodes|ForEach-Object {[pscustomobject]@{Name=$_.Current.Name;Class=$_.Current.ClassName;Type=$_.Current.ControlType.ProgrammaticName}}|ConvertTo-Json -Depth 3|Set-Content (Join-Path $OutputDirectory 'upstream-features-ui-failure.json')}catch{}
    throw $failure
}finally{
    if(-not $testApp.HasExited){$testApp.Kill();$testApp.WaitForExit()};$proxy.Stop()
    $resolved=[IO.Path]::GetFullPath($profile)
    if((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') -or (Split-Path $resolved -Leaf) -ne ('qs-upstream-features-'+$id)){throw 'Unexpected feature profile'}
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
