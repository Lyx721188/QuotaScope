# QuotaScope Windows 1.2 交接

更新：2026-10-01 16:32（Asia/Shanghai）。工作区 `D:\Projects\QuotaScope`，分支 `main`。

## 恢复来源与当前范围

接续 ZCode 会话「继续移植上游用量估计功能」：
`sess_fe513453-66ce-414f-b86c-47d6f3d36ff5`。
只读核对 tasks-index/会话 SQLite 的用户与公开消息；未把截图进度当作代码完成证据。
公开消息摘要保存在 `work/zcode-resume-2026-10-01.json`。

接管基线：`97266b4`，只有 `model.rs::usage_page` 尚未提交，`work/` 已经未跟踪。
已保留并完成这份 URL 表，没有重置工作树，也没有启动或委派其他 agent。

- `570e654`：Windows 改名与旧 Pulse 数据兼容。
- `bb24b0e`：M1–M5、剩余 API/keyAndAddress 路由与 Claude/Codex 价值估计链。
- `97266b4`：M6、浏览器 cookie 基础设施及 18 个 session provider。
- `18f969a`：M7 功能与 M8 Windows 文档/版本/本地包。
- 后续本轮：依用户截图调整用量卡、设置页、进度条端点、HarmonyOS Sans 与悬浮光晕。没有推送、打 tag 或创建线上 Release。

## 本轮完成

1. **托盘 dashboard**：启用账户按 rail 顺序显示摘要；旧读数标记过时，失败读数显示原因。
   用量页子菜单使用上游的 18 个明确 URL，补齐接管时漏掉的 Ollama URL。
   菜单复用 App.readings，不额外取数；扩展名称按 manifest 显示。
2. **设置账户页**：订阅/API 分组（按 billing，不按是否输入 key），名称/raw id 搜索，
   PasswordBox 明文切换，保存/搜索/切页等动作隐藏明文。搜索也过滤扩展账户。
3. **保存修复**：网关 Save 同时保存编辑后的 key 与地址；没有 key 草稿时保留原 key。
   无效地址不会保存 key。空白 key 仍表示用户明确清除该凭据。
4. **余额 UI**：每个可报告余额的 provider 有 Since top-up/Balance only/My budget，
   正数预算、Warn below 与保存按钮。NaN/inf/零/负数/半输入拒绝；空白提醒关闭。
   明确保存时写 per-account basis，避免清除预算后旧 DeepSeek 全局预算重新生效。
   补齐 Replicate/TypeSafe 的余额能力标志。自身限额优先于推算余额环。
5. **详细卡**：`settings.detailed_cards` 按 account id 保存，默认关闭；展示 plan、更新时刻、
   有实际长度/reset 的窗口时钟。卡片尺寸由同一套附加行数据测量和绘制。
   `PanelWindow::set_entries` 在历史返回或详细开关改变时重新放置/调整已打开卡片。
6. **历史**：`history.rs` 支持 Claude/Codex 本机账本与 z.ai/智谱服务器统计。
   本机历史/价值估计受 `reads_token_spend` 门控；服务器统计对已启用且打开详细卡的账户读取。
   国际/大陆 host 分开，查询近 30 天，小时桶汇总为日期并填补日期空隙。
   Missing key / Failed / Answered(empty) 区分。服务器无 token 类型拆分，只展示 token，不定价。
   本机扫描无法把不可读/删除记录算进去，空状态措辞是“无可读取的本机记录”。
7. **线程/失效**：历史与估算离开 UI 线程，每 5 分钟更新。
   设置/保存/手动刷新清除历史并增加 generation；旧配置的请求结果被丢弃。
   Token spend 关掉后卡片不展示任何本机估算/历史。
8. **文档/版本**：Windows crates 和 lockfile 升至 1.2.0（未发布）；
   README 修正“完全不读会话正文”的旧表述，说明本机扫描、models.dev 和 cookie 导入边界。
   `Docs/windows-1.2.md`、`Docs/providers/windows-ports.md`、`Docs/extensions.md` 为当前 Windows 文档。
   旧 macOS provider 说明前增加 Windows 来源边界。根目录 VERSION 是遗留 macOS 版本，不控制 Windows 构建。

## 视觉改进

- 对照本地 `upstream/main` 的 UsageDetailCard/AccountUsageCard.swift：250 点宽度、18 点内边距、紧凑标题及窗口行。
- 今天 / 7 天 / 30 天活动摘要、短格式 token、30 个日历日期柱，灰色胶囊图形与当日强调。
  本机估值标为 API 价值估算；未公开价格和服务器统计不推断费用。
- 单一圆角进度条取代矩形与圆端的叠加，消除端点深色接缝。
- 同一内容函数测量并绘制卡片，移除固定空白尾部；空/失败及 CJK 多行内容也参与测量。
- 移除鼠标跟随强调色径向光晕及其弹簧动画；保留环的轻微悬停反馈。
- 设置页统一卡片和右侧控制列；账户默认摘要，配置可展开/收起，启用账户优先。
  搜索、切页、保存、收起配置仍隐藏明文；凭据草稿按 provider 保留。
