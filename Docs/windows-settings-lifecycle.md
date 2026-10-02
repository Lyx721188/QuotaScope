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

字体与 Mica 加入后仍复现，但两者都不是必要触发条件。以上能把问题定位到当前
WinUI 运行时的复杂控件跨宿主重启路径，不能据此断言某个内部缓存、悬空指针或
具体 HRESULT 是原因：原始 WER 仅保留了事件与 Report.wer，没有可解析的转储。
`0xc000027b` 是外层异常，不能充当内部资源错误的解释。

## 修复与验证

WinUI 宿主保持到整个托盘应用退出；设置关闭时隐藏，再次打开或重复启动程序时
唤醒同一宿主。生命周期请求独立于账户状态 generation，空闲时也能处理重开和退出。
这纠正了应用宿主随单个设置窗口关闭而结束的生命周期设计；新建窗口本身仍可在
同一宿主内正常工作。当前选择复用设置窗口，不通过反复重启 XAML 来刷新界面。

常规测试 568 项通过。正式构建的首次打开前、关闭后各空闲 310 秒的进程回归通过，
还覆盖三轮托盘重开、重复启动、重复请求和隐藏状态退出。发布工作流对打包程序
执行短版生命周期回归，保留首次显示 20 秒以检查延迟的 WinUI 资源故障；云端 CI
本次尚未运行。

最小程序保留在 `windows/quotascope-win/tests/fixtures/host_lifecycle_probe.rs`，
独立于生产入口。`restart` 测两次宿主启动，`replace` 测同一宿主内三次新窗口。
`PROBE_CONTROLS` 可选 `nav`、`combo`、`toggle`、`password`、`button`、`all`；
`PROBE_NATIVE_CLOSE=1` 使用与用户关闭相同的 WM_CLOSE 路径。
`PROBE_FONTS`、`PROBE_MICA` 是可选对照，默认不启用。

本次构建的详细日志与独立诊断可在工作树 `windows/target/` 中找到：
`crash-before.json`、`lifecycle-before.log`、`lifecycle-release-idle.log`、
`host-probe-only-*.log`、`host-probe-recreate-same-host.log`。
