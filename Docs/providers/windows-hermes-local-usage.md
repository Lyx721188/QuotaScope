# Hermes 本机 Token 读取

移植参考：[Pulse 1.7.2 HermesReader.swift](https://github.com/qunqin24/Pulse/blob/b570dd77fa3dd9c7ce779812eedff1db8738aaf1/Sources/Pulse/Usage/Readers/HermesReader.swift)。

## 数据来源

开启本机 Token 读取后，Token 页可读取默认 `~/.hermes/state.db`、`HERMES_HOME` 指定根目录、该根的 `profiles/<名称>/state.db`，以及 Windows 的 `%LOCALAPPDATA%/hermes/state.db`。`HERMES_HOME` 替代默认 Hermes 根；标准导入仍可用。数据库中的 `sessions` 和可选 `session_model_usage` 只读取身份、模型、开始时间及计数白名单，不查询聊天、标题、工作区或费用。

两个读取步骤使用同一只读 SQLite 事务，能看到已提交 WAL；有按模型用量行的会话不会再叠加会话总计，明确的零值用量也会覆盖旧总计。不同 profile 独立累计，不猜测跨库会话是否重复；同一文件经多个路径发现只读一次。每条是累计会话／模型部署，不是逐请求事件，日期以会话开始时间分桶，不能解释成当天精确消耗。

## 计数与计价

当前 [Hermes 标准化计数](https://github.com/NousResearch/hermes-agent/blob/bd970b0588bb663dde5569b3e60eeb0e15541e21/agent/usage_pricing.py) 明确区分输入与缓存，[数据库写入器](https://github.com/NousResearch/hermes-agent/blob/bd970b0588bb663dde5569b3e60eeb0e15541e21/hermes_state_usage.py) 保存这些字段；旧数据库没有可靠的语义标记来证明历史 input 与缓存的包含关系。因此本次沿用 Pulse 的保守规则：

- 缓存读写均明确为零时，可信 input 可以归类为新输入。
- 缓存非零、缺失或无效时，只把可信 input 保留为未分类计数，缓存不再额外相加；保留已知模型，筛选和按小时图仍包含这些 Token。
- output 保留可信整数；reasoning 不额外叠加 output。缓存或 reasoning 非零／缺失的记录提示不完整。
- 缺值、负数、浮点、文本或超过单字段 1 万亿的计数不会补零；其余可信字段仍保留。没有模型归入 unknown，没有可用开始时间则不伪造日期。
- 只给已确认计价种类的部分使用公开模型价格。其余部分显示“缺少类型明细”及“未能计价”，不借用相邻模型或把未知金额声明为零。

这是可确认的本机记录子集，不是 Hermes 的账单或额度。标准导入应只补独立记录；软件不会猜测导入副本与原数据库是否重叠。

## 读取与缓存边界

只扫描命名 profile 的直接子目录，排除链接／重解析点；最多 16 个数据库、1000 个目录项、64 个候选路径。数据库最多 256 MiB、WAL 最多 128 MiB；每库最多 10 万会话行及 10 万模型用量行、4 秒，整个来源最多 20 秒。身份与桶分别受 8 MiB 估计容量约束。超限或无法读取时保留可信部分并提示缺口。

缓存只有内存中的聚合计数，最多 16 份且估计总容量 8 MiB，五分钟过期，不保留会话或 route 身份。SQLite 固定头、WAL 提交头、文件身份、精确时间戳与本机 UTC 偏移共同决定复用；追加、替换、删除和修复会重新读取。取消或扫描中变化不缓存半成品；关闭本机读取、清理缓存或离页会沿现有扫描控制器撤销任务和回收内存。

## 验证状态

13 项合成数据库回归与一项通用未分类聚合回归、整仓 821 passed / 6 ignored、fmt、普通 Clippy（233 项已有诊断，无新增）及隔离 Debug Token 页已通过。实际 portable Release CI 仍待独立核对。界面脚本使用空账户、独立 Hermes 目录与合成数据库，不代表真实账号或真实历史对账。

```powershell
# 在 windows/ 中，程序需带 WinUI 运行时资源
./quotascope-win/tests/hermes_ui_smoke.ps1 -Executable target/debug/quotascope.exe -Python python
```
