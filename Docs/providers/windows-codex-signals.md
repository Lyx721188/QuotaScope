# Windows Codex 本地异常线索

此功能读取本机 Codex rollout，请求记录中的参数不是服务端实际运行模型的证明。不会发送模型探测，也不据此更改账户额度或 Token 账单。

在设置启用本地 Token 读取，进入“账户”，展开 Codex，查看“本地异常线索”。范围是最近 30/90 个本地自然日或全部记录；根目录是 `%USERPROFILE%\.codex\sessions` 和 `archived_sessions`，只读 `rollout-*.jsonl`。记录只归属本机主账户，不推断附加账户归属。

“刷新本地线索”重新读取文件；时段切换复用完整文件 facts。取消保留已有完整快照，离开账户页、收起 Codex 或关闭窗口释放该页面的结果和文件缓存。重新打开会重新读取。筛选页不会自动把不断追加的日志写进已显示的完整快照。

格点统计使用 `last_token_usage.reasoning_output_tokens`，只计正推理回复。分母要求至少 516；命中规则为 `reasoning % 518 == 516`。至少 20 个符合分母的回复、5 次命中并达到 5% 才显示“格点集中”；不足显示“样本不足”。完整度不足时显示可读取部分的计数，不给集中判断。“未出现集中”也不是模型正常或参数一致的保证。这是上游经验规则，算法标识 `codex-signals-v1`，没有服务端模型证据。

参数差异仅比较该轮 `task_started` 前已记录的 `thread_settings_applied`；中途设置变动到下一轮才参与判断。没有这类记录、旧于 0.144 的版本、缺少可比较字段、未知 effort 保持不可判断。模型变化、已知 effort 降低和同模型上下文窗口缩小分别列出，最多展示最近八条。helpers、auto-review、fork 回放及重复计数保守排除。没有记录差异不能说明不可判断轮次参数一致。

缓存只保留每日/模型计数与参数差异，不保存完整对话。按路径、size、精确修改时间和算法区分；删除、追加、改写失效。读取中途变动、IO/UTF-8 失败和取消均不存半份结果。最多缓存 1,024 个文件、8 MiB 保守结构记账预算；这不是进程 RSS 上限。聚合最多 1,024 个模型，窗口最多展示 64 个；目录最多检查十万个条目、深度 64，超出标记记录不完整。

本地复现入口（工作目录 `windows/`）：

```powershell
cargo test -p quotascope-core codex_signal --locked
cargo test -p quotascope-win codex_signal_state --locked
cargo run -p quotascope-core --example codex-signals-read --locked
pwsh -NoProfile -File quotascope-win/tests/codex_signals_ui_smoke.ps1 -Executable target/debug/quotascope.exe
```

示例只输出聚合数字，30 秒后取消；不会输出对话、路径、凭据或模型名。UI 脚本使用独立 profile 和合成日志，不修改现有安装版，不接触真实账户。公开功能状态见[开发计划](../next-upstream-plan-2026-10-04.md)，CI 与验收边界见[发布验收](../release-acceptance-2026-10-04.md)。原始日志和个人样本留在本地忽略目录；源码、Debug 窗口、Release 桌面和发布验收分别记录。
