# 发布前验收与下一轮实施任务

日期：2026-10-04（Asia/Shanghai）。功能基线：`3c69d2f`，包含 N1 性能、N2 DeepSeek 官网历史、N3 Codex 本地线索及 N4a DSH 解压；R1 自动化与文档基线为 `ef9292d`。

本页承接[公开开发计划](next-upstream-plan-2026-10-04.md)，将后续工作拆成可独立验收的任务。R1 自动化与文档已完成，N4b 的原生读取、缓存、冻结样本和 Debug UI 已通过；R1d 的 Release Token 页也通过 dcaa37c 的 CI。标准导入的计数与零值覆盖修复已随 `53bd960` 通过整仓、隔离 UI 与 Release CI；统一导入资源上限随 `f08ee5a` 通过本地检查及 [Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37188857492)。浏览器回退副本随 `3e95c23` 通过 737 项本地测试及 [Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37190912782)。账本空白日期现仅补齐最新 366 天，所有实际记录保留，日期规划/输出支持取消；整仓 744 passed / 6 ignored、隔离 Debug UI 通过。该批 [Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37191964998) 已通过。ZCode 缓存签名已限制到主库/WAL固定头部，本地 core 670 passed / 5 ignored、fmt/普通 Clippy 通过；该批 [Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37192497068) 已通过。模型时序的有界遍历/读取与独立缺口提示已实现，整仓 750 passed / 6 ignored、隔离账户 UI 通过；该批 [Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37194125169) 已通过。日期上界与异常输出计数已通过 12 项定向、整仓 756 passed / 6 ignored 和扩展账户 Debug UI；未来首字事件排除、有效首字 2.00 秒、Token 保持及取消/重开通过。[对应 Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37195452537) 已通过。Codex 原生计数校验与缓存缺口通过整仓 766 passed / 6 ignored、账户/维护/续读/筛选/取消 Debug UI；同 ZIP 的 Release 账户回归已接入 CI，需核对对应提交。真实账户、升级和多 DPI 交互仍各自保留验收状态。

## 1. 当前基线与这轮交付

