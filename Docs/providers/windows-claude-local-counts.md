# Windows Claude Code 本机计数

Claude 的 input、cache creation、cache read 是独立输入类别，三者相加才是总输入；output 单独计入。流式 usage 是累计快照，同一文件同一回复保留各字段最大值及首次时间桶，不新增跨文件去重。参考[官方缓存计数](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)和[流式累计语义](https://platform.claude.com/docs/en/build-with-claude/streaming)。

input/output 要求显式 JSON 整数；四个字段各接受 `0..=1,000,000,000,000`。缓存字段缺省沿用兼容规则为 0，显式 null、字符串、小数、负数或超限均无效；兼容缺省不能证明真实缓存用量为零。非法记录保留此前可确认用量，并使文件计数不完整；不会将小数截断或负数钳制为有效计数。

每个文件已确认的四类总量也分别限制为一万亿，聚合使用 checked 加法。超限快照不提交回复最大值或桶增量，后续合法快照仍可读。这是软件读取容量，不是服务商额度；配合[一万文件发现上限](windows-native-transcript-discovery.md)，避免该原生来源的整数汇总溢出。大型正常 JSONL 仍逐行读取；这不保证无限独立回复或模型的全进程内存上限。

Claude 解析缓存改为 `ledger-6-claudeCode.json`，旧缓存重建；新旧版本均计入磁盘预算和清理，Codex 仍使用版本 5。缺口随文件缓存保存，暖缓存不清除；修复原文件后手动刷新可恢复。派生 Token 快照按原有刷新/过期规则更新。

合成回归覆盖极端整数、必填缺失、显式无效值、同 ID 恢复、独立缓存输入、零值、合成错误、容量原子更新、缓存重载/修复及旧新缓存预算。Token UI 检查其他来源筛选不显示 Claude 缺口、Claude 保留 150 Token、修复刷新清除提示以及写入版本 6；对应 CI 使用 ZIP 中的 Release 程序重跑该脚本。

在 `windows/` 执行 `cargo test --workspace --lib ledger::tests::claude_native --locked -j 1`；UI 入口为 `quotascope-win/tests/zcode_ui_smoke.ps1`。每次是否通过以对应提交证据为准。这些是未发布改动，不表示真实账户已对照通过；Windows 1.3.1 尚未包含本次规则。
