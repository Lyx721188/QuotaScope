param([Parameter(Mandatory=$true)][string]$Executable,[string]$OutputDirectory='target/pulse-ui-validation')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class PulseWindow {
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int left, top, right, bottom; }
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct WNDCLASS { public uint style; public IntPtr lpfnWndProc; public int cbClsExtra; public int cbWndExtra; public IntPtr hInstance; public IntPtr hIcon; public IntPtr hCursor; public IntPtr hbrBackground; public string lpszMenuName; public string lpszClassName; }
  public delegate IntPtr WndProcDelegate(IntPtr h, uint m, IntPtr w, IntPtr l);
  static WndProcDelegate proc = WndProc;
  static IntPtr WndProc(IntPtr h, uint m, IntPtr w, IntPtr l) { return DefWindowProcW(h,m,w,l); }
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, IntPtr data);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder name, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT rect);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern ushort RegisterClassW(ref WNDCLASS c);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr CreateWindowExW(uint ex, string cls, string name, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr inst, IntPtr param);
  [DllImport("user32.dll")] public static extern IntPtr DefWindowProcW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool UpdateWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool DestroyWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool InvalidateRect(IntPtr h, IntPtr r, bool erase);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern IntPtr SetClassLongPtrW(IntPtr h, int index, IntPtr value);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateSolidBrush(uint color);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr GetModuleHandleW(string name);
  public static IntPtr Find(uint pid, string name) { IntPtr found=IntPtr.Zero; EnumWindows((h,p)=>{uint id; GetWindowThreadProcessId(h,out id); var s=new StringBuilder(256); GetClassName(h,s,s.Capacity); if(id==pid && s.ToString()==name && IsWindowVisible(h)) {found=h;return false;} return true;},IntPtr.Zero); return found; }
  // A plain topmost window painted from its class brush: the test needs a
  // real desktop surface behind the panel, not a WinForms window that never
  // gets a message loop.
  public static IntPtr Backdrop(uint colorref, RECT bounds) {
    var cls = "QuotaScopeGlassBackdrop" + Guid.NewGuid().ToString("N");
    var wc = new WNDCLASS { style=3, lpfnWndProc=Marshal.GetFunctionPointerForDelegate(proc), hInstance=GetModuleHandleW(null), hbrBackground=CreateSolidBrush(colorref), lpszClassName=cls };
    if (RegisterClassW(ref wc)==0) throw new Exception("RegisterClass failed " + Marshal.GetLastWin32Error());
    var hwnd = CreateWindowExW(0x8 /*WS_EX_TOPMOST*/, cls, "QuotaScopeGlassBackdrop", 0x80000000u | 0x10000000u, bounds.left, bounds.top, bounds.right-bounds.left, bounds.bottom-bounds.top, IntPtr.Zero, IntPtr.Zero, wc.hInstance, IntPtr.Zero);
    if (hwnd==IntPtr.Zero) throw new Exception("CreateWindow failed " + Marshal.GetLastWin32Error());
    ShowWindow(hwnd, 5); UpdateWindow(hwnd);
    return hwnd;
  }
  public static void Recolor(IntPtr hwnd, uint colorref) {
    SetClassLongPtrW(hwnd, -10 /*GCLP_HBRBACKGROUND*/, CreateSolidBrush(colorref));
    InvalidateRect(hwnd, IntPtr.Zero, true);
    UpdateWindow(hwnd);
  }
  public static void RaiseAbove(IntPtr hwnd) {
    SetWindowPos(hwnd, new IntPtr(-1) /*HWND_TOPMOST*/, 0, 0, 0, 0, 0x0002 | 0x0001 | 0x0010 /*NOMOVE|NOSIZE|NOACTIVATE*/);
  }
}
'@
$evidence=[IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $evidence | Out-Null
$exeIcon=[Drawing.Icon]::ExtractAssociatedIcon([IO.Path]::GetFullPath($Executable))
if(!$exeIcon){throw 'Compiled executable icon is missing'}
$iconBitmap=$exeIcon.ToBitmap()
try {$iconBitmap.Save((Join-Path $evidence 'pulse-icon.png'),[Drawing.Imaging.ImageFormat]::Png)} finally {$iconBitmap.Dispose();$exeIcon.Dispose()}
$id=[guid]::NewGuid().ToString('N')
$profile=Join-Path $env:TEMP ('qs-pulse-'+$id)
$data=Join-Path $profile 'appdata/QuotaScope'
New-Item -ItemType Directory -Force -Path $data,(Join-Path $profile 'home') | Out-Null
$settings=@{hasRun=$true;enabledAccounts=@('codex');dockSide='left';language='zh';readsTokenSpend=$false;checksForUpdates=$false;autoCollapse=$false;proxyMode='manual';proxyUrl='http://127.0.0.1:9'}
$fixtureVersion=[regex]::Match([Diagnostics.FileVersionInfo]::GetVersionInfo([IO.Path]::GetFullPath($Executable)).ProductVersion,'\d+\.\d+\.\d+').Value
$settings.notices=@{lastVersion=$fixtureVersion}
$settingsPath=Join-Path $data 'settings.json'
[IO.File]::WriteAllText($settingsPath,($settings|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
$info=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
$info.ArgumentList.Add('--settings');$info.UseShellExecute=$false
$info.Environment['APPDATA']=Join-Path $profile 'appdata'
$info.Environment['LOCALAPPDATA']=Join-Path $profile 'local'
$info.Environment['USERPROFILE']=Join-Path $profile 'home'
$info.Environment['HERMES_HOME']=Join-Path $profile 'hermes'
$info.Environment['QUOTASCOPE_TEST_INSTANCE']=$id
$testApp=[Diagnostics.Process]::Start($info)
try {
  $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
  function TopWindow {
    if($testApp.HasExited){throw 'Appearance app exited'}
    return [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,$condition)|Where-Object {$_.Current.Name -like 'QuotaScope *' -and -not $_.Current.IsOffscreen}|Select-Object -First 1
  }
  function Nodes {
    $window=TopWindow
    if(!$window){return @()}
    return $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
  }
  # The Settings window is wide enough to sit over the card that a left-docked
  # Pulse rail opens. Keep it on the right, fully visible for UIA, so the rail
  # and card are sampled against the actual desktop.
  function MoveSettingsAside {
    $window=TopWindow
    if(!$window){return}
    $hwnd=[IntPtr]$window.Current.NativeWindowHandle
    if($hwnd -eq [IntPtr]::Zero){return}
    $bounds=New-Object PulseWindow+RECT
    if(-not [PulseWindow]::GetWindowRect($hwnd,[ref]$bounds)){return}
    $screen=[Windows.Forms.Screen]::PrimaryScreen.Bounds
    $x=[Math]::Max(0,$screen.Right-($bounds.right-$bounds.left)-24)
    [PulseWindow]::SetWindowPos($hwnd,[IntPtr]::Zero,$x,72,0,0,0x0015) | Out-Null
  }
  function SamplePixel([int]$x,[int]$y) {
    $bmp=[Drawing.Bitmap]::new(1,1)
    $g=[Drawing.Graphics]::FromImage($bmp)
    try {$g.CopyFromScreen($x,$y,0,0,$bmp.Size);return $bmp.GetPixel(0,0)}finally{$g.Dispose();$bmp.Dispose()}
  }
  $until=[DateTime]::UtcNow.AddSeconds(30)
  do {$page=Nodes|Where-Object {$_.Current.Name -eq '通用' -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1;if($page){break};Start-Sleep -Milliseconds 200}while([DateTime]::UtcNow -lt $until)
  if(!$page){throw 'General page missing'}
  $page.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
  function Choose([string]$option,[string]$expected) {
    $until=[DateTime]::UtcNow.AddSeconds(20)
    do {$combo=Nodes|Where-Object {$_.Current.Name -eq '侧边栏外观' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::ComboBox}|Select-Object -First 1;if($combo){break};Start-Sleep -Milliseconds 200}while([DateTime]::UtcNow -lt $until)
    if(!$combo){throw 'Appearance picker missing'}
    $combo.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do {
      $item=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)|Where-Object {$_.Current.Name -eq $option -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::ListItem}|Select-Object -First 1
      if($item){break};Start-Sleep -Milliseconds 100
    }while([DateTime]::UtcNow -lt $until)
    if(!$item){throw "Appearance option missing: $option"}
    $item.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $until=[DateTime]::UtcNow.AddSeconds(10)
    do {if((Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json).panelAppearance -eq $expected){return};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
    throw "Appearance was not persisted: $expected"
  }
  Choose 'Pulse' 'pulse'
  Choose 'Windows' 'windows'
  Choose 'Pulse' 'pulse'
  MoveSettingsAside
  Start-Sleep -Seconds 2
  $panel=[PulseWindow]::Find([uint32]$testApp.Id,'QuotaScopePanel')
  if($panel -eq [IntPtr]::Zero){throw 'Pulse panel missing after switching'}
  $rect=New-Object PulseWindow+RECT
  if(-not [PulseWindow]::GetWindowRect($panel,[ref]$rect)){throw 'Could not read panel bounds'}
  $w=$rect.right-$rect.left;$h=$rect.bottom-$rect.top
  if($w -lt 30 -or $h -lt 90){throw "Unexpected panel dimensions: $w x $h"}
  $bitmap=[Drawing.Bitmap]::new($w,$h)
  $graphics=[Drawing.Graphics]::FromImage($bitmap)
  try {$graphics.CopyFromScreen($rect.left,$rect.top,0,0,$bitmap.Size);$bitmap.Save((Join-Path $evidence 'pulse-rail.png'),[Drawing.Imaging.ImageFormat]::Png)}finally{$graphics.Dispose();$bitmap.Dispose()}
  $glass=Nodes|Where-Object {$_.Current.Name -eq '半透明玻璃（实验性）' -and $_.Current.ClassName -eq 'ToggleSwitch'}|Select-Object -First 1
  if(!$glass){throw 'Experimental glass toggle missing'}
  $glassPattern=$glass.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
  if($glassPattern.Current.ToggleState -ne [Windows.Automation.ToggleState]::Off){throw 'Experimental glass must default to off'}
  $glassPattern.Toggle()
  $until=[DateTime]::UtcNow.AddSeconds(10)
  do {if((Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json).pulseGlass -eq $true){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
  if((Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json).pulseGlass -ne $true){throw 'Glass choice was not saved'}
  Start-Sleep -Seconds 1
  $bitmap=[Drawing.Bitmap]::new($w,$h);$graphics=[Drawing.Graphics]::FromImage($bitmap)
  try {$graphics.CopyFromScreen($rect.left,$rect.top,0,0,$bitmap.Size);$bitmap.Save((Join-Path $evidence 'pulse-glass.png'),[Drawing.Imaging.ImageFormat]::Png)}finally{$graphics.Dispose();$bitmap.Dispose()}
  if((Get-FileHash (Join-Path $evidence 'pulse-rail.png')).Hash -eq (Get-FileHash (Join-Path $evidence 'pulse-glass.png')).Hash){throw 'Glass toggle did not change the rendered rail'}
  # Put a real topmost desktop surface behind the panel and repaint it in two
  # known colours. The corner sample proves the surface is actually visible
  # behind the panel's transparent pixels; the fill samples then prove the
  # glass composites over it instead of drawing one fixed tint.
  $backdrop=[PulseWindow]::Backdrop([uint32]255,$rect)  # red: COLORREF 0x0000FF
  try {
    [PulseWindow]::RaiseAbove($panel)
    Start-Sleep -Seconds 1
    $bitmap=[Drawing.Bitmap]::new($w,$h);$graphics=[Drawing.Graphics]::FromImage($bitmap)
    try {
      $graphics.CopyFromScreen($rect.left,$rect.top,0,0,$bitmap.Size)
      $red=$bitmap.GetPixel([int]($w/2),[int]($h/2))
      $redEdge=$bitmap.GetPixel(2,$h-3)
      $bitmap.Save((Join-Path $evidence 'pulse-glass-red.png'),[Drawing.Imaging.ImageFormat]::Png)
    } finally {$graphics.Dispose();$bitmap.Dispose()}
    [PulseWindow]::Recolor($backdrop,[uint32]16711680)  # blue: COLORREF 0xFF0000
    Start-Sleep -Seconds 1
    $bitmap=[Drawing.Bitmap]::new($w,$h);$graphics=[Drawing.Graphics]::FromImage($bitmap)
    try {
      $graphics.CopyFromScreen($rect.left,$rect.top,0,0,$bitmap.Size)
      $blue=$bitmap.GetPixel([int]($w/2),[int]($h/2))
      $blueEdge=$bitmap.GetPixel(2,$h-3)
      $bitmap.Save((Join-Path $evidence 'pulse-glass-blue.png'),[Drawing.Imaging.ImageFormat]::Png)
    } finally {$graphics.Dispose();$bitmap.Dispose()}
    if(($redEdge.R-$blueEdge.R) -lt 60 -or ($blueEdge.B-$redEdge.B) -lt 60){throw "The backdrop surface was not visible behind the panel ($redEdge / $blueEdge)"}
    if(($red.R-$blue.R) -lt 25 -or ($blue.B-$red.B) -lt 25){throw "Glass did not transmit the changing background ($red / $blue)"}
  } finally {[PulseWindow]::DestroyWindow($backdrop) | Out-Null}
  $toggle=Nodes|Where-Object {$_.Current.Name -eq '空闲时自动收起' -and $_.Current.ClassName -eq 'ToggleSwitch'}|Select-Object -First 1
  if(!$toggle){throw 'Auto-collapse toggle missing'}
  # A red desktop surface behind the panel makes collapse and re-open
  # measurable: an expanded glass rail is a dark red, a collapsed one leaves
  # the probe pixel pure red.
  $railBackdrop=[PulseWindow]::Backdrop([uint32]255,$rect)
  try {
    [PulseWindow]::RaiseAbove($panel)
    Start-Sleep -Milliseconds 400
    $probeX=$rect.left+[Math]::Min(($w-2),[Math]::Max(12,[int]($w*0.35)))
    $probeY=$rect.top+[int]($h/2)
    [PulseWindow]::SetCursorPos(5,5) | Out-Null
    $toggle.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Seconds 4
    if($testApp.HasExited){throw 'App exited during collapse animation'}
    $bitmap=[Drawing.Bitmap]::new($w,$h);$graphics=[Drawing.Graphics]::FromImage($bitmap)
    try {$graphics.CopyFromScreen($rect.left,$rect.top,0,0,$bitmap.Size);$bitmap.Save((Join-Path $evidence 'pulse-collapsed.png'),[Drawing.Imaging.ImageFormat]::Png)}finally{$graphics.Dispose();$bitmap.Dispose()}
    $collapsedPixel=SamplePixel $probeX $probeY
    if($collapsedPixel.R -lt 235 -or $collapsedPixel.G -gt 20){throw "Rail did not visually collapse: probe pixel $collapsedPixel still covered"}
    $reopenedPixel=$null
    foreach($offset in 2,4,6,10,16) {
      [PulseWindow]::SetCursorPos(($rect.left+$offset),$probeY) | Out-Null
      Start-Sleep -Milliseconds 250
      [PulseWindow]::SetCursorPos(($rect.left+$offset+1),($probeY+1)) | Out-Null
      Start-Sleep -Seconds 1
      $reopenedPixel=SamplePixel $probeX $probeY
      if($reopenedPixel.R -lt 225 -or $reopenedPixel.G -gt 20){break}
      $reopenedPixel=$null
    }
    if(!$reopenedPixel){throw "Rail did not visually reopen: probe pixel $reopenedPixel"}
    $bitmap=[Drawing.Bitmap]::new($w,$h);$graphics=[Drawing.Graphics]::FromImage($bitmap)
    try {$graphics.CopyFromScreen($rect.left,$rect.top,0,0,$bitmap.Size);$bitmap.Save((Join-Path $evidence 'pulse-reopened.png'),[Drawing.Imaging.ImageFormat]::Png)}finally{$graphics.Dispose();$bitmap.Dispose()}
    if(($collapsedPixel.R-$reopenedPixel.R) -lt 20 -and ($reopenedPixel.G-$collapsedPixel.G) -lt 10){throw "Rail did not visually reopen ($collapsedPixel -> $reopenedPixel)"}
  } finally {[PulseWindow]::DestroyWindow($railBackdrop) | Out-Null}
  $until=[DateTime]::UtcNow.AddSeconds(15)
  do {
    [PulseWindow]::SetCursorPos(($rect.right-[int]($w/2)),($rect.top+[int]($h/2))) | Out-Null
    Start-Sleep -Milliseconds 250
    $card=[PulseWindow]::Find([uint32]$testApp.Id,'QuotaScopeFlyout')
    if($card -ne [IntPtr]::Zero){break}
    [PulseWindow]::SetCursorPos(($rect.right-[int]($w/2)+1),($rect.top+[int]($h/2))) | Out-Null
  } while([DateTime]::UtcNow -lt $until)
  if($card -eq [IntPtr]::Zero){throw 'Detail card did not open on hover'}
  Start-Sleep -Seconds 1
  $cardRect=New-Object PulseWindow+RECT
  if(-not [PulseWindow]::GetWindowRect($card,[ref]$cardRect)){throw 'Could not read card bounds'}
  $bitmap=[Drawing.Bitmap]::new(($cardRect.right-$cardRect.left),($cardRect.bottom-$cardRect.top));$graphics=[Drawing.Graphics]::FromImage($bitmap)
  try {$graphics.CopyFromScreen($cardRect.left,$cardRect.top,0,0,$bitmap.Size);$bitmap.Save((Join-Path $evidence 'pulse-card.png'),[Drawing.Imaging.ImageFormat]::Png)}finally{$graphics.Dispose();$bitmap.Dispose()}
  # The rail and flyout are visible together on the same monitor. A changing
  # solid surface under the flyout catches a second duplication request failing:
  # the opaque fallback cannot transmit these colours.
  $cardBackdrop=[PulseWindow]::Backdrop([uint32]255,$cardRect)
  try {
    [PulseWindow]::RaiseAbove($panel)
    [PulseWindow]::RaiseAbove($card)
    # Leave the pointer on the rail: the card stays open and the sample is not
    # confused with a closed/hidden flyout.
    [PulseWindow]::SetCursorPos(($rect.right-[int]($w/2)),($rect.top+[int]($h/2))) | Out-Null
    Start-Sleep -Seconds 2
    $cw=$cardRect.right-$cardRect.left; $ch=$cardRect.bottom-$cardRect.top
    $bitmap=[Drawing.Bitmap]::new($cw,$ch);$graphics=[Drawing.Graphics]::FromImage($bitmap)
    try {
      $graphics.CopyFromScreen($cardRect.left,$cardRect.top,0,0,$bitmap.Size)
      $cardRed=$bitmap.GetPixel([int]($cw*0.7),[int]($ch*0.75))
      $bitmap.Save((Join-Path $evidence 'pulse-card-red.png'),[Drawing.Imaging.ImageFormat]::Png)
    } finally {$graphics.Dispose();$bitmap.Dispose()}
    # A transparent flyout would show the pure red backdrop unchanged. The
    # 22% bubble layer must darken it, proving the card is actually drawn.
    if($cardRed.R -ge 240){throw "Detail card was not drawn over the red backdrop ($cardRed)"}
    # Move the card repeatedly while the 125 ms desktop capture throttle is
    # active. The red background should never flash to the dark fallback.
    for($step=0; $step -lt 16; $step++) {
      $dx=if($step % 2){2}else{0}
      [PulseWindow]::SetWindowPos($card,[IntPtr]::Zero,($cardRect.left+$dx),$cardRect.top,0,0,0x0015) | Out-Null
      Start-Sleep -Milliseconds 45
      if([PulseWindow]::Find([uint32]$testApp.Id,'QuotaScopeFlyout') -eq [IntPtr]::Zero){throw "Detail card disappeared during motion at step $step"}
      $frame=[Drawing.Bitmap]::new($cw,$ch);$g=[Drawing.Graphics]::FromImage($frame)
      try {
        $g.CopyFromScreen(($cardRect.left+$dx),$cardRect.top,0,0,$frame.Size)
        $sample=$frame.GetPixel([int]($cw*0.7),[int]($ch*0.75))
        if($sample.R -lt 120 -or $sample.R -le ($sample.B+60)){throw "Detail card flashed dark while moving at step $step ($sample)"}
      } finally {$g.Dispose();$frame.Dispose()}
    }
    [PulseWindow]::SetWindowPos($card,[IntPtr]::Zero,$cardRect.left,$cardRect.top,0,0,0x0015) | Out-Null
    [PulseWindow]::Recolor($cardBackdrop,[uint32]16711680)
    Start-Sleep -Seconds 1
    if([PulseWindow]::Find([uint32]$testApp.Id,'QuotaScopeFlyout') -eq [IntPtr]::Zero){throw 'Detail card disappeared before blue backdrop sample'}
    $bitmap=[Drawing.Bitmap]::new($cw,$ch);$graphics=[Drawing.Graphics]::FromImage($bitmap)
    try {
      $graphics.CopyFromScreen($cardRect.left,$cardRect.top,0,0,$bitmap.Size)
      $cardBlue=$bitmap.GetPixel([int]($cw*0.7),[int]($ch*0.75))
      $bitmap.Save((Join-Path $evidence 'pulse-card-blue.png'),[Drawing.Imaging.ImageFormat]::Png)
    } finally {$graphics.Dispose();$bitmap.Dispose()}
    if($cardBlue.B -ge 240){throw "Detail card was not drawn over the blue backdrop ($cardBlue)"}
    if(($cardRed.R-$cardBlue.R) -lt 25 -or ($cardBlue.B-$cardRed.B) -lt 25){throw "Detail card did not transmit the changing background ($cardRed / $cardBlue)"}
  } finally {[PulseWindow]::DestroyWindow($cardBackdrop) | Out-Null}
  Stop-Process -Id $testApp.Id -Force
  $testApp.WaitForExit()
  $testApp=[Diagnostics.Process]::Start($info)
  $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty,$testApp.Id)
  $until=[DateTime]::UtcNow.AddSeconds(30)
  do {$page=Nodes|Where-Object {$_.Current.Name -eq '通用' -and $_.Current.ClassName -like '*NavigationViewItem'}|Select-Object -First 1;if($page){break};Start-Sleep -Milliseconds 200}while([DateTime]::UtcNow -lt $until)
  if(!$page){throw 'General page missing after restart'}
  $page.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
  $until=[DateTime]::UtcNow.AddSeconds(20)
  do {$combo=Nodes|Where-Object {$_.Current.Name -eq '侧边栏外观' -and $_.Current.ControlType -eq [Windows.Automation.ControlType]::ComboBox}|Select-Object -First 1;if($combo){break};Start-Sleep -Milliseconds 200}while([DateTime]::UtcNow -lt $until)
  if(!$combo){throw 'Appearance picker missing after restart'}
  $selected=$combo.GetCurrentPattern([Windows.Automation.SelectionPattern]::Pattern).Current.GetSelection()
  if(@($selected|Where-Object {$_.Current.Name -eq 'Pulse'}).Count -ne 1){throw 'Pulse was not restored in the picker'}
  $glass=Nodes|Where-Object {$_.Current.Name -eq '半透明玻璃（实验性）' -and $_.Current.ClassName -eq 'ToggleSwitch'}|Select-Object -First 1
  if(!$glass -or $glass.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -ne [Windows.Automation.ToggleState]::On){throw 'Glass choice was not restored after restart'}
  $glass.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
  $until=[DateTime]::UtcNow.AddSeconds(10)
  do {if((Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json).pulseGlass -eq $false){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $until)
  if((Get-Content -LiteralPath $settingsPath -Raw|ConvertFrom-Json).pulseGlass -ne $false){throw 'Glass could not be switched off'}
  @{appearance='pulse';switches=@('pulse','windows','pulse');restored=$true;glassRestored=$true;glassDisabled=$true;glassTransmitsBackground=$true;cardGlassTransmitsBackground=$true;cardMoveNoDarkFlash=$true;panelWidth=$w;panelHeight=$h}|ConvertTo-Json|Set-Content (Join-Path $evidence 'pulse-appearance.json')
} finally {
  if($testApp -and -not $testApp.HasExited){Stop-Process -Id $testApp.Id -Force}
}
