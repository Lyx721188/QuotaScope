# Windows 设置崩溃：生命周期取证

2026-10-02，在 QuotaScope Windows 版 1.2.2、本机 `Microsoft.ui.xaml.dll 3.2.3.0` 和
`windows-reactor 0.100.0` 上验证。不能据此声称所有 WinUI 版本都禁止再次调用
`Application::Start`：最小文本窗口的两次启动实际通过。

## 已确认的触发链

旧代码每次收到托盘设置命令，都调用 `App::run_component::<SettingsApp>`。
Reactor 在内部调用 `Application::Start`；关闭最后一个 XAML 窗口会结束这次宿主的
消息循环，而 QuotaScope 的 Win32 托盘线程仍然运行。下次打开设置又启动一套宿主。
这不是普通的新建窗口操作。[Application.Start 的生命周期说明](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.application.start)
说明它直到应用关闭才返回。

原始崩溃事件发生于 2026-10-02 19:12:03 +08:00：`Microsoft.ui.xaml.dll`，
异常 `0xc000027b`，偏移 `0x3a9c5d`。随后以不加载任何 QuotaScope 账户、缓存、
后台请求、Direct2D 面板的独立程序，复现了相同模块、版本、异常码和偏移。

## 最小对照结果

| 操作 | 控件 | 实测 |
| --- | --- | --- |
| 关闭宿主，再启动第二次 | TextBlock | 通过 |
| 关闭宿主，再启动第二次 | ToggleSwitch | 通过 |
| 关闭宿主，再启动第二次 | PasswordBox | 通过 |
| 关闭宿主，再启动第二次 | Button | 通过 |
| 关闭宿主，再启动第二次 | NavigationView | 第二次启动崩溃 |
| 关闭宿主，再启动第二次 | ComboBox | 第二次启动崩溃 |
| 关闭宿主，再启动第二次 | 完整控件组合，不加载字体和 Mica | 第二次启动崩溃 |
| 一个宿主内连续创建、关闭三次窗口 | 完整控件组合 | 三次全部通过 |

字体与 Mica 加入后仍复现，但两者都不是必要触发条件。原始 WER 仅保留了事件与
Report.wer，没有转储；后续独立复现已取得转储，分析如下。`0xc000027b` 是外层
异常，不能单独充当内部资源错误的解释。

## 原生异常栈：2026-10-02 后续取证

