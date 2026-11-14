# Windows 官方服务状态

本轮参考 Pulse `338c796`，覆盖上游 44 家服务商、45 个可读取站点。MiniMax 国际站与中国站根据账户所选站点展示。当前状态、独立故障通知、历史条和官网公布的可用率均走公开状态页面，缺少可用率时保留未知。

## 查看与通知

在设置的账户页展开受支持服务商，可查看官方组件名称、当前状态、可读取的历史条、官网可用率和检查时间，并手动刷新或打开官网。查看状态独立于本机 Token 读取与账户凭据。

现有三家解析继续支持以下通知规则。

| 服务商 | 当前状态来源 | 通知范围 |
|---|---|---|
| Codex | incident.io 官方页面的 Codex 分组，隐藏组件排除 | 该分组的可见组件 |
| Claude Code | Atlassian Statuspage summary，排除组标题和按规则隐藏的健康组件 | Claude Code 与 Claude API 的固定组件 ID；聊天网站等其他组件仍可查看 |
| DeepSeek | Flashcat 网页内 Next.js Flight 的 `initialData`，展开可见分组 | 名称含 `API` 的服务；聊天网站仍可查看 |

另外覆盖 Cursor、Grok Bot、Copilot、Devin、Windsurf、Augment、Factory、Kilo Code、MiniMax、Kimi Code、Moonshot、Warp、Manus、Venice、ZoomMate、Replicate、OpenAI Platform、JetBrains AI、Amp、AIXY、ElevenLabs、Notion AI、Perplexity、Poe、v0、Hugging Face、Chutes、Synthetic、DevPass、Raycast AI、TypeSafe、Zed、GitKraken、Atlas Cloud、Command Code、Cline Pass、Hyper、OpenCode Go、DeepInfra、IBM Bob 和 Gemini。页面地址、组件筛选和通知规则由 [service_status.rs](../../windows/quotascope-core/src/service_status.rs) 与 [service_status_extra.rs](../../windows/quotascope-core/src/service_status_extra.rs) 定义。

新增协议包括 incident.io、Better Stack、Instatus、Upptime、Gatus、UptimeRobot 和各站点自有 JSON。通知页的“服务商发生故障或恢复时通知”默认关闭，独立于额度预警。开启后每五分钟检查已启用且支持状态页的服务商，多个账户及同公司的摘要请求共享缓存。关闭某个服务商或故障通知开关，会遗忘相应通知记录。

故障首次出现、严重程度恶化时通知；仍有故障但有所改善时不通知，并允许下一次恶化重新通知。仅对曾通知过且官方明确恢复正常的组件通知恢复。维护、陌生状态或网络失败都不构成恢复证据。组件从页面消失时静默遗忘，不补发恢复通知。通知状态单独保存在 `%APPDATA%\QuotaScope\status-alerts.json`，重启后仍能去重。

## 历史与可用率

OpenAI 按本机自然日展示含今天的 91 天，DeepSeek 展示 90 天；每个历史事件按实际重叠日期取当日最严重状态，尚未开始和未来事件不影响过去。Claude 保留官网近 90 天的日期与可核对的条形颜色，不把缺损日期的位置错配到其他日期。早于官网数据起点的日期显示无数据。

OpenCode 按官网绘制 56 个三小时窗口，覆盖七天，时间边界对齐 UTC。其他站点按公开协议提供的日历史或当前组件展示。

可用率只显示官网提供的有效百分比，不从彩色条数量反推。历史读取失败会单独提示缺口，已经读到的当前状态仍保留；当前状态失败则不能由旧历史证明当前健康。后台故障通知只读取当前状态，展开账户才读取历史；DeepSeek 两者复用同一公开响应。

## 读取边界

账户展开页和后台通知共享每家五分钟的内存缓存；失败也有冷却，手动刷新可立即重试。关闭、收起或离开账户页后，已启动的有界请求可结束，但晚到结果不能重新填回旧页面。未展开且未开启通知时不请求状态页。

每个公开请求链最多 15 秒、响应最多 4 MiB，最多展示 128 个唯一组件。请求使用软件的代理设置，公开状态页可跟随最多三次 HTTPS 重定向。带账户凭据的服务商请求继续采用自己的受限客户端。Flashcat 字符串按 JSON 解码，嵌套遍历有深度和节点上限。

状态页读取失败显示明确提示；若保留前一读数，其原检查时间保持不变，不能当作新读数。陌生状态显示“无法识别的状态”。官方未列出的受影响组件仅在对应完整、可解析的协议列表中按官网语义视为正常；不可解析的页面不能变成“全部正常”。

## 验证

2026-11-12 对全部 45 个站点的当前读取和详情读取进行了独立 profile 实测。44 个站点当前状态可读，AIXY 返回 HTTP 403。Cursor / Grok Bot 的重定向兼容后当前状态可读，其历史和 ZoomMate、Replicate 的历史返回缺口，界面保留可确认的当前状态。逐站点结果位于本机忽略目录 `work/upstream-2026-11-12/status-live-final.tsv`。本轮完整验证见 [同步记录](../upstream-sync-2026-11-12.md)。

历史批次本地整仓 807 passed / 6 ignored，fmt 与普通 Clippy 通过（233 项已有诊断，无新增）。隔离 Debug UI 的独立开关、禁用服务商不请求、失败不显示正常、三家区域、手动重试、离页和关闭重开已通过。先前当前状态批次的 [3f96e89 Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37210111588) 成功，服务状态与 DSH、ZCode、账户 UI 均使用同一 portable Release 程序；[历史批次 edae9e6 的 Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37212105208) 已通过，四份 UI 验收使用同一实际 portable Release 程序。

纯解析与通知回归覆盖公开冻结页面、Codex 分组、组件顺序和隐藏规则、Flight 跨块拼接、未知状态、超限与重复 ID、故障／恶化／改善／恢复、关闭开关、组件移除、独立账户去重和磁盘重启去重。合成 UI 脚本使用隔离 profile 与拒绝外连的回环代理，覆盖失败提示、手动重试、独立开关、离页和关闭重开。

UI 运行：在 `windows/` 中，用包含 WinUI 运行时资源的程序执行：

```powershell
./quotascope-win/tests/service_status_ui_smoke.ps1 -Executable target/debug/quotascope.exe
```

另外在 2026-10-04 使用空账户、关闭本机 Token 读取的隔离 Debug 窗口读取真实公开页面，三家的当前状态、历史条与官网可用率均可读。实时探测运行方式为在以上命令追加 `-ProbePublicPages`；它会访问公开官网，分别记录当前状态、历史条和官网可用率是否可读，不作为依赖外网的 CI 必过条件。

冻结页面和隔离失败回归本身不证明官网实时可达；公开页面实时探测不证明真实账户成功。官方状态是服务商公布的状态，可能晚于实际故障，不能据此推断当前请求一定成功或失败。
