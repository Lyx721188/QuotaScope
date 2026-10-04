# Windows ZCode 本地用量格式

格式取证日期：2026-10-04。纯解析、有界只读 SQLite、原生来源接入、缓存及隔离 Debug Token 页已验收。ZCode 从标准导入改为原生读取；54 项目录现在为 35 项已知格式、19 项仅标准导入。最终 Release 包的 Token UI 已加入 CI，结果须按对应运行确认。

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

解析、数据库、缓存、路径和导入边界共 16 项定向测试通过。冻结用量字段的 Python 与 Rust 独立复算一致，包含模型/时间桶摘要；这不证明账户所有请求完整。比对脚本只投影用量字段，原始请求身份改为哈希，临时 SQLite 退出时删除；不复制对话表或凭据。真实聚合数字只保留本地忽略目录。

## 资源和缓存边界

- 固定数据库上限 256 MiB，WAL 128 MiB，最多 100,000 行，读取期限 4 秒，锁等待 100 毫秒。只接受原生时间索引的 `started_at`/BINARY 首列，不为缺失或改变的索引排序整库。
- 原始用量对象最多 4 KiB，SQLite 单行最多 64 KiB，页缓存约 1 MiB；Token 时间/模型桶使用按键及节点开销估算的 8 MiB 预算（不是实测堆内存字节），不保留请求身份、原始 JSON 或连接。
- 文件身份、长度、精确修改时间和少量头尾摘要共同识别数据库/WAL变化；空 WAL 不含帧，视作不存在。内存缓存 5 分钟过期，禁用读取或清理统计缓存会释放它。
- 标准导入仍支持；上限为 16 个根、8 层、10,000 个目录项、1,024 个文件，单文件 8 MiB、单行 4 MiB、总输入 64 MiB。超过边界标记不完整，重复数据库副本不累计。
- 取消、锁定失败、损坏、变化中或超限的读数不替换完整缓存。稳定的缺计数记录可缓存其 partial 状态，修复后重新读取。

隔离页面使用合成数据核对 120→170 的重试追加、缺计数回到 120 并提示缺口、修复恢复 170、未计价模型 50、同大小同时间戳替换到 270、删除空态与关闭重开。时间、模型、分组和排序筛选通过；这些合成数字不是账户账单。

```powershell
pwsh -File quotascope-win/tests/zcode_ui_smoke.ps1 -Executable target/debug/quotascope.exe -Python python
```

脚本用独立配置、用户目录和本地合成价格运行。Debug 采用独立进程身份；Release 验收放在 CI 独立桌面，使用 ZIP 内的原程序。不要在已有正式实例运行的桌面用 Release 脚本，正式实例的互斥身份会复用已有窗口。构建与 UI 验收顺序执行，避免正在运行的 EXE 阻止 Cargo 更新文件。
