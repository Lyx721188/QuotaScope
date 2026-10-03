[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputPath
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) { throw 'QuotaScope executable was not found.' }
$text = (& $Executable --json | Out-String)
if ($LASTEXITCODE -ne 0) { throw 'QuotaScope report failed.' }
$report = $text | ConvertFrom-Json
$rows = @(
    foreach ($account in $report.accounts) {
        foreach ($window in $account.windows) {
            [pscustomobject]@{
                Account = $account.id
                Provider = $account.provider
                Plan = $account.plan
                Window = $window.id
                Scope = $window.scope
                UsedPercent = $window.usedPercent
                Estimated = $window.estimated
                ObservedAt = $account.observedAt
                AgeSeconds = $account.ageSeconds
                Source = $account.source
                ResetsAt = $window.resetsAt
            }
        }
    }
)
if ($OutputPath) { $rows | Export-Csv -LiteralPath $OutputPath -NoTypeInformation -Encoding UTF8 }
else { $rows | ConvertTo-Csv -NoTypeInformation }
