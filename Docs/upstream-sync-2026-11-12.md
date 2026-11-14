# 2026-11-12 上游同步

上游参考为 [Pulse `338c796`](https://github.com/qunqin24/Pulse/tree/338c796)（2026-11-11，1.13.0 发布后的 main），比较起点为之前已拉取的 `b3bf083`。用户已明确要求完整同步 Windows 可用的新功能。本轮使用独立 Rust/WinUI 实现移植协议与交互行为，保留 Windows 1.3.1 的版本号，改动列在 Unreleased。

## 本轮实现

| 范围 | Windows 行为 |
|---|---|
| Open Design / OpenDesign Go | 读取个人工作区、5 小时/日/周/月额度、套餐名和接口明确返回的美元钱包余额。优先导入的浏览器登录，其次桌面 prod 登录。账单接口之一拒绝时继续尝试其余接口；无数据不画 0% |
| DimAgent | 通过 `dim usage --json` 读取套餐积分、独立加油包、各自 Token 窗口和 web_search 额度；过期包不作为当前额度，包的到期不冒充窗口重置 |
| Antigravity CLI | 自动模式先问桌面 language server，再尝试 agy；设置中也可固定 CLI 或桌面。先读版本，低于 1.1.11 或无法识别版本时绝不调用 `/usage`；CLI 不产生模型请求，日志写入 NUL |
| Cursor | 识别 get-plan-info 的 weekly 套餐，以 get-current-period-usage 的周额度替换月度模型池，保留额外消费额度；企业账户的 overall 单独作为月额度 |
| 小米余额 | 无 Coding Plan、但余额接口有有效读数时显示预付余额，接入已有余额口径和低余额提醒；有套餐时继续使用套餐窗口 |
| 官方状态页 | 上游全部 44 家服务商的协议已接入；MiniMax 在同一目录中按站点选择状态页，44 家服务商对应 45 个可读取站点。支持 Statuspage、incident.io、Flashcat、Better Stack、Instatus、Upptime、Gatus、UptimeRobot、OpenCode、DeepInfra、IBM、Google Cloud；同公司共享摘要请求，按组件筛选，OpenCode 保留 56 个三小时条与缺测 |
| 扩展图标 | manifest 可选 icon 字段，限定扩展目录内 PNG、256 KiB、1024×1024，检查容器与 CRC；平台解码失败回退文字。扫描时保存字节、按扩展账户查找，内容哈希变化后下次绘制重解码 |
| 浏览器选择 | 通用设置和 Open Design 设置可指定浏览器。默认项只读取系统默认浏览器；所有浏览器是显式选项。Cookie 与 Chromium localStorage 导入遵循同一选择 |
| 会话恢复 | 所有注册的浏览器 Cookie 服务在后台刷新时重新加载自己的 DPAPI 凭据，修复已导入却仍提示缺少登录的问题 |
| Token 范围 | 本月、上月、今年和自定义首末日期（含边界），兼容已有最近 7/30/90 天等范围；自定义范围可保存，日期无效不应用；年度活动图继续独立于范围选择 |
| 计数与价格 | Grok 输入只拆出已有缓存，输出不重复加 reasoning；仅已确认的 Gemini 3.8 Flash -n / -xpe 变体使用基础模型价格，实验模型保持未计价 |

内置服务商及读取路由共 **78** 项，MiniMax CN 已并入 MiniMax 的站点选择。本轮没有增加 Token 来源目录数量；MiniMax Code 与 Command Code 升级为原生读取，Kimi 同时读取新旧目录。新服务商图标从同一上游 commit 复制并转换为 Windows PNG；来源与许可见 [第三方声明](../THIRD_PARTY_NOTICES.md)。

## Open Design 配置

1. 在浏览器登录 [Open Design 控制台](https://open-design.ai/cloud/dashboard)。
2. 打开 QuotaScope → 账户，搜索 Open Design，展开配置，选择登录所在浏览器并点“从浏览器导入”。Chrome / Edge 数据库受保护或锁定时，选择“所有浏览器”可通过本机已连接的 OpenCLI Browser Bridge 读取。也可以粘贴请求中的 Cookie header。
3. 启用账户并刷新。已经登录桌面应用时，可以直接读取 `%USERPROFILE%\.amr\config.json` 的 prod profile。

**模型调用 API Key 不能查询账户额度。** 只会保留 better-auth 前缀 Cookie，且必须有 `__Secure-better-auth.session_token`。桌面登录文件只读，pendingLogin 不算已登录；controlKey 仅发往 HTTPS 的 open-design.ai 或其子域，请求不跟随重定向。Cookie 按现有 Windows DPAPI 方式保存。附加账户使用独立会话，不回退到桌面主账户。

直接数据库通道兼容 Chromium v10 / v11；App-Bound（v20）和锁定数据库可通过已连接的 OpenCLI Browser Bridge 导入，或手动粘贴 Cookie header。桥接仅在选择“所有浏览器”时启用。导入在后台执行，结果持续显示。

DimAgent 需要自行运行 `dim auth login`，程序只启动 CLI，不接管其凭据。两个新增 CLI 使用隐藏进程、有限输出与超时，Windows Job 在退出时回收子进程。

## 本轮扩展实现

- MiniMax 合并国际站与中国站目录，两个加密密钥槽位独立保留。旧附加账户复制后读回确认，再迁移显示偏好。Qoder、StepFun 同样按站点保存凭据，保存密钥与切站都会使旧凭据对应的缓存失效。
- Qoder 保留个人与团队积分的原始用量、总量和接口提供的余额。设置可选择显示数值。31 天分页记录在设置及详情卡中显示今天、7 天、31 天的折前与消耗，分页不完整会保留失败状态。
- Token 回顾支持日、周、月、年和前后周期导航，切换粒度保留所选时段。月初七天、年初七天、每周第一天与凌晨五点前沿用上游默认周期规则，使用当前工具及模型筛选。原生 Direct2D/WIC 输出 1080×1920 PNG，可保存单张、整组及复制图片。卡片包含总览、工具与模型、月历、年历、月度条形图、时段、订阅比较及成绩单，按数据覆盖情况出现。
- MiniMax Code、Command Code 与新 Kimi Code 会话升级为原生 Token 来源。Devin 合并 Oh My Pi 中明确标注 devin 的记录。Codex 活动与归档会话以较长副本去重，并迁移旧计数档案。
- 账户设置统一展示可读取的历史。附加 Qoder 与 DeepSeek 使用各自凭据。关闭本机读取或离开页面会取消对应扫描，凭据改变后的迟到响应不进入当前显示。
- 估算金额支持 USD、CNY、HKD、TWD、JPY、KRW、RUB，使用每日公开汇率或手工汇率。实际账单与余额保留服务商报告的币种。
- 连接诊断捕获共享 HTTP 客户端及服务商专用路由的响应，保留方法、地址、状态、类型和有限正文。已知凭据及常见敏感字段脱敏，每次检查最多 8 项、每项 32 KiB，保存在内存中。
- 分段进度条、关闭边缘吸附、到达屏幕四角的自由放置、Antigravity 最多三个模型组圆环及独立开关已接入。Gemini 4 Argon 的估值归属独立处理。
- 详情卡标题固定，切换账户保留滚动位置。机器人头像可按账户选择颜色、性格、头型，折叠时暂停。Claude Code、Codex、Command Code 的工作标记读取有限生命周期记录。实验弹性吸附使用 Windows Mica 窗口的原生弹簧位移。
- 测试通知、设置行图标、俄语和最新上游词表已接入。Codex 的“较弱模型”推断面板退出生产 UI。新版 Cursor 的加密登录使用当前 Windows 用户解锁，OpenCode 的 EntitlementError 显示无套餐。
- 安装版本消息从 QuotaScope 自己仓库的 Messages/<version>/<language>.md 读取并可关闭。GitHub 提示至少经过两周且有七个活跃日才出现一次。关于页另行标明 Pulse 上游作者的支持链接。

## 验证结果

- `cargo fmt --all -- --check` 与 `git diff --check` 通过。
- `cargo test --workspace --locked --quiet -j2 -- --test-threads=1` 通过，合计 **892 passed / 10 ignored**。默认忽略项包括依赖桌面、真实环境或独立性能条件的检查。
- `cargo clippy --workspace --all-targets --locked` 通过，现有及本轮的样式告警保留在日志中。
- `cargo build --workspace --release --locked` 通过。Release CLI 的 `--help`、空配置 `--json`、旧配置 `--json`、设置字节和修改时间保持、缓存与凭据目录保持均通过。
- 八项隔离 Debug UI 回归通过，覆盖新增服务商入口、缺少会话诊断、浏览器、分段显示、自由放置、弹性吸附、手工汇率拒绝无效值、站点、头像、Antigravity 通道与分组、俄语切换、日周月年回顾、PNG 导出、账户时序、服务状态、更新、DSH、ZCode 与 Hermes。
- 中文关闭重开和账户页离开后自动重读已实测。CLI 集成回归覆盖空配置和迁移前配置的只读要求。
- 原生 WIC PNG 显式回归通过，生成中、英、俄三种语言、四种周期的 **69 张** 1080×1920 卡片，已检查代表图像的布局、中文、俄语与未知日期。
- 45 个官方公开站点详情实测完成，44 个当前状态可读，AIXY 返回 HTTP 403。Cursor / Grok Bot 的 HTTPS 重定向兼容后当前状态可读，其历史和 ZoomMate、Replicate 的历史保留缺口提示。

本地证据在忽略目录 `work/upstream-2026-11-12`，包含测试、Clippy、构建、逐站点结果、八份 UI JSON、PNG 及 Release 检查结果。CI 已接入新增设置回归，后续 CI 将对 portable Release 运行同一组脚本。本轮本机 UI 验证使用隔离 Debug 实例。

首次同步使用上游合成数据验证 Open Design、DimAgent 等鉴权协议。Open Design 的真实账户验收见下节。DimAgent 的真实账户额度仍待登录验收。Chromium v20 的直接数据库解密边界继续适用。

首次同步构建的 Release 程序为 `windows/target/release/quotascope.exe`，运行所需 DLL、PRI、Fonts 与语言资源位于同一目录。该次 SHA-256 为 `C6B878869386DF549C98EA6807CD9E28AB51BC30DD95E8005F34CB151A670ADC`。当前修正版见下节。

## 2026-11-13 导入修复与覆盖安装

- 本机 Edge 登录会话存在，但原有数据库通道无法读取。增加已连接 OpenCLI Browser Bridge 的本机读取通道，选择“所有浏览器”时使用。真实会话可正常导入，两个 Open Design 额度接口均返回 HTTP 200。
- 导入转为后台任务，持续显示进行中和结果。保存后读回 DPAPI 凭据确认，浏览器、站点或凭据变化会使旧任务失效。
- 修复带会话账户缓存重启时的重复加锁。先复制启用账户，再释放设置锁并验证缓存对应的凭据。
- 完整回归 **894 passed / 10 ignored**，Clippy、fmt、Release 与 CLI 检查通过。带合成会话缓存的设置启动、导入失败消息持续显示及原有设置回归通过。首次完整测试有一项 Kiro 初始化超时，单项和随后完整重跑均通过，首次日志保留为 `import-full-test-first-attempt.log`。
- 修正版已覆盖安装至 `%LOCALAPPDATA%\Programs\QuotaScope\quotascope.exe`。在已安装的 Release 中实际点击“从浏览器导入”成功，自动刷新得到两项真实额度和套餐信息。导入前后的其他加密凭据逐项核对一致，四个已有启用账户保留。正常退出并重新启动后，设置与 Open Design 配置可正常打开。

当前 Release SHA-256 为 `8D5649B44A37D610FFA14A15244FEAF455D1C38DDC593773E70992E15EE0F9FC`。安装程序 SHA-256 为 `788AA827816D30C33C70A89EE3ED2598DC4842AFFE4DADD4D4D6D3D4FED99063`。程序和数据备份位于 `work/upstream-2026-11-12/upgrade/backup-20261113-131902`，已安装按钮、额度、其他凭据及重启验收结果在 `work/upstream-2026-11-12/upgrade/live-import-result.json`。

验收脚本曾因 PowerShell 自动解析日期后重复换算 UTC 而误报刷新超时，修正后已重新导入并验证新的额度时间。

## 完成核对

- [x] 日、周、月、年 PNG 实际渲染。
- [x] 中文新增文案、俄语词表和语言切换。
- [x] 新增功能的正常设置入口与持久化，历史和 Token 扫描的取消、离页、关闭重开。
- [x] fmt、整仓测试、Clippy、Release、隔离 UI 和 CLI。
- [x] 功能目录、数量、扩展图标契约、许可和验证结果。

macOS 刘海、Liquid Glass、Sparkle、Dock 和标题栏由 Windows 原生平台界面承接。弹性吸附保留 Mica，水滴表面形变依赖上游的 macOS 绘制实现。ego lite 的上游发现路径仅覆盖 macOS。Windows 的日志缓存继续使用已有格式。

已有 Windows 来源及交互式 OAuth 边界记录在[移植进度](upstream-implementation-status.md)。
