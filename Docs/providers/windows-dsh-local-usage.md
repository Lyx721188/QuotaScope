# Windows DSH 本机 Token 读取

在设置启用本地 Token 读取，打开“Token 消耗”，来源选择 DeepSeek Harness。读取 `%USERPROFILE%\.dsh\sessions` 下的 `session*.jsonl`、`session*.zstd`；标准导入目录继续单独支持，不因扩展名猜测内容。

只有首四字节为 `28 B5 2F FD` 才使用 Zstandard。纯文本即使带 `.zstd` 后缀也按 JSONL 读取。压缩追加可包含多个拼接 frame；无需系统安装 zstd CLI 或 DLL。依赖通过 Cargo.lock 固定并静态链接，BSD 通知随包在 THIRD_PARTY_NOTICES.md 中提供。

本项目按既定边界只计 `assistant/message` 的已报告用量；助手片段、工具、压缩摘要及标题生成等额外事件不在此统计范围。因此这是本机助手记录的部分消耗，不是账户所有请求的账单。`reasoningTokens` 已包含在 output，不再次相加。seq 小于 session.seedLength 的分叉前缀排除；相同回复身份、时间、模型与计数的重放/副本只计一次。模型优先取回复 metadata，其次沿用请求 header，缺真实模型或时间时跳过并标记缺口。

input/output 主计数必须是已报告的非负整数。缺失、负数、非整数、过大或不可读的字段不会补成零；该回复跳过并保持“不完整”。格式中未出现的可选 cache 字段按默认零处理；显式 null/坏值则不当零。明确报告的全零用量与缺失不同。

流式读取保留解码器、64 KiB 行缓冲及一行数据，原始压缩字节和解码量分别最多 64 MiB，单行最多 4 MiB，解码 history window 最多 8 MiB。超过任一限制、损坏/截断、UTF-8/IO 失败或读取期间文件变化，来源保持“不完整”，前面可读的计数只作为部分记录。限制是解码输入、输出和工作区约束，不是整个进程 RSS 保证。

Token 页会显示所选来源的读取缺口，来源覆盖区也会标记；筛选缓存和排序保留这一状态。修复原始文件后手动刷新可重新读取并消除已解决的缺口。取消通过同一个 scan 控制器传递到压缩输入和逐行读取，不发布取消任务的半成品快照。

工作目录 `windows/` 的验证入口：

```powershell
cargo test -p quotascope-core zstd_stream --locked
cargo test -p quotascope-core additional_spend --locked
cargo test -p quotascope-core scan::tests --locked
cargo test -p quotascope-core spend_analysis --locked
cargo build -p quotascope-core --example dsh-read --locked
& C:/Python314/python.exe quotascope-win/tests/dsh_live_compare.py --reader target/debug/examples/dsh-read.exe
pwsh -NoProfile -File quotascope-win/tests/dsh_ui_smoke.ps1 -Executable target/debug/quotascope.exe
```

真实比对需要 Python 3.14 的标准库 `compression.zstd`。脚本最多选择 12 份近期、至少一分钟未变化的文件（可设置 1–32），冻结字节到经过路径检查的临时目录，用独立解码/计数与 Rust 比较；输出只有聚合数字、缺口类别和 corpus/hash，临时真实对话在退出时删除，不进入 Git 或证据日志。这个小样本比对不证明所有历史文件完整。具体检查结果、二进制与包哈希见 [执行计划](../next-upstream-plan-2026-10-04.md)。
