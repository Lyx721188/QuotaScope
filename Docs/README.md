# Docs

这里保留 Windows 版本仍在使用的服务商资料、接口说明和行为约定。应用源码和构建
入口统一位于 [`windows/`](../windows/)；仓库不再包含 macOS/Swift 实现。

## 开始阅读

| 文档 | 内容 |
|---|---|
| [json-output.md](json-output.md) | `quotascope.exe --json` 的状态栏输出约定 |
| [providers/README.md](providers/README.md) | 各服务商的读取通道、鉴权和本地数据来源 |
| [providers/windows-browser-cookie-read.md](providers/windows-browser-cookie-read.md) | 浏览器只读读取、临时副本隔离与支持边界 |
| [providers/windows-ledger-calendar.md](providers/windows-ledger-calendar.md) | 日期展开上限、稀疏历史与图表日历位置 |
| [providers/windows-model-timings.md](providers/windows-model-timings.md) | 模型速度扫描上限、时序缺口与计数的区分 |
| [providers/windows-codex-local-counts.md](providers/windows-codex-local-counts.md) | Codex 累计计数校验、倒退基线、缺口与刷新 |
| [providers/windows-claude-local-counts.md](providers/windows-claude-local-counts.md) | Claude 整数校验、回复最大值、文件容量与缺口缓存 |
| [providers/windows-native-transcript-discovery.md](providers/windows-native-transcript-discovery.md) | Codex/Claude 会话发现上限、缺口与大型文件边界 |
| [release-acceptance-2026-10-04.md](release-acceptance-2026-10-04.md) | 当前发布验收、ZCode 原生读取和后续账户验证任务卡 |

用户入口是仓库根目录的 [`README.md`](../README.md)。开发、测试和发布以
[`windows/README.md`](../windows/README.md) 及 GitHub Actions 为准。
