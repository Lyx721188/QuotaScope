# Windows ZCode 本地用量格式

格式取证日期：2026-10-04。纯解析与有界只读 SQLite reader 已完成定向回归；尚未接入来源扫描、缓存和 UI。当前 ZCode 目录仍仅支持标准导入，目录数量不变。

已核对 ZCode 3.14.4 所带 CLI 的计数归一化与数据库写入流程，以及包含 `0010_usage_observability` 的 SQLite 格式。固定数据位置为 `%USERPROFILE%/.zcode/cli/db/db.sqlite`。只研究用量表和程序逻辑，不读取 `message`、`part`、配置凭据或会话正文；原始探查材料只保留在本地忽略目录。

## 计数规则

| 字段 | 语义与处理 |
|---|---|
| `model_usage.id` | 一次模型尝试的稳定身份；写入按主键更新，不能把每次更新重复累计 |
| `logical_request_id` / `attempt_index` | 一个逻辑请求可以有多次尝试；不同尝试分别保留，不按逻辑请求或助手消息去掉重试 |
| `query_source` | 包括主请求、子任务、工作流、压缩和标题等模型调用；范围是已记录的模型请求用量 |
| `started_at` | 请求开始时间，毫秒；用它划分自然日和时间桶，缺失或非法不猜日期 |
| `model_id` | 保留来源记录的模型标识；未知型号保持未计价，不借用邻近模型价格 |
| `input_tokens` | 已归一化的全部输入，包含缓存读/写；新输入 = input − cache read − cache creation |
| `output_tokens` / `reasoning_tokens` | output 是全部输出，reasoning 是其细分，不能再次相加 |
| `computed_total_tokens` | 归一化 input + output；须与拆分后的计数一致 |
| `provider_total_tokens` | 可选的提供方总计；有值时应与确定的计数一致，不以不一致值反推字段 |
| `raw_usage_json` | 写入的归一化用量对象；只检查计数字段是否真的存在、类型是否正确，避免把数据库默认 0 当成报告了 0 |
| `status` | completed/error/cancelled 均可能有已报告用量；有可信用量才计数，错误或取消不能一律归零 |

`turn_usage` 是聚合结果，不能再加到 `model_usage` 上。数据库主键已经折叠同一尝试的更新；对话分叉和回复副本也不应通过再次扫描会话来累计。

## 无法确定的记录

缺失计数字段、默认零但没有原始用量、负数/浮点/异常大数、缓存超过输入、reasoning 超过输出、总计冲突、未知状态或时间都会保留缺口。历史/自定义提供方若使用不同归一化方式，先标记不完整，不能仅凭输入总计等式推断缓存关系。

明确报告的全零用量与缺失不同。缺失模型保留未知状态；没有公开价格的真实模型计入 Token，金额保持未计价。SQLite 记录可能按客户端保留策略删除，这份本地历史不能代表账户全部请求或真实账单。

## 实现和验收顺序

1. 纯解析与合成数据库：含缓存、reasoning、零值、缺失、非法值、失败/取消、独立重试及主键更新，使用独立期望值核对。
2. 有界只读 SQLite：不迁移、不写数据库，处理 WAL 一致性、锁定、损坏、取消、读取中变化与资源上限。
3. 缓存：未变内容复用；追加、原位更新、替换、删除、修复及 WAL 变化失效；取消不发布半成品。
4. 冻结用量样本独立复算与隔离 Token 页验收后，才将来源从标准导入改为原生支持并更新目录说明。

工作目录 `windows/` 的检查入口：

```powershell
cargo test -p quotascope-core zcode_spend --locked
cargo build -p quotascope-core --example zcode-read --locked
python quotascope-win/tests/zcode_live_compare.py --reader target/debug/examples/zcode-read.exe --output target/zcode-compare.json
```

纯解析及只读数据库的 8 项定向测试通过。冻结用量字段的 Python 与 Rust 独立复算一致，包含模型/时间桶摘要；这不证明账户所有请求完整。比对脚本只投影用量字段，原始请求身份改为哈希，临时 SQLite 退出时删除；不复制对话表或凭据。真实聚合数字只保留本地忽略目录。
