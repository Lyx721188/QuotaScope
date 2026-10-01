# QuotaScope Windows 1.2 交接

更新：2026-10-01 15:27（Asia/Shanghai）。工作区 `D:\Projects\QuotaScope`，分支 `main`。

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
- 本轮：M7 功能与 M8 Windows 文档/版本/本地包。没有推送、打 tag 或创建线上 Release。

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

## 验证证据

最终源码检查均退出 0：

- `cargo fmt --all -- --check`
- `cargo test --workspace`：476 core + 3 M7 integration + 31 existing integration + 4 Windows tests，
  **514 passed，2 ignored**。忽略的真实环境测试没有运行。
- `cargo clippy --workspace --all-targets`：通过但保留仓库已有风格警告；不是 `-D warnings` 零警告验收。
- `cargo build --release`：Windows 1.2.0，本轮最终构建约 1m22s。
- `git diff --check`：通过。
- 新文档 71 个本地链接验证存在；77 provider 表按 model/localization/registry核对，67 可取数路由、10 未移植。

日志：`work/m7-tests.log`、`work/m7-clippy.log`、`work/m7-release-build.log`。

本地包：`work/quotascope-windows-1.2.0.zip`（26,401,192 bytes），
展开目录 `work/quotascope-windows-1.2.0/` 包含 exe、DLL、PRI 和语言资源。
Windows FileVersion 为 1.2.0。

`work/validate-m7.ps1` 使用临时 APPDATA/USERPROFILE、禁用账户配置做进程冒烟，未改实际账户设置：

- 打包目录 exe 的 `--json` 退出 0，输出含 generatedAt/accounts；配置 SHA256不变且未写新的 app-data 文件。
- 打包目录 exe 的 `--settings` 存活 20 秒，然后只停止该测试进程。
- 运行时/包元数据与哈希：`work/m7-validation.json`，时间 2026-10-01T15:26:48+08:00。
- exe SHA256：`C1B2E39E30AFD08145BBA8BBC10B76868543EB4949D854442C0371F81D545739`。
- zip SHA256：`5FD6C4712ED1CC4E98862FFB8D1E0321A641ABBB95736B7C80DC1F2EB1A93A0C`。

**证据边界**：启动存活不证明交互布局正确。没有用截图完成托盘/设置/悬停卡的交互验收，
也没有通过真实 z.ai/智谱账户验证统计端点。所有新统计边界当前是 fixture 验证。
本轮没有安装/替换用户现用程序，也没有线上发布。`work/` 包含接管前引用和本轮本地产物，保持未跟踪。

## 唯一下一项

**对本地 1.2.0 包做交互 UI 与实际账户验收**，依次检查：

1. 托盘右键摘要、stale/error、已知用量页链接。
2. Accounts 搜索/分组、输入 key 与 gateway address 后 Save，再重开确认（不要输出 key）。
3. 深浅色及不同 DPI 下的明文切换、预算验证与低余额设置。
4. 开启某账户详细卡；打开 Token spend 后看 Claude/Codex history/估值，再关闭确认消失。
5. 真实 z.ai/智谱的 missing key/成功/失败显示；历史返回时保持鼠标不动，确认卡片重新测量且不裁切。

退出产物：对应交互截图/过程记录和端点成功/失败的脱敏证据。
通过后再根据用户要求决定推送/发布；本地构建与 GitHub CI/Release 是分开的验收。

## 仍未移植/保留差距

- 10 个 provider：Kiro、Ollama Cloud、Grok Bot、Volcengine、Devin、Alibaba Token Plan、
  Gemini、JetBrains AI、Windsurf、Nous Portal。
- 浏览器 localStorage（Windsurf 及 Devin 第三路）、Codex reset credits、window starter、
  完整 55 个本机 token reader、附加账号登录、52 个新增 provider 的完整位图图标。
- Kind 中 daily/credits/messages/topUp/sharedCredits 仍映射为 other/spend；
  noPlan 仍使用 NoLimitsReported；bonus pack 仅到期时间、没有到期金额。
- macOS 专属刘海/Liquid Glass/Sparkle 等不属于 Windows实现。

上游参考使用本地 `upstream/main` 的 Swift 源码（`git show`），没有把它的在线未来更新当作已核验。
