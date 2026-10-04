# Windows 原生会话文件发现

Codex 与 Claude Code 的原生 JSONL 扫描先发现文件，再逐行读取。发现阶段限制目录深度 64、目录项 100,000、会话文件 10,000，以及估计路径存储 8 MiB。根目录记为第 0 层，第 64 层内的文件仍可读；第 65 层不进入。

目录项包括目录和其他扩展名文件，避免大量无关文件使遍历无限增长。路径预算按每个路径的系统字符串长度乘 2，再加 64 字节计入；这是保守的工程容量记账，不是整个进程的内存上限。超过文件、目录项或路径预算后停止发现。选择顺序取决于文件系统，不能保证保留的部分恰好是最新或最早历史。

缺失根目录表示没有本机记录；根类型错误、目录读取失败、发现后消失的子目录、超限或排除的子链接表示覆盖缺口。子链接不递归进入；明确配置的根路径仍按现有方式读取。已发现且成功读取的文件保留真实计数，账户/Token 页面沿用不完整提示。发现缺口每次重新核对，不写成单个完整文件永久损坏；移除过深目录或恢复可读文件后，手动刷新可清除解决的缺口。

这些上限只适用于 Codex/Claude 的原生会话发现，其他本机格式的发现规则需要按来源分别验收。它们也不限制单个正常会话为 8 MiB：大型 JSONL 仍逐行读取，Codex 追加仍核验整个旧前缀后续读。取消不提交未完成结果，筛选继续复用已有快照。

回归覆盖深度两侧、文件/目录项预算、路径预算恰好相等与不足一字节、空/缺失/错误类型根、缺口恢复、暖缓存和取消。隔离账户 UI 的普通深层目录样本检查浅层 3000 Token 保留、缺口可见以及删除样本后恢复。此样本没有创建 junction；静态链接判断与普通目录回归不等同于真实 junction 验收。大型流式、续读和取消另用对应 UI 脚本核对。

在 `windows/` 执行：

```powershell
cargo test --workspace --lib ledger::tests::native_discovery --locked -j 1
pwsh -NoProfile -File quotascope-win/tests/codex_signals_ui_smoke.ps1 -Executable target/debug/quotascope.exe -OutputDirectory target/codex-account-ui-validation
pwsh -NoProfile -File quotascope-win/tests/streaming_ui_benchmark.ps1 -Executable target/debug/quotascope.exe -Label after -VerifyUnchangedCache -VerifyFilters
pwsh -NoProfile -File quotascope-win/tests/spend_ui_smoke.ps1 -Executable target/debug/quotascope.exe
```

每次是否通过以对应提交和证据为准。以上为未发布改动，不表示真实账户已对照通过；Windows 1.3.1 尚未包含这组发现上限。
