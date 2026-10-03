# 上游功能移植进度

2026-10-04 后续跟进见 [执行计划与交接](next-upstream-plan-2026-10-04.md)：
主目录已同步 Windows 1.3.1 `db92d3e`，本地基线通过（644 passed、6 ignored，正式构建通过）；下一轮按性能、
DeepSeek 控制台、Codex 本地异常线索、原生来源顺序推进。
下表保留 1.3.0/1.3.1 的既有能力与验收边界，新改动的状态以执行计划的记录为准。

后续分支 N1a-1 已在本地验证：批量查价复用单份价格表的惰性索引与 model/vendor 命中、未命中缓存；
保留第一方/厂商优先级和未计价语义。尚未发布，原始测试与合成性能结果见执行计划。
N1a-2 已验证原始日志未变时解析缓存不重写；隔离进程重启后读取的真实缓存 mtime/哈希保持，
追加、重启续读、改写与删除回归通过；N2 整仓检查又发现浮点时间戳 JSON 精度漂移，已改用整数纳秒并通过回归。
N1b 已验证：Token 页的聚合、小时与模型列表移到单个可取消后台 worker，同一原始快照最多缓存四个筛选结果；
仅排序复用聚合，换筛选与关闭后拒绝旧完成结果。真实隔离窗口的筛选、缓存保持和取消/关闭/重开通过。
N1c 本地回归完成：655 passed / 6 ignored、fmt/普通 Clippy/Release/CLI、三轮流式 UI；
完整 Debug payload 的独立安装、托盘生命周期与卸载通过，Release 安装包构建成功。
Release 桌面与真实账户、多 DPI 等仍未验证，也没有替换用户安装版或发布。
N2a 历史表达已通过定向回归：实际费用携带币种，人民币不再走美元格式，缺少币种不显示金额，本机 API 估值保持。
N2b 控制台纯解析已通过 9 项合成测试，含 envelope、币种/缺失金额、时区、DST 和钱包；尚未读取控制台账户。
N2c-1 的固定 transport、401/403 分类、一次续读及 console 存储槽/KeyRing 账户隔离已通过定向检查；
N2c-2 的有界缓存、账户历史与余额 fallback 已完成定向验证，尚未调用真实浏览器/账号。
N2d 本地已验证：异步官网导入/清除、主/附加账户隔离、真实 Windows DPAPI 和合成 HTTP/WinUI；
679 passed / 6 ignored、fmt/普通 Clippy/Release 通过，完整 Debug 独立安装/托盘/卸载通过，Release 包构建成功。
手动刷新绕过官网历史缓存，余额每次取得本账户当前加密 token，避免续期后继续使用旧 KeyRing 会话。
N3a 本地纯解析已通过 9 项合成测试和普通 core Clippy；只比较 task_started 时已生效的用户设置，
旧版本、无可比较 applied 记录、helpers/reviewer、fork 回放和未知 effort 保守处理。
N3b 已通过 15 项 parser/reader 与 2 项 worker 状态测试；内存 facts 按路径/size/精确 mtime/算法版本复用，
最多 1,024 文件、8 MiB 保守记账预算，聚合最多 1,024 模型，超出时标记部分记录。
单个 worker 可取消，关闭/离页释放 reader 并拒绝旧结果。
N3c 已接入 Codex 主账户展开页：中文自然日时段、刷新/取消、规则版本、样本分母、不可判断与记录缺口，
把记录参数差异和格点启发式分开。696 passed / 6 ignored、fmt/普通 Clippy/Debug/Release 与隔离 UI 通过；
完整 Debug 独立安装/托盘/卸载通过，Release 包仅构建。本机 30/90 天只读 facts smoke 完成，未判断服务端模型。
配置见 [Windows Codex 本地线索](providers/windows-codex-signals.md)。
当前唯一下一项为 N4a DSH 实际 Zstandard；真实账户与 Release 桌面未验证，没有发布或升级安装版。

基线：Windows 1.2.4 `d9a1644`；Pulse `b396306`（1.7.0 发布后的 main）。
本轮要求：除第一项 Claude 跨文件回复去重外，其余尝试实现。
发布：Windows 1.3.0（`windows-v1.3.0`）。本机安装版没有替换。
Windows 1.3.1 将 Kiro 替换为专用 ACP 读取器，补齐严格解析、稳定池 ID、错误分类与有界子进程回收；
保留其余 1.3.0 路由和能力。Kiro 真实账号验证仍未完成，见 [配置与验证边界](providers/windows-kiro.md)。

