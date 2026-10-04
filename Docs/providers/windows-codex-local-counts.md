# Windows Codex 本机累计计数

本机 Token 来自日志中的 `total_token_usage` 累计快照，不累加 `last_token_usage`。缓存读取包含在 input 内；reasoning 不再次加入 output。账户额度、服务商账单、其他设备用量与本机记录分别显示。

input/output 必须是明确的 `0..=1,000,000,000,000` JSON 整数；上限是工程读取限制，不是服务商允许的 Token 数量。缓存字段沿用现有兼容规则，缺省为 0，显式 null、字符串、浮点、负数或超限均无效；cache read 必须不超过 input，其增量也不能超过 input 增量。[上游协议](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/protocol.rs) 使用整数计数，`info: null` 可以只是额度更新，正常跳过。现代协议会提供 cached input；兼容记录的字段缺省不能证明实际缓存用量为零。

非法计数和累计倒退不会更新最后有效基线，已有可确认用量保留并标记不完整。例如累计 1100，随后倒退，再恢复到同一个 1100 时，结果保持 1100；不会再次计入 150。真正重置后的缺失增量不推测。缺模型或时间的增量也不移交给后续已知模型/时间，只保留可归属的部分。

缺口随文件缓存保存，JSON 重载及后续追加不会把旧缺口变成完整记录；修复或替换原文件后重读才能清除。Codex 使用 `ledger-5-codex.json` 和新版续读状态，旧累计缓存重新构建；新旧可重建缓存都纳入磁盘预算与清理。账户“刷新”在后台使内存账本失效，再核对文件变化；展开账户可复用正常缓存。

合成回归覆盖异常整数、缺字段、缓存矛盾、倒退后恢复、未知归属、零值、额度空更新、缓存重载/追加/修复及旧续读状态。隔离账户 UI 检查计数缺口、手动修复、Token 保持和时序提示的独立性。Release CI 通过实际 ZIP 内的程序执行账户回归，结果与 DSH/ZCode 合成结果一起保存；每项是否通过以对应提交 CI 为准。

在 `windows/` 执行：

```powershell
cargo test --workspace --lib ledger::tests --locked -j 1
pwsh -NoProfile -File quotascope-win/tests/codex_signals_ui_smoke.ps1 -Executable target/debug/quotascope.exe -OutputDirectory target/codex-account-ui-validation
```

这些是未发布改动和读取规则，不表示真实账户已对照通过。Windows 1.3.1 尚未包含本次计数校验与缺口修复。
