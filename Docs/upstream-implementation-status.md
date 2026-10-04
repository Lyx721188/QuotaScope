# 上游功能移植进度

更新日期：2026-10-04。当前公开路线图见[下一轮计划](next-upstream-plan-2026-10-04.md)，依赖和验收标准见[实施任务](release-acceptance-2026-10-04.md)。

第二轮新增上游参考：Pulse 1.7.2 `b570dd7`。Windows 已接入 Codex、Claude Code、DeepSeek 当前官方状态、独立故障通知开关及持久化去重，验证与范围见[服务状态说明](providers/windows-service-status.md)。官网约 90 天历史条与公布的可用率已实现；缺损历史单独提示，缺少可用率保留未知。

当前状态与通知的 [3f96e89 Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37210111588) 已通过，四份 UI 结果使用同一 Release 程序。新增历史批次本地整仓 807 passed / 6 ignored、fmt、普通 Clippy（无新增诊断）、隔离失败 UI 和三家真实公开页面 UI 通过；历史批次 Release CI 待核对。

上一轮最终基线为 main `bd1f19d`：780 passed / 6 ignored，[Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37200397474) 成功，同一程序的三个隔离 UI 验收均通过。

原生会话发现已增加 64 层、10 万目录项、1 万 JSONL 和 8 MiB 估计路径容量上限，缺口每次重新核对。新增 7 项定向与整仓 773 passed / 6 ignored、fmt/普通 Clippy（无新增告警）通过；账户深层目录提示/修复与 40 万行（68,800,061 字节）的流式、续读、暖缓存、筛选及取消 Debug UI 通过。三个脚本使用相同程序；[34a454d 的 Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37199456283) 已通过，三份 UI 使用同一 Release 程序。Claude 原生整数/文件容量/原子回复状态及缓存 6 已实现，新增 7 项回归、整仓 780 passed / 6 ignored、fmt/普通 Clippy（无新增告警）、扩展 Token Debug UI 的 150 Token 保留/提示/修复/筛选/新缓存/重开通过；[bd1f19d 的 Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37200397474) 已通过。规则见 [Claude 本机计数](providers/windows-claude-local-counts.md)。缓存字段缺省的完整性仍需独立设计。规则见[原生会话发现](providers/windows-native-transcript-discovery.md)。

## main 新增能力（尚未发布）

| 阶段 | 状态与行为 | 验证边界 |
|---|---|---|
| N1 性能 | 价格索引及命中/未命中缓存；未变日志不重写解析缓存，整数纳秒时间戳；单个可取消聚合 worker，最多四个筛选结果 | 追加、改写、删除、重启、筛选、取消与关闭重开回归通过；保持价格优先级和未计价语义 |
| N2 DeepSeek | 实际费用携带币种；有界官网历史缓存、余额回退、异步浏览器导入/清除；凭据、续读和缓存按账户隔离 | 合成协议、HTTP、Windows DPAPI 与隔离 UI 通过；真实账户未验证，见[说明](providers/windows-deepseek-console.md) |
| N3 Codex | 保守比较设置/请求参数；有界、可取消 facts reader；主账户展开页显示时段、规则、分母、不可判断与缺口 | parser/worker/隔离 UI 通过；不证明服务端实际运行模型，见[说明](providers/windows-codex-signals.md) |
| N4a DSH | 按 magic 流式解压 Zstandard 与拼接帧；原始/解码字节、单行和窗口有界；损坏、缺计数、变动保留 partial | 纯解析、取消、隔离 UI、冻结样本独立复算通过；不代表全部历史完整，见[说明](providers/windows-dsh-local-usage.md) |
| N4b ZCode | 固定 SQLite 用量表；归一化输入含缓存、输出含 reasoning；WAL/索引/取消及内存缓存有界 | 18 项定向回归、冻结用量独立复算及隔离 Debug UI 通过；Release UI 也通过 [dcaa37c 的 CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37184221991)，见[说明](providers/windows-zcode-local-usage.md) |
| R1 自动化 | ZIP 许可证与哈希；相同 Release payload 的独立身份安装、托盘/设置生命周期、卸载；失败日志保留 | [ef9292d 的 Windows CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37171576671) 成功；[dcaa37c 的 CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37184221991) 另通过 DSH/ZCode Release Token 页；原 AppId 升级、真实账户、多 DPI 人工验收仍分别待完成 |

N1–N4a 的源码基线回归为 706 passed / 6 ignored；含 ZCode 的 dcaa37c 为 724 passed / 6 ignored，fmt、普通 Clippy、Release 和 JSON CLI 完成；已有 Clippy 告警仍存在。每次后续改动以对应 commit 的检查为准。