## 已实现与验证边界

| 范围 | 当前结果 | 验证 |
|---|---|---|
| 单文件流式回复 | 同一回复合并各字段最大计数，保留首次时间桶 | 早期/最终/乱序快照、跨桶样本；不同文件仍分别累计 |
| 价值估算 | 5% 下限、按额度读数时刻截止、边界桶按重叠比例计入；无价格或不完整记录不推算 | 时间边界、计价缺失、周期重置回归 |
| 外部消耗保护 | 至少相隔两分钟的额度上涨缺少对应本机消耗时，暂停本周期价值估算 | 短间隔、账户、周期、无价格 Token、持久化回归；属于保守推断 |
| 模型与缓存详情 | 30 天模型占比/命中率、24 小时速度、真实 Codex 首字延迟、逐会话缓存列表 | 解析样本；缺失显示未知；设置关闭/离页取消读取 |
| Codex 缓存 | GPT-5.6 及后续型号显示官方最短 30 分钟复用期，不保证命中 | 型号/缓存计数回归；旧型号不套用期限 |
| OpenCode 官网 | Go 额度、跨设备请求账单、实际美元费用；官网会话与 API key 分别加密保存 | 分页去重、循环游标、不完整页面、认证失败样本；真实账号未验证 |
| OpenCode 2 / Kilo SQLite | 新旧消息表兼容、助手记录过滤、重复 ID 合并 | 临时只读数据库回归 |
| Token 来源与小时图 | 54 项目录；34 项已知格式读取（含导出缓存），20 项仅支持标准导入 | DSH 分叉/重放、导入 schema/ID、计数和筛选；不宣称 54 项全部原生支持 |
| 后台统计 | 默认关闭；单份 5 分钟快照、30 秒扫描期限；关闭选项取消释放；前台扫描取消后台扫描 | 取消/关闭/缓存回归；不保留完整对话文本 |
| 十个额度路由 | Kiro、Ollama Cloud、Grok Bot、Volcengine、Devin、Alibaba Token Plan、Gemini、JetBrains AI、Windsurf、Nous Portal | 上游额度 fixtures、独立火山签名向量、非法数据样本；真实账号未逐一验证 |
| Chromium localStorage | 有界只读 LevelDB；只读当前 manifest/指定网站及键；CRC/Snappy/版本/删除校验 | 日志、manifest、过期文件、跨域、损坏记录回归 |
| Claude 回退 | CLI OAuth 优先，状态栏缓存最多 15 分钟；显式导入 Desktop/浏览器会话后可读网站额度 | 只保存额度字段、缓存过期、多组织选择回归；App-Bound 加密不支持 |
| 附加账户 | API key/Cookie/会话 JSON 账户；Claude 可导入 OAuth JSON；移除和独立刷新 | 凭据隔离；不借用主账户历史/缓存，不复用移除的账户编号 |
| Windows 面板 | 底部停靠、自由竖排/横排、位置保存、长卡滚轮滚动 | 四边/多 DPI 几何、真实窗口生命周期；未逐项人工拖拽/滚动验收 |
| 托盘与快捷键 | 左键图形用量页、右键面板开关；可选 Ctrl+Shift+F10/F11/F12 | 真实托盘打开/重开/第二次启动/退出；快捷键占用冲突未提示 |
| 机器人与语言 | 概览页轻量眨眼伙伴；繁中/日/韩字典与语言选择 | 编译和真实窗口存活；Windows 专有文字部分回退英文，完整 BotMark 未移植 |
| 代理与连接信息 | 系统/禁用/手动 HTTP(S) 代理；共享请求、Cursor、Codex RPC、CLI/扩展进程；每账户来源/缓存/年龄 | 地址验证；诊断导出不含代理 URL/凭据；真实代理网络未验收 |
| 脚本与深链接 | --statusline、--dashboard、--refresh、--url、CSV 导出；安装器可选协议注册 | 隔离 profile 实际 CLI/CSV 验证；深链接拒绝凭据/参数/任意命令；Inno 语法编译；未实际安装注册 |