| 项目 | 当前证据 | 能证明什么 |
|---|---|---|
| 功能基线 | `3c69d2f` 汇总 N1/N2/N3/N4a | 主线功能已汇总；发布状态另行记录 |
| 功能基线源码回归 | 706 passed / 6 ignored；fmt、普通 Clippy、Release 和 JSON CLI 均 exit 0 | 对应 3c69d2f；后续改动分别复验，Clippy 仍有告警 |
| main 原 CI | [37169814380](https://github.com/Lyx721188/QuotaScope/actions/runs/37169814380)，对应 `3c69d2f`，成功 | Release 构建、便携包设置窗口生命周期通过；原流程没有执行 EXE 的安装/卸载 |
| DSH 独立复算 | 冻结字节的 Python 与 Rust 结果一致；缺 usage 保持 partial | 已检查样本可复算，不能代表所有历史文件完整 |
| R1d / N4b CI | [37184221991](https://github.com/Lyx721188/QuotaScope/actions/runs/37184221991)，对应 `dcaa37c`，成功 | 两个 Token 脚本均 Passed、使用相同 Release EXE SHA；只证明隔离合成账户 |
| R1 新 CI | [37171576671](https://github.com/Lyx721188/QuotaScope/actions/runs/37171576671)，对应 `ef9292d`，成功 | ZIP 许可证、相同 Release payload 的独立身份安装、生命周期与卸载通过 |

本轮改动：

1. 修正根 README 的 67/77 路由旧口径，移除 Windows README 的旧 14/17 表，统一指向 77 项 Windows 路由表。
2. 说明 54 个 Token 来源由 34 项已知格式和 20 项标准导入组成；新增主线能力与已发布版本分别说明。
3. 在 CHANGELOG 增加 Unreleased 条目，覆盖性能、DeepSeek、Codex、DSH 的用户可见变化和限制。
4. ZIP 包补入 LICENSE 与 THIRD_PARTY_NOTICES.md，解压后核对其哈希。
5. CI 使用当次 Release payload 编译独立身份的验收安装器，静默安装到临时目录，检查程序、资源和许可证，运行托盘/设置生命周期，再卸载并检查清理。
6. CI 保留安装、生命周期、卸载日志及 result.json；失败也上传已有日志。每次打包使用该次运行独有的目录，避免误选旧 payload。

CI 结果按源码 commit 和 `quotascope-installer-validation` artifact 核对。公开运行链接见上表；原始日志与个人样本保留在本地忽略目录，不上传到 Git。程序版本暂保持 1.3.1；下一次功能发布建议 1.4.0，因为汇总了多个用户可见功能，具体版本在发版准备时落实。

## 2. R1：发布前验收

目标：建立可重复的分发包验收，并形成准确的发布说明。先完成自动检查，再安排自动环境无法代表的本机升级与交互检查。

| 任务 | 操作与边界 | 通过标准 | 产物 |
|---|---|---|---|
| R1a 文档 | 统一 README、Windows README、CHANGELOG、移植状态；旧版本历史保留原日期/数字 | 当前入口不再宣称只有 67 个路由或 14/17 已移植；主线与 Release 区分明确 | 本轮文档提交 |
| R1b 便携包 | 在 CI 解压真实 ZIP，核对许可证；使用包内程序执行生命周期测试 | 哈希一致；首次打开、关闭重开、第二次启动、隐藏退出均通过 | CI 日志与 ZIP/SHA256 |
| R1c EXE 安装包 | 使用同一 Release payload 和 Inno 脚本，仅改验收 AppId、安装目录与输出名；不注册协议和桌面入口 | EXE 哈希/版本一致，资源/许可证存在；安装程序可运行；卸载后 EXE 和独立注册项移除 | quotascope-installer-validation artifact |
| R1d Token 页 | Debug 合成 UI 已覆盖；CI 新增 Python 3.14 的 DSH/ZCode 脚本，使用实际 ZIP 内 Release 程序，[dcaa37c 的运行](https://github.com/Lyx721188/QuotaScope/actions/runs/37184221991) 已通过 | 20→30 的追加样本、损坏提示、修复恢复、切源、关闭重开符合预期 | 带程序 SHA 的 Release 交互记录 |
| R1e 本机升级 | 明确升级窗口后备份已有配置；记录旧版本/程序哈希，完成升级和回退检查 | 账户配置、DPAPI 凭据仍可使用；登录不串号；旧版本可恢复 | 升级/回退记录 |
| R1f 发布准备 | 根据最终范围设置版本、更新锁文件/发布说明、构建并复验最终提交 | tag、Cargo 版本、EXE 版本及包名一致；所有阻塞问题解决，已知限制列明 | 可审查的 Release 说明和包清单 |

R1b/R1c 使用 CI 独立桌面和隔离配置。EXE 验收使用独立身份的安装器，验证相同 payload 和安装脚本；它仍不能替代原 AppId 的用户升级、可选协议注册、手动多 DPI 和真实账户验证。

自动检查失败时先保存日志、定位到打包/启动/资源/卸载步骤，只修复失败项，再重跑受影响检查。没有源码变化时不重复跑无关性能实验。

## 3. N4b：ZCode 原生用量读取

N4b-1 至 N4b-4 已完成：ZCode 3.14.4 的归一化 SQLite 用量格式、18 项定向回归、缓存与扫描、冻结字段独立复算和隔离 Debug UI。详见[字段、上限及命令](providers/windows-zcode-local-usage.md)。源码整仓检查为 724 passed / 6 ignored，普通 Clippy 通过并保留旧告警。最终 Release UI 已由 dcaa37c 的 CI 单独验收；不代表账户所有历史请求完整。下列任务卡保留验收规则。

### N4b-1 格式取证

- 输入：本机 ZCode 版本、实际数据位置、稳定的只读样本；先确认真实路径是否与现有 `.zcode` 候选一致。
- 公开记录仅含格式版本、表/事件键、字段类型、计数规则和合成示例。真实对话、私人路径、凭据及实际聚合数字只保留在本地忽略目录。
- 找到请求/回复身份、模型、时间、input/output/cache 计数及含义，确认推理 Token 是否已包含在 output，确认 usage 是单次增量还是会话累计。
- 若来源为数据库，检查 WAL/锁文件和只读一致性；若为 JSONL，检查追加、半行、轮转和重复记录。
- 完成标准：字段映射表、计数规则、不可判断情形、至少一组可独立复算的冻结样本。没有可信 Token 字段则停止实现 reader，保留标准导入并记录原因。

### N4b-2 纯解析与回归

- 入口：`windows/quotascope-core/src/additional_spend.rs`；若格式复杂，拆出专用模块，避免把异构解析继续堆入一个 match。
- 优先构造脱敏 fixture：正常、明确零值、缺值/负数/异常大数、未知模型、缺时间、重复回复、累计计数重置、fork/resume、损坏与截断。
- 按真实语义计数；缺计数不补零，未知模型不借用近似模型价格，重复/重放的排除规则必须有稳定身份依据。
- 完成标准：纯解析测试、独立期望值、缺口状态通过。此时仍不把来源标记为真实账户全量准确。

### N4b-3 扫描、取消与缓存

- 接入 roots/collect/native_supported 和现有 spend 注册，不增加重复 catalog 项。
- 限制扫描深度、文件数量、单文件/单行读取量；复用 scan 控制器，取消和读取中变更不能发布完整结果。
- 以路径、长度、精确时间戳和解析版本判断失效；覆盖追加、改写、同大小替换、删除、修复和进程重启。
- 完成标准：热读不反复扫描未变内容；刷新能看到追加/修复；取消不覆盖最后一个完整快照；异常被标成 partial。

### N4b-4 实际样本与 UI

- 对同一份冻结字节做独立聚合，比较 input/output/cache/total、模型/日期桶、重复记录数量和缺口类别。
- 在隔离 Token 页面核对来源选择、时间范围、筛选、排序、未计价提示、partial、关闭/重开和刷新。
- 完成标准：真实样本可复算，受影响模块检查与整仓检查通过；提供来源说明、命令、退出码和程序 SHA。
- 更新口径：只有这一步完成才将 ZCode 从“标准导入”改成“原生读取”，同步 34/20 等数量；总目录数量仍为 54，除非另有新增来源。

## 4. R2：逐服务商真实账户验证

每个账户单独执行，所需登录由用户本人完成。已有可用登录只做授权范围内的只读核对，不代为登录或输出密钥。

| 顺序 | 对照范围 | 必测边界 | 退出标准 |
|---|---|---|---|
| DeepSeek 官网 | 同账号、同自然日、同币种的余额/费用/Token | 无 API key、Key 拒绝、官网会话过期、空历史、金额缺失、跨日；附加账户独立刷新/清除 | 同口径数值一致，错误分类正确，主/附加账户凭据和缓存不串用 |
| Kiro ACP | 受支持的 Windows 原生 CLI；读取其已登录账户的额度池 | 初始化失败、登录过期、超时、缺额度上限、日期型重置、刷新后子进程回收 | 同账户 CLI 结果一致，无残留子进程，不把缺分母猜成百分比 |
| 其他常用路由 | 按用户实际使用选择 1–2 项推进 | 浏览器会话过期、空值、网络错误和来源标注 | 每项独立记录已实现/本地验证/实账验证状态 |

单个服务商缺登录不阻塞其他来源工作。报告给出差异字段、时间范围和复现步骤，不保存完整账单或浏览器凭据。错误和过期场景优先用隔离 fixture，避免主动破坏用户真实登录。

## 5. R3：多账户与 Windows 体验

| 优先级 | 工作 | 验收 |
|---|---|---|
| 第一批 | 附加账户交互式登录/OAuth 续期；Codex 独立登录进程 | 登录、刷新、撤销、删除后的凭据/缓存/进程按账户隔离；不借主账户登录，不回收他人进程 |
| 第二批 | 快捷键冲突提示、多 DPI 移动/拖拽/滚动、窗口位置恢复 | 冲突可理解；不同缩放和负坐标显示器不丢窗口，重启保留合理位置 |
| 第三批 | Windows 专有文字的繁中/日/韩补齐，最后处理完整 BotMark | 核对实际 UI 字符串和布局，缺译可追踪；动画不增加持续后台负载 |

不把这些项目混入一个提交。账户安全和读取正确性先于视觉扩展；已取消的悬浮行为、Claude 跨文件去重仍保持既定范围。

## 6. 建议提交顺序与交接

1. 已完成：CI 安装/卸载验收、ZIP 许可证与入口文档，`ef9292d` 的 Windows CI 成功。
2. 已完成：ZCode 格式记录、纯解析及只读 SQLite。
3. 已完成：ZCode 扫描/缓存/Debug UI 与独立复算；35/19 支持数已更新。Release Token 页由 dcaa37c 的 CI 通过。
4. 标准导入、导入资源上限、浏览器回退副本分别随 53bd960、f08ee5a、3e95c23 完成整仓及 Release CI。日期展开与取消已通过本地 744 项测试及隔离 Debug UI，规则见[日期范围](providers/windows-ledger-calendar.md)。日期改动的 Release CI 已通过；ZCode 固定头部签名的 Release CI 已通过；模型时序扫描上限及独立缺口提示已通过整仓 750 项测试，隔离账户 UI 也已通过，其 Release CI 已通过；时序日期/计数有效性通过整仓 756 项及扩展账户 Debug UI，对应 Release CI 已通过；Codex 原生计数及缺口缓存通过整仓 766 项和账户/维护/续读/筛选/取消 Debug UI；核对本批 Release CI（含账户）后，唯一下一项是原生 Codex/Claude 文件发现的目录/路径容量边界，然后推进 DeepSeek 与 Kiro 每个服务商各一批验收/修复；需要登录的缺口独立记录。
5. 按实际优先级推进多账户与交互；正式发布时单独完成版本/tag/包的一致性。

每次交接至少包含：分支/commit、改动行为、验证命令和退出码、程序与包哈希、真实账户与桌面各自状态、未解决问题、唯一下一项。原始日志与个人交接放 `windows/target/` 或 `work/`；只提交脱敏结论和合成 fixture。
