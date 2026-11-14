# Windows Pulse 液态玻璃调研

2026-11-14。原有 Pulse 轨道和详情气泡通过 Direct2D 绘制半透明深色形状，DirectComposition 将其叠到桌面；原实现没有背景模糊、折射或色散。DWM 背景被有意关闭，以保留带曲线和尖角的透明轮廓。

## 可参考的实现

- [WPF Liquid Glass Effect](https://github.com/dragosniamtu/WPF-Liquid-Glass-Effect)：实时桌面或窗口采集，加 shader 模糊和折射；当前版本是 WPF 和外部运行库示例，不可直接用于这里的 Rust/Direct2D 窗口。注意验证采集是否真的实时以及采集失败后的回退。
- [WindowsLiquidGlass](https://github.com/JetComX/WindowsLiquidGlass)：Apache-2.0，Win32/D3D11/HLSL 的模糊、折射、色散实现，可参考 shader。其当前 `ApplyBg` 使用图片或纯色，不能据此假设它已经捕获其他窗口；它还创建自己的 HWND swap chain，不能直接插入现有 DirectComposition 渲染管线。
- [LiquidGlassWinUI](https://github.com/luckyelysia/LiquidGlassWinUI)：MIT，WinUI 元素的背景画刷，有模糊、折射和高光。它依赖 Windows App SDK 2.2.0 特定内部符号的 IAT hook，且主要处理 WinUI 元素后面的内容；Pulse 轨道和气泡是独立 Win32/Direct2D 窗口，不宜直接套用。
- [DWM 系统背景](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwm_systembackdrop_type)：Windows 11 的 transient backdrop 是 Desktop Acrylic，可提供模糊，但作用于整个窗口边界，不提供折射。直接启用会破坏当前透明的曲线轮廓和气泡尾部。

## 对本项目的建议

优先验证独立原型：为轨道和气泡提供实时桌面背景纹理，再在现有 D3D11/DirectComposition 管线中进行局部模糊、基于形状边缘法线的折射和高光，最后以现有形状作为遮罩绘制。需处理多屏及 DPI、拖动、设备丢失、窗口被采集导致的反馈、受保护内容、GPU 开销和采集失败回退；避免引入系统内部 hook。尤其要验证轨道和气泡同时显示时的采集与合成。单纯增加透明度、白边或全窗口 Acrylic 都不能满足实时折射。

## 实验原型（GPU）

启用实验开关后，Desktop Duplication 按显示器共享采集接口并将桌面帧复制到 D3D11 纹理，供轨道和详情卡同时使用；GPU 像素着色器进行模糊、近似圆角矩形的边缘折射、轻微色散和高光，Direct2D 再以轨道或卡片的真实轮廓裁切。光学场借鉴 [WinUI.Composition.Hlsl](https://github.com/Millennium-Science-Technology-R-D-Inst/WinUI.Composition.Hlsl) 的边缘法线和色散思路，并非直接使用其 WinUI Composition 管线或完整材质。当前光学场不能精确跟随 Pulse 的弯曲轮廓和气泡尖角。shader 的源代码和预编译字节码位于 `windows/quotascope-win/shaders/`；修改后运行 `windows/scripts/compile-glass-shaders.ps1`，使用 Windows SDK 的 FXC 分别编译 `vs_main`（vs_4_0）和 `ps_main` 与 `ps_hdr`（ps_4_0），并一起提交源代码与字节码。

采集失败或 GPU 资源创建失败时回退到原有半透明着色。为防止自身画面被重复采集，启用时会把两个窗口从 Desktop Duplication 中排除；其他使用此 API 的录屏或截图软件也可能看不到这两个窗口。采集仍限于渲染设备所属显卡上的未旋转显示器，窗口必须完全位于单个显示器内；其他情况回退。采集尝试间隔至少 125 毫秒，帧格式变化时会重建纹理；支持 BGRA 和浮点 scRGB 输入，后者转换为 SDR 输出。仍需验证真实桌面上的多屏、不同 DPI、拖动、录屏影响和性能。

## 2026-11-14：背景闪烁与替代方案

用户反馈详情卡背景在黑色与透明之间跳变、气泡尖角附近出现重叠、整体偏暗。检查发现：窗口坐标在 125ms 的采集节流周期内变化时，旧实现直接返回空背景；气泡主体和尖角分别绘制造成叠色；文字没有按各自列宽裁切。现在窗口移动时立即重新采样最近的共享 GPU 桌面帧，采集本身仍节流；背景暂不可用时使用与正常材质接近的透明着色。气泡先对主体和尖角做几何并集再绘制一次，详情文字限制在各自的列和主体边界内。遮色由深色减弱到 22%，采样核扩展以改善模糊。

重新核对的候选方案：

- [liquidDX11](https://github.com/charlie-x/liquidImgui)：同样使用 DXGI Desktop Duplication 与 D3D11/HLSL，另使用多级 Dual Kawase 模糊；其示例是独立 ImGui 程序，不能直接作为现有窗口材质接入。多级模糊可以作为后续效果和性能对比，不应在没有测量的情况下宣称更快。
- [LiquidGlassForPC](https://github.com/Charles-2K/LiquidGlassForPC)：使用 Windows Graphics Capture 与 DirectComposition，并有 Dock 示例；替换采集方式仍需验证录屏行为、多显示器和两窗口同步。
- [Windows HostBackdropBrush](https://learn.microsoft.com/en-us/uwp/api/windows.ui.composition.compositor.createhostbackdropbrush) 由系统合成窗口后方内容，Windows 11 的 Win32 窗口可通过 [DWMWA_USE_HOSTBACKDROPBRUSH](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute) 启用；它可作为未来无采集的模糊备选，但不能把采样结果读回，且不能直接使用当前自定义折射 shader，不能当作等价替换。

当前继续使用现有 GPU 采集管线，优先消除确定的显示缺陷；未把外部示例作为依赖。系统背景方案和多级模糊仍需独立原型和真实桌面性能比较。
