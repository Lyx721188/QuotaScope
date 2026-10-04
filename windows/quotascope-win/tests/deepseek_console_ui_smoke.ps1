# Run from windows/ with a complete Debug payload and an unlocked desktop.
# All credentials and Chromium files are synthetic. A loopback-only failed
# proxy prevents enabled test accounts from reaching any external service.
param([Parameter(Mandatory = $true)][string]$Executable)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Security.Cryptography.ProtectedData
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Text;
public static class DeepSeekFixture {
    static void Var(Stream stream, int value) {
        while (value >= 128) { stream.WriteByte((byte)((value & 127) | 128)); value >>= 7; }
        stream.WriteByte((byte)value);
    }
    public static byte[] Record(byte[] data) {
        uint crc = 0xffffffff;
        foreach (byte value in new byte[] { 1 }) { crc ^= value; for (int n=0;n<8;n++) crc = (crc >> 1) ^ ((crc & 1) == 1 ? 0x82f63b78u : 0); }
        foreach (byte value in data) { crc ^= value; for (int n=0;n<8;n++) crc = (crc >> 1) ^ ((crc & 1) == 1 ? 0x82f63b78u : 0); }
        crc = ~crc;
        crc = ((crc >> 15) | (crc << 17)) + 0xa282ead8u;
        using (var stream = new MemoryStream()) {
            stream.Write(BitConverter.GetBytes(crc), 0, 4);
            stream.Write(BitConverter.GetBytes((ushort)data.Length), 0, 2);
            stream.WriteByte(1); stream.Write(data, 0, data.Length); return stream.ToArray();
        }
    }
    public static void Storage(string directory, string origin, string token, bool deleted) {
        Directory.CreateDirectory(directory);
        File.WriteAllText(Path.Combine(directory, "CURRENT"), "MANIFEST-000001\n", new UTF8Encoding(false));
        File.WriteAllBytes(Path.Combine(directory, "MANIFEST-000001"), Record(new byte[] { 2, 2, 9, 0 }));
        byte[] key = Encoding.UTF8.GetBytes("_" + origin + "\0\u0001userToken");
        byte[] value = Encoding.UTF8.GetBytes("\u0001{\"value\":\"" + token + "\"}");
        using (var stream = new MemoryStream()) {
            stream.Write(BitConverter.GetBytes((ulong)20), 0, 8);
            stream.Write(BitConverter.GetBytes(1), 0, 4);
            stream.WriteByte(deleted ? (byte)0 : (byte)1);
            Var(stream, key.Length); stream.Write(key, 0, key.Length);
            if (!deleted) { Var(stream, value.Length); stream.Write(value, 0, value.Length); }
            File.WriteAllBytes(Path.Combine(directory, "000002.log"), Record(stream.ToArray()));
        }
    }
}
'@
$testId = [guid]::NewGuid().ToString('N')
$profile = Join-Path $env:TEMP ('qs-deepseek-ui-' + $testId)
$data = Join-Path $profile 'appdata/QuotaScope'
$local = Join-Path $profile 'local'
$browser = Join-Path $local 'Microsoft/Edge/User Data/Profile 1/Local Storage/leveldb'
$unrelated = Join-Path $local 'Microsoft/Edge/User Data/Default/Local Storage/leveldb'
New-Item -ItemType Directory -Path $data -Force | Out-Null
[DeepSeekFixture]::Storage($unrelated, 'https://unrelated.example', 'synthetic-unrelated', $false)
[DeepSeekFixture]::Storage($browser, 'https://platform.deepseek.com', 'synthetic-console-A', $false)
$reserve = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$reserve.Start()
$proxyPort = $reserve.LocalEndpoint.Port
# Keep the listener reserved without accepting connections. It cannot tunnel
# requests or forward credentials, and is stopped during cleanup.
$settings = @{hasRun=$true; enabledAccounts=@('deepSeek#1'); detailedCards=@(); language='zh'; readsTokenSpend=$false; proxyMode='manual'; proxyUrl="http://127.0.0.1:$proxyPort"}
[IO.File]::WriteAllText((Join-Path $data 'settings.json'), ($settings | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
function Write-Keys($keys) {
    $plain = [Text.Encoding]::UTF8.GetBytes(($keys | ConvertTo-Json -Compress))
    $blob = [Security.Cryptography.ProtectedData]::Protect($plain, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    [IO.File]::WriteAllBytes((Join-Path $data 'keys.dat'), ([Text.Encoding]::ASCII.GetBytes('QSCOPEK1') + $blob))
}
function Read-Keys {
    $bytes = [IO.File]::ReadAllBytes((Join-Path $data 'keys.dat'))
    if ([Text.Encoding]::ASCII.GetString($bytes,0,8) -ne 'QSCOPEK1') { throw 'Encrypted store header missing' }
    if ([Text.Encoding]::ASCII.GetString($bytes).Contains('synthetic-')) { throw 'Credential persisted in plaintext' }
    $plain = [Security.Cryptography.ProtectedData]::Unprotect($bytes[8..($bytes.Length-1)], $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    [Text.Encoding]::UTF8.GetString($plain) | ConvertFrom-Json -AsHashtable
}
Write-Keys @{'deepSeek'='synthetic-api-primary'; 'deepSeek#1'='synthetic-api-extra'; 'deepSeek#1:console'='synthetic-extra-old'}
$info = [Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings')
$info.UseShellExecute = $false
$info.Environment['APPDATA'] = Join-Path $profile 'appdata'
$info.Environment['LOCALAPPDATA'] = $local
$info.Environment['USERPROFILE'] = Join-Path $profile 'home'
$info.Environment['HERMES_HOME']=Join-Path ($info.Environment['USERPROFILE']) '.hermes'
$info.Environment['QUOTASCOPE_TEST_INSTANCE'] = $testId
$testApp = [Diagnostics.Process]::Start($info)
function Get-Nodes {
    if ($testApp.HasExited) { throw 'Isolated application exited' }
    $window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
}
function Wait-Node([string]$name) {
    $until = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $node = Get-Nodes | Where-Object { $_.Current.Name -eq $name } | Select-Object -First 1
        if ($node) { return $node }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $until)
    throw "UI text missing: $name"
}
function Click([string]$name) {
    $node = Wait-Node $name
    $until = [DateTime]::UtcNow.AddSeconds(15)
    while (-not $node.Current.IsEnabled -and [DateTime]::UtcNow -lt $until) { Start-Sleep -Milliseconds 100 }
    $node.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}
function Select-Page([string]$name) {
    $node = Get-Nodes | Where-Object { $_.Current.Name -eq $name -and $_.Current.ClassName -like '*NavigationViewItem' } | Select-Object -First 1
    if (-not $node) { throw "Navigation missing: $name" }
    $node.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
}
function Configure-DeepSeek {
    Select-Page '账户'
    $until = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $search = Get-Nodes | Where-Object { $_.Current.ControlType -eq [Windows.Automation.ControlType]::Edit -and $_.Current.ClassName -eq 'TextBox' } | Select-Object -First 1
        if (-not $search) { Start-Sleep -Milliseconds 100 }
    } while (-not $search -and [DateTime]::UtcNow -lt $until)
    if (-not $search) { throw 'Account search field missing' }
    $search.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('DeepSeek')
    do {
        $controls = @(Get-Nodes | Where-Object { $_.Current.Name -in @('配置','收起') -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::Button })
        if ($controls.Count -ne 1) { Start-Sleep -Milliseconds 100 }
    } while ($controls.Count -ne 1 -and [DateTime]::UtcNow -lt $until)
    if ($controls.Count -ne 1) { throw 'Account search did not select DeepSeek alone' }
    $configure = $controls | Where-Object { $_.Current.Name -eq '配置' } | Select-Object -First 1
    if ($configure) { $configure.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke() }
    [void](Wait-Node '导入 DeepSeek 官网会话，可读取整个账户近 30 个自然日的 Token 和实际账单。API key 独立保存。')
}
function Wait-Key([string]$id, $expected) {
    $until = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $keys = Read-Keys
        if ($keys[$id] -eq $expected) { return $keys }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $until)
    throw "Credential slot did not reach expected state: $id"
}
try {
    $root = [Windows.Automation.AutomationElement]::RootElement
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, $testApp.Id)
    $until = [DateTime]::UtcNow.AddSeconds(15)
    do {
        Start-Sleep -Milliseconds 200
        if ($testApp.HasExited) { throw 'Isolated application exited before settings opened' }
        $window = $root.FindAll([Windows.Automation.TreeScope]::Children, $condition) | Where-Object { $_.Current.Name -like 'QuotaScope *' } | Select-Object -First 1
    } while (-not $window -and [DateTime]::UtcNow -lt $until)
    if (-not $window) { throw 'Settings window missing' }
    Configure-DeepSeek
    Click '从浏览器导入'
    $keys = Wait-Key 'deepSeek:console' 'synthetic-console-A'
    [void](Wait-Node '已从 Edge 导入会话。')
    if ($keys['deepSeek'] -ne 'synthetic-api-primary' -or $keys['deepSeek#1'] -ne 'synthetic-api-extra' -or $keys['deepSeek#1:console'] -ne 'synthetic-extra-old') { throw 'Primary import overwrote another credential slot' }
    [DeepSeekFixture]::Storage($browser, 'https://platform.deepseek.com', 'synthetic-console-A', $true)
    Click '从浏览器导入'
    [void](Wait-Node '在浏览器里没有找到匹配的会话。')
    [void](Wait-Key 'deepSeek:console' 'synthetic-console-A')
    [DeepSeekFixture]::Storage($browser, 'https://platform.deepseek.com', 'synthetic-console-B', $false)
    Select-Page '用量概览'
    [void](Wait-Node '导入前请在浏览器登录对应的 DeepSeek 账户。附加账户不会从其他浏览器登录自动续期。')
    Click '从浏览器导入'
    $keys = Wait-Key 'deepSeek#1:console' 'synthetic-console-B'
    if ($keys['deepSeek:console'] -ne 'synthetic-console-A') { throw 'Extra import overwrote primary console slot' }
    Click '清除官网会话'
    $keys = Wait-Key 'deepSeek#1:console' $null
    [void](Wait-Node '官网会话已清除。')
    if ($keys['deepSeek#1'] -ne 'synthetic-api-extra' -or $keys['deepSeek:console'] -ne 'synthetic-console-A') { throw 'Extra clear removed API key or primary session' }
    Configure-DeepSeek
    Click '清除官网会话'
    $keys = Wait-Key 'deepSeek:console' $null
    [void](Wait-Node '官网会话已清除。')
    if ($keys.Count -ne 2) { throw 'Clear left unexpected credential slots' }
    [pscustomobject]@{Passed=$true; DebugExecutable=[IO.Path]::GetFullPath($Executable); Sha256=(Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash; Browser='Edge Profile 1'; OriginFiltered=$true; DeletionRespected=$true; DpapiEncrypted=$true; ApiKeysPreserved=$true; AccountSessionsIsolated=$true; ExtraAndPrimaryClear=$true; LocalTokenReadingDisabled=$true; ExternalRequestsBlockedByLoopbackProxy=$true; RealAccountVerified=$false} | ConvertTo-Json | Tee-Object -FilePath 'target/upstream-followthrough-20261004/n2d-console-ui-result.json'
} finally {
    if (-not $testApp.HasExited) { $testApp.Kill(); $testApp.WaitForExit() }
    $reserve.Stop()
    $resolved = [IO.Path]::GetFullPath($profile)
    if ((Split-Path $resolved) -ne [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')) { throw 'Unexpected test profile location' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