- 常规设置按面板/刷新/Windows/Token spend 分区；切页重置滚动位置。
- 私有加载未修改的 HarmonyOS Sans SC Regular / Medium / Bold，随包保留许可与关于页声明。
  DirectWrite 使用三字重字体集合；WinUI 通过应用字体资源使用包内字体，未安装系统字体。
  Windows 标题栏/菜单及图标字体由系统控制。

## 验证证据

最终源码检查均退出 0：

- `cargo fmt --all -- --check`
- `cargo test --workspace`：476 core + 3 M7 integration + 31 existing integration + 7 Windows tests，
  **517 passed，2 ignored**。忽略的真实环境测试没有运行。
- 新字体测试验证实际字体家族、三种非合成字重及中文字形；新增卡片测试覆盖短格式数字、CJK 换行及三个面板尺寸。
- `cargo clippy --workspace --all-targets`：退出 0，仍有风格警告；不是 `-D warnings` 零警告验收。
- `cargo build --release -p quotascope-win`：最终构建约 1m21s。
- `git diff --check`：通过。

日志：`work/visual-tests.log`、`work/visual-clippy.log`、`work/visual-final-build.log`。
本地包：`work/quotascope-windows-1.2.0.zip`（43,293,789 bytes），
展开目录 `work/quotascope-windows-1.2.0/` 包含 exe、DLL、PRI、语言资源和完整 `Fonts/`。
Windows FileVersion 为 1.2.0。

- exe SHA256：`DD95C5776CDB702426CE5D1D66DB008AF20DBFDA2CAAB758F257A61D04A644B4`。
- zip SHA256：`740AE9AA1329C62923A2DFC34FDDC48C622835A520644772A8B8DED22784535D`。
- 包清单：`work/visual-package.json`（2026-10-01T16:29:35+08:00）。
- 最终包 exe 的 `--json` 退出 0、0 账户、字段正确；隔离配置哈希不变且未写新数据文件：
  `work/visual-json-validation.json`。

## 交互验收与证据边界

通过 computer-use 原生窗口快照验收，实际运行的是包内 exe：

- 常规页右侧控制列对齐、分区卡片和 HarmonyOS Sans 生效；账户页默认紧凑摘要、启用账户排序、独立卡片和配置展开/收起均已实测。
- 真实 Codex OAuth 读数成功，本机历史/估值成功；最终卡片为 250 × 497 点（启用预测时），
  双限额、活动摘要、图表、未公开价格提示及脚注完整，没有旧版多余空白和端点接缝。
- Antigravity 未运行时中文不可用卡片为 250 × 101 点；z.ai 缺 key 明确显示需要配置。
- 搜索 codex 后仅保留匹配账户，展开配置保持；中文“配置/收起”无英文遗漏。之前切页回顶已实际检查；最终配置使用独立 APPDATA，USERPROFILE 保留真实位置以读取 Codex 本机登录/账本。
  最初隔离 USERPROFILE 导致测试窗口提示登录，已纠正；未复制或修改真实 Codex 登录凭据。
- 本机 localhost 网关 fixture `/v1/usage` 读到假密钥与配额；日志不含凭据。
  `work/ui-acceptance/requests.jsonl`。测试服务已停止。

没有将浅色单屏快照称为所有 DPI/主题已验收；真实 z.ai/智谱统计端点仍未用真实 key 验证。
本轮没有安装到正式目录或线上发布。用户在验收期间手动调整了测试窗口设置；这些选项保留，
只恢复为了截图临时修改的悬浮位置。`work/` 保持未跟踪。

## 后续验收

当前用户要求的视觉改进已实现，并更新本地包；如要正式发布，下一项是扩大主题/DPI 和实际 z.ai/智谱统计验收。
尚无真实 key 的成功证据，不能把 fixture 成功当作账户接口成功。推送与线上 Release 仍需按用户后续要求执行。

## 仍未移植/保留差距

- 10 个 provider：Kiro、Ollama Cloud、Grok Bot、Volcengine、Devin、Alibaba Token Plan、
  Gemini、JetBrains AI、Windsurf、Nous Portal。
- 浏览器 localStorage（Windsurf 及 Devin 第三路）、Codex reset credits、window starter、
  完整 55 个本机 token reader、附加账号登录、52 个新增 provider 的完整位图图标。
- Kind 中 daily/credits/messages/topUp/sharedCredits 仍映射为 other/spend；
  noPlan 仍使用 NoLimitsReported；bonus pack 仅到期时间、没有到期金额。
- macOS 专属刘海/Liquid Glass/Sparkle 等不属于 Windows实现。

上游参考使用本地 `upstream/main` 的 Swift 源码（`git show`），没有把它的在线未来更新当作已核验。