当前目录为 77 个 Windows 路由、54 个 Token 来源（35 个已知格式读取，含导出缓存；19 个标准导入）。ZCode 的 reader、缓存、独立复算和 Debug UI 已通过，原生支持已接入。标准导入计数与零值覆盖随 `53bd960` 通过 [Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37186053767)，统一读取预算及流式 JSON 数组随 `f08ee5a` 通过 [CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37188857492)，浏览器副本隔离及清理随 `3e95c23` 通过 [CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37190912782)。日期展开仅补最近 366 天、所有实际记录保留，整仓 744 passed / 6 ignored、隔离 Debug UI 通过；[日期改动 Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37191964998) 通过。固定头部签名另通过本地 core 670 passed / 5 ignored 及 [Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37192497068)。模型时序扫描新增上限与独立缺口提示，整仓 750 passed / 6 ignored；隔离账户 UI 通过；[该批 Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37194125169) 已通过。日期上界与计数有效性已通过 12 项定向、整仓 756 passed / 6 ignored 和扩展账户 Debug UI（未来事件排除、有效首字 2.00 秒、Token 保持）；[对应 Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37195452537) 已通过。Codex 原生累计校验/缺口缓存通过整仓 766 passed / 6 ignored、账户/维护/续读/筛选/取消 Debug UI；同 ZIP 的 Release 账户回归通过 [176cb24 的 CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37197866133)，三份结果使用相同 Release 程序哈希。真实账户与升级仍分别保留验收状态。

## 已发布的 1.3.0 / 1.3.1 基线

以下表格与验证记录保留相应发布阶段的范围；DSH 压缩等后续变化以上方 main 状态为准。

基线：Windows 1.2.4 `d9a1644`；Pulse `b396306`（1.7.0 发布后的 main）。
范围：保留 Claude 单文件回复合并；不新增跨文件回复去重。
发布：Windows 1.3.0（`windows-v1.3.0`）。
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
- 19 项来源的原生格式：CodeBuddy、WorkBuddy、Cherry Studio、Command Code、OpenCodeReview、Hermes、Goose、Zed、Kiro、Crush、Unsloth、Antigravity CLI、MiMo Code、Devin Desktop、Freebuff、Trae、Warp、MiniMax Code、GitHub Copilot。仅接受标准导入；缺少 Token 的来源保持未知。
- 1.3.0/1.3.1 的 DSH 仅读取纯文本 JSONL；main 已补入有界 Zstandard 解压，详见上方 N4a。
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

标准导入的计数、时间和覆盖规则见[格式说明](providers/windows-usage-imports.md)。

Claude 状态栏：将 Claude 的 statusLine.command 配置为 `"完整路径\quotascope.exe" --statusline`。应用不会自动覆盖现有 Claude 设置/状态栏命令。只保存 rate_limits 的额度百分比、重置时间和读取时刻。
`quotascope.exe --dashboard` 打开概览，--refresh 刷新，`--url quotascope://dashboard` 处理导航。URL 接受 settings/dashboard/refresh，并兼容报告中的 account 链接打开设置；不接受参数、凭据或命令。
CSV 脚本：[Export-QuotaScope.ps1](../windows/scripts/Export-QuotaScope.ps1)，指定 -Executable 与可选 -OutputPath；仅导出缓存额度，保留时间/来源，缺失不写成 0。
安装时可选择注册 quotascope://，卸载仅移除仍指向该安装路径的注册。

## 验证

- cargo test --workspace --locked：通过，631 项测试通过，5 项需特定环境的测试默认忽略；桌面生命周期另行运行通过。
- cargo build --release --workspace --locked：通过；生成 `windows/target/release/quotascope.exe`（约 8.3 MiB）；构建验证与安装验证分别记录。
- cargo fmt --all -- --check 与 CI 的 cargo clippy --workspace --all-targets --locked：通过。Clippy 有风格告警，严格 -D warnings 不是通过状态。
- settings_lifecycle --ignored：隔离空账户 profile，底部自由横排和动画开启；真实托盘左键概览、设置关闭重开、重复打开、第二次启动、隐藏退出通过，最终重跑 37.24 秒。
- 隔离 CLI profile：--json/CSV 导出保留 25% 样本与来源/时间，不改写缓存；两次 --statusline 更新额度并丢弃输入中的路径/会话标识。
- Inno Setup 临时占位 payload 语法编译通过；不等于当前应用已打包/安装。
- 上游 fixtures 来自 b396306，只证明解析，不能代替真实账号请求。

原始验证日志仅保留本地：`work/upstream-parity-validation-20261003/verification-{tests,clippy,lifecycle,cli,installer-syntax}.log`。该目录不纳入 Git。

## 排除

Claude Code 不添加跨文件回复去重。macOS 刘海、Liquid Glass、Sparkle 保持平台差异，Windows 使用已有原生机制。
