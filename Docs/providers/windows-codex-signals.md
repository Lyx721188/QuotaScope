# Windows Codex 模型与时序

2026-11-12 上游同步已撤下账户中的“本地异常线索”和“较弱模型”推断面板。旧 reader 保留用于既有数据和解析回归。

设置中启用本机 Token 读取，进入“账户”并展开 Codex，可查看最近 30 天按记录模型统计的 Token、占比与缓存命中，以及最近 24 小时的实测生成速度、首字延迟和活动提示缓存。三条可读的实测回复后才显示速度，读取缺口会单独提示。

本机主账户读取 `%USERPROFILE%\.codex\sessions` 与 `archived_sessions`。同一会话在两个目录出现时采用较长副本，避免归档导致重复计数。附加账户的日志归属需要服务商记录支持。详细口径见 [本机计数](windows-codex-local-counts.md)、[时序](windows-model-timings.md) 与 [会话发现](windows-native-transcript-discovery.md)。

从 `windows/` 运行隔离 UI 验证，脚本名称沿用历史路径，当前检查模型用量、首字延迟、损坏记录修复、目录发现缺口和关闭重开。

```powershell
pwsh -NoProfile -File quotascope-win/tests/codex_signals_ui_smoke.ps1 -Executable target/debug/quotascope.exe
```

脚本使用合成日志、独立 profile 和受限网络。完整同步范围见 [本轮记录](../upstream-sync-2026-11-12.md)。
