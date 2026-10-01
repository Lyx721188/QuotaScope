# Windows 与上游功能差距

核对日期：2026-10-01。上游固定为 `qunqin24/Pulse` 的 `a3415cc7c8265152646015680e80329c09f2f62f`
（本轮重新 fetch 后的 main，2026-09-30）。Windows 基线为 `59a97c2`，另含本轮停靠/间距修复。
依据是两端源码、服务注册表与设置入口；不是对所有真实账户的联网验收。

## 已有的主要能力

- 77 个内置 provider 的目录；67 个已注册读取路由。路由存在不等于该 provider 的全部登录方式、统计功能均已实现。
- 用量环、第二环、窗口时钟、剩余量、线性耗尽预测、余额基准与预算；边缘停靠、跟随显示器、自动收起。
- WinUI 设置页、账户分组与搜索、凭据保存与明文切换、通知与自启动。
- 18 个浏览器会话 provider 的导入基础设施，Windows DPAPI 凭据保护。
- Claude/Codex 本机账本与 API 价值估算，z.ai/智谱服务器统计，按账户开关详细卡。
- 托盘摘要、用量页链接、扩展 manifest 与自建网关、只读缓存 `--json`。
- Windows 原生 Mica/DirectWrite/WinUI，以及随包 HarmonyOS Sans。

## 未完成清单

