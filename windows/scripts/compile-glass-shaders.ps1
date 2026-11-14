param([string]$Fxc)
$ErrorActionPreference = 'Stop'
if (-not $Fxc) {
    $kit = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $Fxc = Get-ChildItem -Path $kit -Filter fxc.exe -Recurse |
        Where-Object { $_.Directory.Name -eq 'x64' } |
        Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $Fxc) { throw 'Windows SDK fxc.exe was not found. Pass -Fxc with its path.' }
$shaderDir = Join-Path $PSScriptRoot '..\quotascope-win\shaders'
$source = Join-Path $shaderDir 'liquid_glass.hlsl'
foreach ($shader in @(
    @('vs_main', 'vs_4_0', 'liquid_glass_vs.cso'),
    @('ps_blur_h', 'ps_4_0', 'liquid_glass_blur_h_ps.cso'),
    @('ps_blur_h_hdr', 'ps_4_0', 'liquid_glass_blur_h_hdr_ps.cso'),
    @('ps_blur_v', 'ps_4_0', 'liquid_glass_blur_v_ps.cso'),
    @('ps_main', 'ps_4_0', 'liquid_glass_ps.cso')
)) {
    & $Fxc /nologo /O3 /T $shader[1] /E $shader[0] /Fo (Join-Path $shaderDir $shader[2]) $source
    if ($LASTEXITCODE -ne 0) { throw "FXC failed for $($shader[0])" }
}