临时修复提交 `0d73028` 已推送到 `codex/fix-settings-host-lifetime`，并建立
[草稿 PR #1](https://github.com/Lyx721188/QuotaScope/pull/1)。该提交的
[push 构建](https://github.com/Lyx721188/QuotaScope/actions/runs/37006650406) 与
[PR 构建](https://github.com/Lyx721188/QuotaScope/actions/runs/37006705342) 均通过，
包括对打包程序的设置生命周期回归。未创建新的发布标签。

用无账户数据的 NavigationView 独立程序复现两次宿主启动，并用 ProcDump 在进程
启动后附加，取得原有 `0xc000027b`。使用加载模块匹配的 Microsoft 公共 PDB，
读取 `STOWED_EXCEPTION_INFORMATION_V2`，而非只查看最后的 fail-fast 堆栈：

- 内部 `ResultCode` 为 `0x80004005`（`E_FAIL`）。
- 保存的原始栈顶为 `OptimizedStyle::InitializeWithDeferredSetters+0x58f`。
- 上游调用依次涉及 `OptimizedStyle::CreateWithDeferredSetters`、
  `CStyle::SetCustomWriterRuntimeData`、`CResourceDictionary::TryLoadDeferredResource`、
  `DirectUI::DefaultStyles::ResolveStyle`、`CControl::ApplyBuiltInStyle`。
- 未处理错误经过 `DirectUI::ErrorHelper::ProcessUnhandledError` 和
  `FailFastWithStowedExceptions`，最终终止进程。

在匹配二进制的失败分支之前设置断点，再次运行无账户的独立程序，取得失败瞬间
的完整内存转储，确认：

- 第一个未解析的样式 setter 是 `NavigationView.PaneToggleButtonStyle`。
- 所属 `XamlType` 对象非空，名称为 `NavigationView`，类型 token 为
  `0x20000425`；其 schema context 仍有强引用，不是类型指针为空或 weak_ptr 已过期。
- `GetDependencyProperty` 返回后，输出 `shared_ptr<XamlProperty>` 为空。
- 该类型的依赖属性解析缓存中，`PaneToggleButtonStyle` 对应的属性 token 和
  属性类型 token 均为 `0`。匹配反汇编显示这些空 token 可以经过
  `GetXamlProperty` 返回空属性，最终触发样式检查的 `E_FAIL`。

因此现在可以具体定位为“第二次宿主启动后，NavigationView 默认样式中的依赖属性
元数据解析失败”，不是账户请求或数据缓存过期导致。WinUI 当前
[公开源码中的对应属性解析检查](https://github.com/microsoft/microsoft-ui-xaml/blob/7b68d3e0b771a57d80098799406234efee479517/dxaml/xcp/components/style/OptimizedStyle.cpp#L166)
也在属性无法解析时返回 `E_FAIL`；此链接不是该发布二进制的精确构建源码。

使用 `Microsoft.UI.Xaml.Controls.dll` 的匹配公共 PDB 继续读取该属性的静态句柄，
发现更具体的生命周期不一致：

- `NavigationViewProperties::s_PaneToggleButtonStyleProperty` 仍为非空，所指
  `DependencyPropertyHandle` 有有效引用计数，底层属性名仍是 `PaneToggleButtonStyle`。
- 该属性的 declaring/target type index 都为 `0x3f0`，property index 为 `0x7c5`。
- 全局类型表中，旧 `0x3f0` 与新 `0x425` 都名为
  `Microsoft.UI.Xaml.Controls.NavigationView`；第二轮的 parser 使用后者。
- 当前 `m_customDPsByTypeAndNameCache` 为空。匹配二进制的
  `NavigationViewProperties::EnsureProperties` 仅在静态属性句柄为空时调用
  `InitializeDependencyProperty`，因此保留的非空句柄会跳过重新注册。

这将机制缩小为“控件保留的属性注册信息与第二轮宿主的元数据类型表不一致”，
不只是笼统的内部初始化失败，也没有证据指向已释放对象的非法地址访问。
公开源码中对应的
[静态属性注册条件](https://github.com/microsoft/microsoft-ui-xaml/blob/7b68d3e0b771a57d80098799406234efee479517/controls/dev/Generated/NavigationView.properties.cpp#L412)
与该二进制相符；
[属性名查找](https://github.com/microsoft/microsoft-ui-xaml/blob/7b68d3e0b771a57d80098799406234efee479517/dxaml/xcp/components/metadata/MetadataAPI.cpp#L1304)
依赖运行时注册表。

### 第一轮退出时的清理调用链

后续在匹配二进制上同时监视元数据销毁、`DeinitializeMUXC`、
`NavigationViewProperties::ClearProperties` 和第二轮样式失败。成功的实时记录为
`nav-reset-trace2.log`：第一轮正常返回，随后第二轮进入已定位的样式失败分支。
第一轮退出实际触发了以下原生调用链：

```text
DispatcherQueue.OnFrameworkShutdownStarting
  → XamlCoreNewShutdown::OnFrameworkShutdownStarting
  → WindowsXamlManager::XamlCore::Close
  → FrameworkApplication::ReleaseCurrent
  → MetadataResetter 析构
  → MetadataAPI::Reset
  → DynamicMetadataStorage::ResetInstance
```

在第一轮退出至第二轮样式失败之间，控件库的 `DeinitializeMUXC` 和
`NavigationViewProperties::ClearProperties` 断点均未命中；旧属性静态句柄则仍存在。
结合失败时的注册表内容，可以串起完整机制：XAML 清空当前注册缓存并推进类型
代际，控件库保留旧代的非空属性句柄，第二轮因此跳过必要的属性重新注册。
不是已经证明的悬空指针，而是两套仍有对象的注册信息不再一致。

Microsoft 公开源码中，
[MetadataResetter 析构](https://github.com/microsoft/microsoft-ui-xaml/blob/7b68d3e0b771a57d80098799406234efee479517/dxaml/xcp/dxaml/lib/MetadataResetter.cpp#L9)
调用 Reset；
[ResetInstance](https://github.com/microsoft/microsoft-ui-xaml/blob/7b68d3e0b771a57d80098799406234efee479517/dxaml/xcp/components/metadata/DynamicMetadataStorage/DynamicMetadataStorage.cpp#L48)
会使旧注册失效、清空当前缓存并保留代际边界。这些源码用于解释实现，具体发布
二进制的调用和对象状态仍以匹配 PDB、反汇编及转储为依据。

### 绕过 Reactor，并完全移除 Rust 的因果对照

先使用直接 WinUI ABI 的 Rust 程序，绕过 Reactor 的组件、协调器、任务调度及
窗口管理：文本两轮通过，NavigationView 在第二轮失败。然后单独编译 C++/WinRT
程序，不链接 Rust、Reactor、QuotaScope 数据和后台服务，获得以下结果：

| 独立 C++ 对照 | 实测 |
| --- | --- |
| TextBlock，宿主启动两轮，无额外清理 | 通过 |
| NavigationView，宿主启动两轮，无额外清理 | 第一轮正常返回，第二轮崩溃 |
| ComboBox，宿主启动两轮，无额外清理 | 第一轮正常返回，第二轮崩溃 |
| NavigationView，轮间额外清理，三轮启动，重复三组 | 9 次启动全部通过 |
| ComboBox，轮间额外清理，三轮启动 | 全部通过 |

C++ 失败事件也报告同一 DLL 版本、`0xc000027b` 和 `0x3a9c5d` 偏移。另取完整
转储并读取 stowed exception，内部仍为 `0x80004005`，原始栈顶仍为
`OptimizedStyle::InitializeWithDeferredSetters+0x58f`（模块 RVA `0x96986f`）。
因此不是把另一种 C++ 崩溃误当成原故障，Rust 投影和 Reactor 也不是必要原因。

这里的“额外清理”是在上一轮 Application、Window 引用释放后，用 GetProcAddress
调用控件 DLL 的 `DeinitializeMUXC`，然后才进入下一轮。直接 ABI 的 Rust 对照也
在相同干预下连续三轮通过，重复三组均成功。这证明补齐控件库清理能够消除当前
复现中的失败，形成了因果对照，而不仅是静态句柄与崩溃的相关性。

[控件库清理源码](https://github.com/microsoft/microsoft-ui-xaml/blob/7b68d3e0b771a57d80098799406234efee479517/controls/dev/dll/dllmain.cpp#L352)
明确用于 XAML 多次初始化之间的依赖属性清理；
[Microsoft 的 XAML Islands 示例](https://github.com/microsoft/microsoft-ui-xaml/blob/7b68d3e0b771a57d80098799406234efee479517/Samples/WinUICppIsland2SampleApp/WinUICppIsland2SampleApp/MainWindow.cpp#L492)
与其原生 hosting 测试也手动调用它。但它在
[导出定义中标为 PRIVATE](https://github.com/microsoft/microsoft-ui-xaml/blob/7b68d3e0b771a57d80098799406234efee479517/controls/dev/dll/Microsoft.UI.Xaml.Controls.def)，
不能据此把它当成稳定的生产公共 API。本 PR 不添加该私有调用。

### 责任与结论边界

可以确认当前原生 WinUI/XAML 控件组合在这条完整宿主退出、重新启动路径上没有
自动完成一致的清理；纯 C++ 同样受到影响。QuotaScope 旧设计把关闭设置窗口当成
结束整个 XAML 宿主，触发了这条路径；Reactor 原入口没有补齐控件库私有清理。
因此可定位为原生宿主重启过程中的清理缺口，同时需要修正应用的生命周期设计。

这不等于 Microsoft 已确认该行为为公开承诺支持路径上的产品缺陷，也不能证明
所有或最新 WinUI 版本都有相同问题。没有测试不同 runtime 版本，更没有证据说
新建普通 Window 本身会崩溃。应用侧能够保持宿主与托盘的生命周期一致，不需要
等待上游才能修复用户遇到的崩溃。

直接由调试器创建进程时，还观察到首次宿主关闭期间的 `0xc0000374`；这与原始
异常不同，不能用来替代上述结论。后续匹配转储采用进程启动后附加并设置
`_NO_DEBUG_HEAP=1`，恢复了第一轮正常关闭、第二轮原始 stowed exception 的路径。

取证文件保留在本地忽略目录：`windows/target/native-nav-attach-dump/`、
`nav-stowed-raw.log`、`nav-stowed-symbols.log`、`nav-style-disassembly.log`、
`nav-live-property4.log`、`nav-failed-property-objects.log`、
`nav-failed-property-cache.log`、`nav-property-resolution-native.log`、
`nav-controls-registration-state.log`、`nav-property-generation-mismatch.log`、
`nav-old-new-class-state.log`、`nav-registration-table.log`、`nav-reset-trace2.log`、
`raw-winui-results.json`、`raw-winui-cleanup-results.json`、`cpp-winui-results.json`、
`cpp-winui-cleanup-results.json`、`cpp-winui-stowed-raw.log`、
`cpp-winui-stowed-symbols.log`。
转储和完整日志不上传仓库。

## 修复与验证

WinUI 宿主保持到整个托盘应用退出；设置关闭时隐藏，再次打开或重复启动程序时
唤醒同一宿主。生命周期请求独立于账户状态 generation，空闲时也能处理重开和退出。
这纠正了应用宿主随单个设置窗口关闭而结束的生命周期设计；新建窗口本身仍可在
同一宿主内正常工作。当前选择复用设置窗口，不通过反复重启 XAML 来刷新界面。

1.2.3 的隐藏方案会保留设置窗口和控件，因此确有资源成本。保持宿主本身不要求保留
全部设置内容：后续可在同一宿主内释放设置窗口/控件，再按需创建，并单独保持
DispatcherQueue/XAML 宿主直到托盘退出。最小对照已证明同宿主内重建三轮控件窗口
可行，但生产实现、内存差额和长期回归仍需另做，不能把这个对照当成优化已完成。

后续当前工作树已实现较小范围的回收：保留同一个设置 HWND、宿主及 NavigationView
外壳，隐藏时移除 Content 页面的控件树，并释放统计快照。重新显示时在同宿主中
恢复常规页，不调用第二次 Application::Start。尚未结束的扫描通过 generation
丢弃旧结果；扫描运行标志独立于页面可见状态，避免快速重开产生重复扫描。
没有强制裁剪进程工作集，也不使用私有 WinUI 清理接口。此处的实现说明不代表
已量化所有账户配置下的内存节约或完成长时间资源稳定性验收。

常规测试 568 项通过。正式构建的首次打开前、关闭后各空闲 310 秒的进程回归通过，
还覆盖三轮托盘重开、重复启动、重复请求和隐藏状态退出。发布工作流对打包程序
执行短版生命周期回归，保留首次显示 20 秒以检查延迟的 WinUI 资源故障；
临时修复提交的云端 CI 已通过，详见上面的运行链接。

最小程序保留在 `windows/quotascope-win/tests/fixtures/host_lifecycle_probe.rs`，
独立于生产入口。`restart` 测两次宿主启动，`replace` 测同一宿主内三次新窗口。
`PROBE_CONTROLS` 可选 `nav`、`combo`、`toggle`、`password`、`button`、`all`；
`PROBE_NATIVE_CLOSE=1` 使用与用户关闭相同的 WM_CLOSE 路径。
`PROBE_FONTS`、`PROBE_MICA` 是可选对照，默认不启用。

独立 C++ 程序保留在
`windows/quotascope-win/tests/fixtures/host_lifecycle_probe.cpp`。它默认两轮启动，
每轮显示两秒后通过 WM_CLOSE 关闭，`PROBE_CONTROLS=text|nav|combo`，
`PROBE_ROUNDS=3` 可改变轮数；`PROBE_RESET_MUXC=1` 只启用上述私有清理实验。

该文件是手动诊断 fixture，不是 Cargo 自动执行的测试。本机使用 Windows SDK
10.0.26100.0 的 cppwinrt.exe 生成投影，用 MSVC 14.51、C++20 编译，链接
`windowsapp.lib` 和 `user32.lib`，嵌入 QuotaScope 构建生成的 app.manifest，并将
程序放在相同的自包含 runtime 目录运行。WinMD 输入取自与项目
Microsoft.WindowsAppSDK 2.4.0 依赖一致的 WinUI 2.3.6、Foundation 2.3.9、
InteractiveExperiences 2.1.6 包，并补上当前 WebView2 Core 1.0.4078.44 元数据，
Windows.winmd 作为 reference。使用别的 runtime 时必须重生成对应投影和清单，
不能把本次结果直接推广到不同版本。

本地构建脚本和 SDK 元数据保留在 `windows/target/debug-tools/`；SDK 生成文件、
工具二进制和转储不纳入仓库。新增 C++ fixture 不改变生产应用。

本次构建的详细日志与独立诊断可在工作树 `windows/target/` 中找到：
`crash-before.json`、`lifecycle-before.log`、`lifecycle-release-idle.log`、
`host-probe-only-*.log`、`host-probe-recreate-same-host.log`。
