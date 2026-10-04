# Windows 标准用量导入

main 支持 `quotascope.usage.v1`，供尚无原生读取器的来源或独立导出记录使用。文件放在 `%APPDATA%\QuotaScope\UsageImports\<source-id>\`，使用 `.jsonl`（每行一个对象）；也接受 `.json` 对象或对象数组。来源 ID 见 [移植状态与导入入口](../upstream-implementation-status.md)。应用仍需启用本机 Token 读取。

```json
{"schema":"quotascope.usage.v1","source":"zed","id":"request-1","timestamp":"2026-10-01T14:00:00+08:00","model":"example-model","usage":{"inputTokens":10,"outputTokens":7,"cacheReadTokens":5,"cacheWriteTokens":3,"reasoningTokens":4,"totalTokens":25}}
```

该合成例子共 25 Token：10 个未命中缓存的输入、5 个缓存读取、3 个缓存写入、7 个输出。输出已包含 4 个 reasoning，不能再次相加。若上游 input 已含缓存，应先减去缓存部分再导入。

| 字段 | 规则 |
|---|---|
| `schema` / `source` | 固定 schema；source 必须与当前来源 ID 完全一致 |
| `id` | 稳定的请求或回复身份，非空白字符串，最多 1024 UTF-8 字节；不能每次导出随机生成 |
| `timestamp` | 正整数 Unix 毫秒，或带时间与时区的 RFC3339 字符串（最多 128 字节）；按本机时区分桶 |
| `model` | 非空白字符串，最多 256 UTF-8 字节；缺失或非法时保留计数，显示 unknown/未计价并标记记录不完整 |
| `inputTokens` / `outputTokens` | 必填 JSON 整数，范围 0–1,000,000,000,000；input 表示新鲜输入，output 包含 reasoning |
| `cacheReadTokens` / `cacheWriteTokens` | 可省略，省略表示无单列缓存计数；出现时必须是相同范围的整数 |
| `reasoningTokens` | 可省略；出现时是 0–outputTokens 的整数，只用于一致性校验 |
| `totalTokens` | 可省略；出现时必须是四种计数之和：input + output + cacheRead + cacheWrite |

不接受数字字符串、小数、布尔值、显式 null、负数或不一致的总数。缺失 input/output 不补零。日期字符串 `2026-10-01`、不带时区的时间、浮点毫秒和无效日期均不用于小时统计。无效记录会被跳过，来源保留“不完整”提示；其他有效记录仍可统计。

同来源、同 ID 的后续可读记录覆盖旧快照，包含明确的全零记录；模型或时段变化时旧桶也随之移除。这个顺序由扫描顺序决定：文件路径按排序读取、同一文件按行读取。建议在单个文件追加快照，避免多个文件包含相互冲突的版本。导入身份与原生记录身份分别处理；不要把已由原生读取器统计的同一批请求再次导入，否则可能重复计数。

缺少公开价格的模型仍显示 Token 和“未计价”，不套用邻近模型价格。导入校验和估算不能证明上游账单完整，也不能覆盖其他设备或已删除的历史。

回归覆盖 malformed counters、缺失时间/身份、同 ID 重复与零值覆盖、reasoning 包含关系，以及文件读取后的 partial/未计价传播。在 `windows/` 执行：

```powershell
cargo test -p quotascope-core additional_spend::tests --locked -j 1
```

本说明对应 main 的未发布改动；Windows 1.3.1 尚未包含本轮严格校验修复。