“已实现”表示代码及其连接存在，不表示所有服务商真实账户和 UI 操作已验收。

## 未完全移植

- 附加账户的交互式 OAuth 登录/续期；Codex 附加账户独立 app-server 登录。
- 20 项来源的原生格式：CodeBuddy、WorkBuddy、Cherry Studio、Command Code、OpenCodeReview、ZCode、Hermes、Goose、Zed、Kiro、Crush、Unsloth、Antigravity CLI、MiMo Code、Devin Desktop、Freebuff、Trae、Warp、MiniMax Code、GitHub Copilot。仅接受标准导入；缺少 Token 的来源保持未知。
- 真正压缩的 DSH Zstandard 文件未解压，会标记不完整；可读取纯文本 JSONL。
- Devin 仅本机缓存套餐，未覆盖全部在线/跨账户路线；JetBrains 不推算 ISO duration 周期；Alibaba 不展开任意字符串嵌套的 JSON 包装。
- 完整 BotMark 动画、所有 Windows 专有文字的繁中/日/韩翻译。
- App-Bound 解密、Firefox localStorage。缺口保持未登录/未知，不伪造数据。

## 使用

账户设置展开 Claude/Codex 可看模型与逐会话缓存；本机读取仍需打开 Token 消耗开关。
新增账户：在凭据框粘贴另一账户凭据，点“添加账户”；概览页可独立刷新/移除。不要用主账户的“保存”按钮来添加账户。
OpenCode 官网会话在 OpenCode 设置中导入，API key 独立保存；Windsurf 在其设置中导入。

标准导入放在 `%APPDATA%\QuotaScope\UsageImports\<source-id>\*.jsonl`；source 与目录 ID 一致，id 稳定唯一，同 ID 后续记录覆盖早期快照：

```json
{"schema":"quotascope.usage.v1","source":"zed","id":"request-1","timestamp":1790850000000,"model":"unknown","usage":{"inputTokens":10,"outputTokens":7,"cacheReadTokens":0,"cacheWriteTokens":0}}
```

Claude 状态栏：将 Claude 的 statusLine.command 配置为 `"完整路径\quotascope.exe" --statusline`。应用不会自动覆盖现有 Claude 设置/状态栏命令。只保存 rate_limits 的额度百分比、重置时间和读取时刻。
`quotascope.exe --dashboard` 打开概览，--refresh 刷新，`--url quotascope://dashboard` 处理导航。URL 接受 settings/dashboard/refresh，并兼容报告中的 account 链接打开设置；不接受参数、凭据或命令。
CSV 脚本：[Export-QuotaScope.ps1](../windows/scripts/Export-QuotaScope.ps1)，指定 -Executable 与可选 -OutputPath；仅导出缓存额度，保留时间/来源，缺失不写成 0。
下次安装可选择注册 quotascope://，卸载仅移除仍指向该安装路径的注册；本轮未改当前注册表。

## 验证

- cargo test --workspace --locked：通过，631 项测试通过，5 项需特定环境的测试默认忽略；桌面生命周期另行运行通过。
- cargo build --release --workspace --locked：通过；生成 `windows/target/release/quotascope.exe`（约 8.3 MiB），没有安装到本机。
- cargo fmt --all -- --check 与 CI 的 cargo clippy --workspace --all-targets --locked：通过。Clippy 有风格告警，严格 -D warnings 不是通过状态。
- settings_lifecycle --ignored：隔离空账户 profile，底部自由横排和动画开启；真实托盘左键概览、设置关闭重开、重复打开、第二次启动、隐藏退出通过，最终重跑 37.24 秒。
- 隔离 CLI profile：--json/CSV 导出保留 25% 样本与来源/时间，不改写缓存；两次 --statusline 更新额度并丢弃输入中的路径/会话标识。
- Inno Setup 临时占位 payload 语法编译通过；不等于当前应用已打包/安装。
- 上游 fixtures 来自 b396306，只证明解析，不能代替真实账号请求。

本机验证日志：`work/upstream-parity-validation-20261003/verification-{tests,clippy,lifecycle,cli,installer-syntax}.log`。该目录不纳入 Git。

## 排除

Claude Code 不添加跨文件回复去重。macOS 刘海、Liquid Glass、Sparkle 保持平台差异，Windows 使用已有原生机制。
