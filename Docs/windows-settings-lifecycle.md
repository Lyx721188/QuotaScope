# Windows 设置崩溃：生命周期取证

2026-10-02，在 Windows 1.2.2、本机 `Microsoft.ui.xaml.dll 3.2.3.0` 和
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

匹配二进制的反汇编显示，该失败分支在解析样式 setter 的属性时，发现类型指针
或返回的依赖属性指针为空，转入 `OnFailure<82>` 并返回 `E_FAIL`。因此现在可以
具体定位为“第二次宿主启动后，复杂控件的默认样式属性解析失败”，不是账户请求
或数据缓存过期导致。WinUI 当前
[公开源码中的对应属性解析检查](https://github.com/microsoft/microsoft-ui-xaml/blob/7b68d3e0b771a57d80098799406234efee479517/dxaml/xcp/components/style/OptimizedStyle.cpp#L166)
也在属性无法解析时返回 `E_FAIL`；此链接不是该发布二进制的精确构建源码。

还不能把这进一步解释为已证实的“某个全局缓存悬空”：目前未取得失败瞬间的
具体类型、属性名及其对象生命周期证据，也未完成绕过 Reactor 的纯 WinUI 对照。
失败发生在 WinUI 内部，与宿主重启相关；原生库、封装的清理策略和调用方式之间的
责任仍需进一步区分。

直接由调试器创建进程时，还观察到首次宿主关闭期间的 `0xc0000374`；这与原始
异常不同，不能用来替代上述结论。后续匹配转储采用进程启动后附加并设置
`_NO_DEBUG_HEAP=1`，恢复了第一轮正常关闭、第二轮原始 stowed exception 的路径。

取证文件保留在本地忽略目录：`windows/target/native-nav-attach-dump/`、
`nav-stowed-raw.log`、`nav-stowed-symbols.log`、`nav-style-disassembly.log`。
转储和完整日志不上传仓库。

## 修复与验证

WinUI 宿主保持到整个托盘应用退出；设置关闭时隐藏，再次打开或重复启动程序时
唤醒同一宿主。生命周期请求独立于账户状态 generation，空闲时也能处理重开和退出。
这纠正了应用宿主随单个设置窗口关闭而结束的生命周期设计；新建窗口本身仍可在
同一宿主内正常工作。当前选择复用设置窗口，不通过反复重启 XAML 来刷新界面。

常规测试 568 项通过。正式构建的首次打开前、关闭后各空闲 310 秒的进程回归通过，
还覆盖三轮托盘重开、重复启动、重复请求和隐藏状态退出。发布工作流对打包程序
执行短版生命周期回归，保留首次显示 20 秒以检查延迟的 WinUI 资源故障；
临时修复提交的云端 CI 已通过，详见上面的运行链接。

最小程序保留在 `windows/quotascope-win/tests/fixtures/host_lifecycle_probe.rs`，
独立于生产入口。`restart` 测两次宿主启动，`replace` 测同一宿主内三次新窗口。
`PROBE_CONTROLS` 可选 `nav`、`combo`、`toggle`、`password`、`button`、`all`；
`PROBE_NATIVE_CLOSE=1` 使用与用户关闭相同的 WM_CLOSE 路径。
`PROBE_FONTS`、`PROBE_MICA` 是可选对照，默认不启用。

本次构建的详细日志与独立诊断可在工作树 `windows/target/` 中找到：
`crash-before.json`、`lifecycle-before.log`、`lifecycle-release-idle.log`、
`host-probe-only-*.log`、`host-probe-recreate-same-host.log`。