| 范围 | 当前状态与差距 | 直接源码依据 |
|---|---|---|
| 10 个 provider | **未移植**：Kiro、Ollama Cloud、Grok Bot、Volcengine、Devin、Alibaba Token Plan、Gemini、JetBrains AI、Windsurf、Nous Portal | [Windows 路由表](Docs/providers/windows-ports.md)、[支持标志](windows/quotascope-core/src/model.rs) |
| Token spend 数据源 | **部分实现**：上游 `SpendAgent` 实际 54 项；Windows 只有 Claude Code/Codex 两个本机 reader，另外 52 个数据源未移植。上游也区分原生计数、导出/捕获、仅费用及无法计数，并非 54 项都有同等验证 | [上游 SpendAgent](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/Usage/SpendAgent.swift)、[Windows ledger](windows/quotascope-core/src/ledger.rs) |
| Token spend 分析页 | **未移植**：专门页面、时间范围选择、按 agent/model 下钻、可排序/分页的汇总表、来源覆盖与不可计价分类。现在只有详细卡的近 30 天活动摘要 | [上游 TokenSpendView](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/Settings/TokenSpendView.swift)、[Windows 设置页](windows/quotascope-win/src/settings_app.rs) |
| 详细卡缓存功能 | **未移植**：缓存命中率、Claude 提示词缓存的 5 分钟/1 小时倒计时、会话数量及过期提示。上游最新提交明确增加了这些功能 | [上游 UsageDetailCard](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/Panel/UsageDetailCard.swift)、[Windows card](windows/quotascope-win/src/card.rs) |
| 其他账户的历史 | **部分实现**：Windows 历史只接入 Claude、Codex、z.ai、智谱；上游还能将其他 agent 的账本映射到账户详细卡 | [上游 CardLedgers](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/Panel/CardLedgers.swift)、[Windows history](windows/quotascope-core/src/history.rs) |
| Codex app-server | **未移植**：app-server 凭据/用量回退、账户累计 token、峰值日、连续使用天数、reset credits 数量/有效期 | [上游 CodexAccountUsage](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/Providers/CodexAccountUsage.swift)、[Windows Codex](windows/quotascope-core/src/providers/codex.rs) |
| Claude 登录回退 | **部分实现**：CLI OAuth 已有；Status Line 与桌面 session 回退未移植。已修正文档中容易被误读为支持 Status Line 的来源栏 | [Windows Claude](windows/quotascope-core/src/providers/claude_code.rs) |
| 多账户 | **未移植**：同一内置 provider 的附加账号创建/登录/切换。现在只启用主账号；扩展 account slot 不能替代完整内置多账户体验 | [Windows settings](windows/quotascope-core/src/settings.rs)、[上游 AppSettings](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/App/AppSettings.swift) |
| 账户外观与窗口选择 | **有底层、缺入口**：provider 顺序、固定 headline 窗口、环颜色均有存储或绘制支持，设置 UI 尚未提供重排、选择窗口与颜色工具；标签上下位置等也未完整暴露 | [Windows settings](windows/quotascope-core/src/settings.rs)、[panel](windows/quotascope-win/src/panel.rs)、[settings_app](windows/quotascope-win/src/settings_app.rs) |
| 托盘 dashboard | **部分实现**：当前是原生文本菜单和链接；上游有按账户切换的完整 dashboard，包含全部窗口、历史、估值及其他附加信息 | [上游 MenuDashboard](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/App/MenuDashboard.swift)、[Windows tray](windows/quotascope-win/src/tray.rs) |
| 启动额度窗口 | **未移植**：WindowPrimer 与按 provider 设置的自动启动周期/运行结果 | [上游 WindowPrimer](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/Usage/WindowPrimer.swift) |
| 连接诊断与路由选择 | **部分实现**：Windows 有失败原因与系统代理；缺少独立诊断页面、导出诊断报告、用户手动代理覆盖、账户读取来源及浏览器选择等完整设置 | [上游 ConnectionDiagnosticsView](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/Settings/ConnectionDiagnosticsView.swift)、[Windows http](windows/quotascope-core/src/http.rs)、[settings](windows/quotascope-core/src/settings.rs) |
| 集成与快捷键 | **部分实现**：已有 `--json`；缺少开发者集成设置、Status Line 安装入口、深链接和全局“打开设置/切换面板”快捷键 | [上游 DeveloperIntegrationsView](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/Settings/DeveloperIntegrationsView.swift)、[GlobalShortcut](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/App/GlobalShortcut.swift) |
| 额度语义 | **P1 已补齐并通过本地回归**：daily/messages/topUp/credits/sharedCredits 独立类别、同日到期额度汇总、noPlan 与 JSON 元数据。信用充值不再被判断为重置；仍保留 Other 用于上游同样使用的其他周期 | [上游 ProviderUsage](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/Usage/ProviderUsage.swift)、[Windows model](windows/quotascope-core/src/model.rs) |
| 图标、语言与动画 | **部分实现**：Windows 位图映射覆盖 17 个 provider raw id，另外 60 个使用字母等回退；界面仅中英，缺繁中/日/韩和语言选择入口；上游的 Bot mark/persona/shape 动画尚未移植 | [Windows assets](windows/quotascope-win/src/assets.rs)、[localization](windows/quotascope-core/src/localization.rs)、[上游 AppSettings](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/App/AppSettings.swift) |
| 显示模式与更新 | **未完整对齐**：全屏隐藏、仅托盘模式等设置未完整暴露；缺 Windows 自动更新器。上游 Sparkle 本身是 macOS 组件，Windows 应实现对应更新能力 | [上游 AppSettings](https://github.com/qunqin24/Pulse/blob/a3415cc7c8265152646015680e80329c09f2f62f/Sources/Pulse/App/AppSettings.swift)、[Windows settings](windows/quotascope-core/src/settings.rs) |

macOS 缺口屏幕、菜单栏位置、Liquid Glass、钥匙串与 Spaces 不列为必须原样移植的 Windows 缺项；应按平台实现对应体验。
用户本轮明确取消了悬浮停靠，因此它不再属于待补功能。

## 建议接下来按依赖顺序推进

1. **先完成交互可靠性验收**：三边自动收起与唤回、间距/大小组合、旧悬浮配置兼容；本轮修复范围。
2. **详细卡信息对齐**：先补额度语义，再做缓存命中率/TTL 与 Codex 账户统计及 credits；每项都需有效数据与空/失败状态。
3. **Token spend**：先建立独立页和来源覆盖状态，再逐类接入剩余 reader；各来源须注明原生记录或显式导出，不能按来源数量宣称真实验证。
4. **账号配置体验**：顺序/固定窗口/颜色入口，随后多账户与诊断/来源选择。
5. **剩余 provider 与体验**：按实际使用需求推进 10 个路由；补图标、语言、快捷键、集成与 Windows 更新能力。

这个清单是源码差距审计，不代表本轮已经承诺或完成后续全部移植。
