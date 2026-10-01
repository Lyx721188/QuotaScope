# 交接文档：同步上游 Pulse v1.6.1 功能到 QuotaScope Windows

> 写于 2026-10-01。本文档是一次进行中的大型移植工作的完整交接，供下一个会话/代理继续。
> **当前工作区状态：所有改动均未提交**（在 `main` 分支上），
> `cargo test --workspace` 全绿（65 + 31 + 1 个测试），`cargo fmt` 已跑。

> **2026-10-01 续接进度（第二轮，本段为准）**：**M5 全部完成**——28 个剩余 profiled provider（25 个 apiKey 型 + Bifrost/LLMProxy/LiteLLM 三个 keyAndAddress 型）已由 6 波并行子代理移植并注册，`is_ported_to_windows`/`reports_spendable_balance`/`uses_api_key`（补了缺失的 Aixy）已同步翻转，设置页 `provider_row` 的地址输入框走 `needs_server_address()` 自动生效。**用量估计链（上游 a3415cc + token spend 账本的最小子集）已完成**：`ledger.rs`（TokenTally/TokenCost/Slot/LedgerDay/UsageLedger + Claude Code/Codex JSONL 解析器 + `ledger-4-<raw>.json` 文件缓存 + 5 分钟内存缓存）、`model_prices.rs`（models.dev 价目表，12 个第一方 provider + 3 个命名空间 vendor、别名链、24h 刷新/5min 重试、`model-prices-4.json`）、`estimate.rs`（BudgetEstimator 完整移植 + `window_text` 卡片行）。真机验证：Codex 111 天 / 8.75 亿 token / $428.42，模型显示名正确解析（跑 `cargo test -p quotascope-core live_ledger_smoke -- --ignored --nocapture`）。卡片估算行经 `settings.reads_token_spend`（默认关，设置页 General→Token spend 区块）门控，`app.rs` 后台线程每 5 分钟算一次经 `AppMsg::Estimates` 回流。`cargo test --workspace` 321 + 31 + 1 全绿。剩余：M6 浏览器会话 cookie 型 provider、M7 UI 收尾（托盘 dashboard、设置重组、per-account 详细卡——z.ai/智谱统计历史依赖它）、M8 文档/版本。已知小差距见第 8 节。

---

## 1. 任务背景

- **本项目**：QuotaScope（原 Pulse for Windows）—— macOS 应用 [Pulse](https://github.com/qunqin24/Pulse) 的 Windows 原生移植版。Rust 编写，两个 crate：
  - `windows/quotascope-core`：平台无关数据层（模型、provider 路由、缓存、告警、`--json`）
  - `windows/quotascope-win`：Windows 壳（Win32 + Direct2D/DirectComposition 悬浮条、托盘；**WinUI 3**（`windows-reactor` 0.100）设置窗口，自包含 Windows App SDK 运行时）
- **上游**：git remote `upstream` = `https://github.com/qunqin24/Pulse.git`（macOS Swift，SwiftPM 包，源码在 `Sources/Pulse/`）。
- **任务**：上游已从 v1.1.1（分叉点 commit `a4232a7`）推进到 **v1.6.1**，共 235 个提交。用户要求把这些新功能**全部移植**到 Rust/WinUI 原生实现。
- 看上游代码不用 checkout：`git show upstream/main:<path>`、`git ls-tree -r upstream/main --name-only`。

## 2. 上游功能清单（按可移植性分类）

### 2.1 已移植完成（本次会话，M1–M3）
见第 4 节。

### 2.2 新 provider（移植的主体工作）
上游 v1.6.1 共 77 个内置 provider + extension 类型。本地原有 17 个（14 个可取数）。新增：

**手写（Swift 文件在 `Sources/Pulse/Providers/`）**：
| id | 文件 | 凭据 | Windows 可移植性 |
|---|---|---|---|
| kiro | KiroUsageService.swift + KiroACPClient.swift | Kiro CLI 子进程 ACP（JSON-RPC over stdio） | 可移植（std::process + ndjson） |
| devin | DevinUsageService.swift | 三路由：vscdb SQLite 快照 / `https://app.devin.ai/api/<org>/billing/quota/usage` Bearer / Chromium localStorage | 前两路可移植（rusqlite 已有）；localStorage 待 M6 |
| xiaomiMiMo | XiaomiMiMoUsageService.swift | 浏览器 cookie（platform.xiaomimimo.com） | 待 M6 |
| sub2api | Sub2APIUsageService.swift | key + **自填地址**（GatewayAddress），`GET {addr}/v1/usage`，四种回复形态 | ✅ gateway.rs 已备好 |
| newAPI | NewAPIUsageService.swift | key + 地址，三条并发请求（subscription/usage/status） | ✅ |
| v2ex | V2EXUsageService.swift | PAT，`GET https://edge.v2ex.com/api/v2/chat/quota` | ✅ 纯 API key |
| qoder | QoderUsageService.swift | 浏览器 cookie，双站（qoder.com/.cn），`GET https://<host>/api/v2/me/usages/big_model_credits`（personal + shared 两个 ring，不求和；bonus pack 有 nextExpiry） | 待 M6 |
| stepFun | StepFunUsageService.swift | 浏览器 cookie，双站（platform.stepfun.com/.ai），`GET <host>/api/step.openapi.devcenter.Dashboard/<method>` | 待 M6 |

**Profiled 52 个**（Swift 文件在 `Sources/Pulse/Providers/Profiled/`，每个 = 元数据 + fetch 闭包；Rust 侧等价物 = 一个模块 + 注册表，不是数据驱动）。
按凭据类型（已逐一核实）：

- **apiKey（25 个，全部可直接移植）**：Aixy, AlibabaCodingPlan, Amp, AtlasCloud, Chutes, ClawRouter, ClinePass, Codebuff, DeepInfra, DevPass, ElevenLabs, Factory, GitKraken, HuggingFace, Hyper, IBMBob, KiloCode, Moonshot, Neuralwatt, OpenAIPlatform, Poe, Synthetic, V0, Venice, VercelAIGateway, Warp, XAIAPI, XKiro, ZenMux
- **keyAndAddress（3 个，自建网关：key + 用户填的地址）**：Bifrost, LLMProxy（LLM API Key Proxy）, LiteLLM
- **sessionCookie（15 个，待 M6 浏览器会话基础设施）**：Abacus, Augment, LongCat, Manus, Mistral（`ory_session_*` 前缀匹配）, NotionAI, Perplexity, QwenCloud, RaycastAI, Replicate, Sakana, T3Chat, TypeSafe, Zed, ZoomMate
- **browserStorage（1 个）**：Windsurf（Chromium localStorage，待 M6 或搁置）
- **localLogin（4 个，读本机 CLI/app 登录文件）**：AlibabaTokenPlan（百炼 CLI）, Gemini（`~/.gemini` OAuth creds）, JetBrainsAI（IDE 本地文件）, NousPortal（Hermes Agent）

### 2.3 其余功能（多数已移植或已评估）
- ✅ 已移植：Grok 100% 判 spent；变红阈值；窗口时钟方向；隐藏托盘图标；二次启动打开设置；window nextExpiry；Extensions 体系；balance ring 通用化；GatewayAddress
- ⬜ 未移植、评估后搁置（见第 7 节）：Codex reset credits（需 `codex app-server` JSON-RPC 子进程）、window starter（重置后自动点火，发 CLI 消息）、token spend 体系（55 个本机 reader，macOS 路径为主）、z.ai/智谱服务器端用量历史（`GET {host}/api/monitor/usage/model-usage`，host: z.ai→`https://api.z.ai`、Zhipu→`https://open.bigmodel.cn`，Bearer key，30 天）、菜单栏 dashboard（Windows 对应物=托盘菜单增强，M7）、per-account 详细卡
- ❌ macOS 专属，不移植：刘海停靠、Liquid Glass、Icon Composer 图标、Sparkle 更新器、菜单栏原生 NSMenu 交互

## 3. 关键架构知识（续接必读）

### 3.1 添加一个 provider 的完整清单（core 侧）
1. `model.rs`：`Provider` 枚举加变体（serde camelCase；特殊名用 `#[serde(rename=...)]`）+ `ALL_PROVIDERS` 数组 + `display_name()`（产品名不翻译）+ `monogram()`（两个字母防撞）+ `is_ported_to_windows()` + 按需 `uses_api_key()` / `reports_spendable_balance()` / `can_report_without_setup()`（发现路径存在性）/ `windows_gap()`（不可移植时设置页文案）
2. `localization.rs`：`provider_raw()` 加映射（**raw 值是 settings/JSON/keys.dat 的主键**）+ 新 UI 字符串 Entry（格式 `Entry { key, en, zh }`，key=英文原文）
3. 新建 `providers/<name>.rs`：`struct XxxService { http: Arc<HttpClient> }`，实现 `ProviderService` 三件套：`provider()` / `origin_token()`（--json 的 source token）/ `fetch(&KeyRing) -> ProviderUsage`。**解析写成 pub 纯函数**便于测试
4. `providers/mod.rs`：`pub mod` + `Services::new()` 的 list 注册（双实例如 zai/minimax 用 `for_mainland()` 式构造器）
5. 测试：`tests/ported.rs` 或文件内 `#[cfg(test)]`，喂 `serde_json::json!` 字面量，不 mock 网络
6. ring 图标：`quotascope-win/src/assets.rs` 嵌入位图，缺省回退 monogram（52 个新 provider 暂用 monogram，图标是独立任务）

### 3.2 核心类型速查（`quotascope-core/src/model.rs`）
- `UsageWindow { id, kind, scope, used_fraction, window_seconds, resets_at: Option<i64>(epoch ms), reports_length, estimate, is_exhausted, next_expiry_ms, label }`
  - `reports_length=false` 的 window_seconds 只是排序键，**不可整除**；`Estimate::{PlanPrice, SinceTopUp, YourBudget}` 是唯二允许推断分母的标记；本项目铁律：**不发明用量百分比**
- `ProviderUsage { account: AccountKey, windows, observed_at, state: Live/Stale/Unavailable(Unavailability), plan, credit_balance(显示串), credit_remaining: Option<CreditAmount>, origin, is_cached }`
- `AccountKey { provider, slot }`，首账号 id=raw，附加账号 `raw#slot`；`Provider::Extension` 每个扩展是一个 account（`extension#<id>`），**不在 ALL_PROVIDERS 里**
- `usage_tint`：GOOD/CAUTION/WARNING/EXHAUSTED 四色 + `color_with(used, exhausted, warning_at)` + `warning_fraction(threshold_percent)`
- Unavailability 枚举 ~30 case，`message()` 走本地化；`alerts.rs::standing()` 把它们分为 Failure/Answered/Neutral 三类（新增 case 必须补这里，否则编译失败）

### 3.3 刷新引擎与共享设施
- `store.rs`：worker 线程 + `Command::{RefreshAll, RefreshAccount, SettingsChanged, Shutdown}` / `Update::{Readings, Alert}`；自适应间隔梯子 `[120..1800]s`，预付费类封顶 300s；`run_pass()` 每账号一个线程
- `run_pass` 现在的签名（M2/M3 后）：`(services, cache, alerts, state, keys, extensions: &Catalog, peaks: &mut Peaks, upd_tx, only)`；fetch 返回后先 `balance_ring::apply` 再 `cache.reconciled`
- `KeyRing`：启动时从 secrets 加载所有 `uses_api_key()` 的 key；`copilot_token` 单列
- `secrets.rs`：`%APPDATA%\QuotaScope\keys.dat`（DPAPI），接口只有 `key_for/set_key/install_dpapi`（DPAPI 函数指针由 win 层注入）
- `http.rs`：reqwest blocking + native-tls + 系统代理，20s 超时，两连加重试；`fetch_json/fetch_json_detailed`（404 单独区分）+ JSON 工具 `number/string_field/number_field/bool_field/array_field/object_field`（字符串数字通吃）
- `localization.rs`：`t(key)` 线性查表、`t_fmt(key, &[..])` 位置式 `{...}` 插值；未命中泄漏 key 本身；语言 `auto|en|zh` 全局 AtomicU8
- `report.rs`（`--json`）：只读缓存不取数不落盘；`WindowReport` 已含 `nextExpiryAt`；**kind/scope/source 一律不翻译**是契约（有测试）
- 缓存 `%APPDATA%\QuotaScope\last-readings.json`；只存 Live，读出即 Stale，过 reset 丢弃，24h 作废

### 3.4 WinUI 3（settings_app.rs，windows-reactor 0.100）模式与坑
- 声明式组件：`Component { create/update/view }` + `Message` 枚举 + `context.callback/message`；`SettingsHost` 在 app 侧，`Shared` 快照 + generation 计数器驱动 250ms poller
- **坑 1**：`.children((...))` 元组最多 ~16 个元素，多了包一层内层 `StackPanel`；`Vec<View>` 不实现 `IntoViews`，动态列表用 `.keyed_children(iter of (key, View))`
- **坑 2**：builder 方法返回 `TextBlock` 等，放进 `Vec<View>`/`children` 要 `.into()`
- **坑 3**：`windows` crate 的 `WAIT_OBJECT_0` 在 `Win32::Foundation` 不在 Threading
- 加设置项模式：`ToggleKey`/`ChoiceKey` 加 case → `apply_toggle`/`apply_choice` 写 `settings::mutate` → `general_view()` 加 `self.toggle/self.choice` → `AppSettings` 加字段（serde default 保证旧配置兼容）

### 3.5 上游 ProfileHTTP 语义（移植 profiled provider 时照抄）
重定向**一律不跟随**（防手动 Authorization/Cookie 头泄漏）；2xx 成功；401/403/**3xx** → 凭据被拒；429 → 限流；其余 → serverError。凭据为 `.apiKey(optional: false)` 时无 key 返回 `ApiKeyMissing`。

## 4. 本次会话已完成的改动（全部未提交）

### M1 核心小功能
- **Grok 100% spent**（上游 9f3b678）：`providers/grok.rs::pool_window` 设 `is_exhausted = percent >= 100.0`；+2 测试
- **变红阈值**（a5508a7 的 Windows 映射）：`settings.warning_threshold`（60–90，默认 75）；`usage_tint::color_with/warning_fraction`；卡片行 "X% Used" 文字超阈值变 WARNING 色（spent 仍 EXHAUSTED）；设置页 General 加 "Turn red past" 选择。**Windows ring 刻意保持系统强调色**（rings.rs 设计注释），故映射到卡片文字
- **窗口时钟方向**（084e0b2）：`settings.window_clock_direction`（elapsed|remaining）；`UsageWindow::window_clock_fraction(remaining, now)`；panel.rs 按设置算弧分数；设置页 "Clock shows" 选择；+2 测试
- **隐藏托盘图标**（d45fbaa）：`settings.hides_tray_icon`；`TrayIcon::set_hidden/shows_icon`（NIM_ADD/DELETE，气球随图标隐藏）；返回路径=再次运行 exe（触发二次启动开设置）；设置页开关+说明
- **二次启动打开设置**（0943ef1）：命名事件 `QuotaScope.Windows.OpenSettings`；`winutil::signal_open_settings/listen_for_open_settings`；main.rs 单实例失败→发信号退出；`AppMsg::OpenSettings` → `settings.show()`
- **nextExpiry**（Warp/Qoder 加量包用）：`UsageWindow.next_expiry_ms` + 卡片 reset 行追加 "expires …"（zh: "{time} 到期"）+ `--json` 的 `nextExpiryAt`
- `--json` 契约测试兼容；`tests/ported.rs` +2

### M2 Extensions 扩展体系（上游 PulseExtension.swift 完整移植）
- 新文件 `quotascope-core/src/extension.rs`（~650 行含测试）：
  - 目录 `%APPDATA%\QuotaScope\Extensions\<folder>\`，manifest **`quotascope-extension.json`**（schemaVersion=1，id 规则 `[a-z0-9.-_]{1,64}` 且字母数字开头、name ≤60、executable 解析后必须在文件夹内、timeout 夹在 1–60s 默认 20）
  - `scan()/scan_in()` 按名序扫描，8 种拒绝原因（NoManifest…DuplicateId）带本地化文案；**读 manifest 绝不运行程序**
  - 运行：`env_clear` + 白名单环境（USERPROFILE/TEMP/代理变量等）+ `QUOTASCOPE_EXTENSION_ID`/`QUOTASCOPE_EXTENSION_SCHEMA=1`（对应上游 PULSE_*，品牌改名是有意的）；stdout 封顶 256KiB；超时 kill（40ms 轮询 try_wait）；读线程防管道死锁
  - 报告解析：status ok/signedOut/unreachable/rateLimited/serverError/noLimits → 对应 Unavailability；limit 有 `usedPercent` 或 `used/limit` 才成 window（id=`extension.<id|index>` 防重、label ≤60）；balance 是纯钱（ISO 三字母大写校验，可为负）
  - 10 个测试（含临时目录扫描测试）
- 接线：`Provider::Extension`（serde "extension"，不在 ALL_PROVIDERS，`from_raw` 有 fallback）；`Unavailability::{ExtensionMissing, ExtensionTimedOut, ExtensionFailed, ExtensionSignedOut}` + `alerts.rs::standing()` 归类（missing=Neutral，其余 Failure）；`settings.extension_names`（account id → manifest 名）+ `ordered_enabled()` 追加启用的扩展账号（按名字排序）；store worker 持 `extension::Catalog`，SettingsChanged 重扫，fetch 线程里 `extension::fetch(&extension)`；panel `RailEntry` 标题用 manifest 名（`account_title`），`placeholder(provider, &settings)` 签名已改；`report.rs::settings_label` 用 manifest 名；设置页 Accounts 底部新增 Extensions 区块（列表开关 + 拒绝原因 + "Look again" 按钮 + 目录路径）
- 上游 5e6c3dd 的"extension 纯余额也拿 balance ring"已由 M3 的 `balance_ring::apply` 覆盖（extensions 在适用范围内）

### M3 两个使能件
- **`gateway.rs`**（GatewayAddress.swift 完整移植，6 测试）：`url_from(typed, path, trimming)` —— 无 scheme 默认 **https**；http 仅 loopback/RFC1918/link-local/IPv6 ULA/`.local`，公网拒绝而非升级；禁 userinfo/fragment/query；`root_of` 去尾斜杠+按最长优先剥 `/v1` 类后缀；`is_usable` 给设置页 Save 用。依赖：workspace 加了 `url = "2"`（reqwest 已传递依赖，锁在 2.5.8）
- **`balance_ring.rs`**（BalanceRing.swift + DeepSeekBalanceBasis.swift 移植，6 测试）：
  - `Basis::{SinceTopUp, Budget, BalanceOnly}`；`basis_for(account)` 读 `settings.balance_bases/balance_budgets`（per-account），DeepSeek 主账号回退旧全局键 `deepseek_basis/deepseek_budget`；`window(balance, basis, budget, peak)` 产出唯一 `Kind::Balance` 窗口（无长度无 reset，**永不 spent**，fraction 封顶 0.99）；`apply(reading, peaks)` 只对 Live+无 windows+有 credit_remaining 的读数生效（排除 DeepSeek/CommandCode 各自的专用路径；extensions 适用）
  - `Peaks`：`balance-baseline.json`，键 `"{account_id}|{currency}"`，advance 只升不降（DeepSeek 保留自己的 `deepseek-baseline.json` 和旧 Baseline，老用户数据不动）
  - `deepseek.rs::windows_for` 委托给 `balance_ring::window`（保留签名与 is_available 语义，测试不变）；store 的 `run_pass` 接入 `Peaks` + apply
- `settings.rs` 新字段：`warning_threshold / window_clock_direction / hides_tray_icon / extension_names / balance_bases / balance_budgets`（全 serde default，旧 settings.json 兼容）

## 5. 下一步工作（按优先级）

### M4：Provider 枚举与共享文件一次性扩充（M5 的前置）✅
一次编辑所有共享文件，把 M5 全部 ~30 个 provider 的变体加进去，之后子代理只写各自文件：
- `model.rs`：枚举 + ALL_PROVIDERS + display_name + monogram + is_ported + uses_api_key（25 个 apiKey 型）+ 新谓词 `needs_server_address()`（Bifrost/LLMProxy/LiteLLM/sub2api/newAPI + 其他 keyAndAddress 型）
- `localization.rs::provider_raw`：按上游 raw id（见 2.2 表）
- `settings.rs`：加 `server_addresses: HashMap<String,String>`（account id → 网关地址，对应上游 AppSettings.serverAddresses）
- `providers/mod.rs`：KeyRing 加 `addresses: HashMap<String,String>`（从 settings 加载）+ `address_for(provider)`；Services::new 注册全部
- `settings_app.rs`：`provider_row()` 给 needs_server_address 的 provider 加地址输入框（TextBox + is_usable 校验 + Save）；API-key 分支已是默认

### M5：apiKey/keyAndAddress provider 分波移植（子代理并行）⏳
**策略**：每波 4–6 个 provider，一个子代理一波；给子代理的 prompt 必须包含：上游 Swift 文件路径（用 `git show upstream/main:Sources/Pulse/Providers/Profiled/XxxUsageService.swift` 读）、`providers/deepseek.rs` 作为模板、第 3.1 节清单、3.5 节 HTTP 语义、`gateway.rs` 用法（keyAndAddress 型）。**子代理只写自己的 `<name>.rs` + 测试**（共享文件 M4 已改好），并跑 `cargo test -p quotascope-core` 验证。完成后主线统一 `cargo fmt` + 全量测试。
- 波次建议：①V2EX+Moonshot+HuggingFace+Venice+DeepInfra（最简单，找手感）②xAI API+OpenAIPlatform+Perplexity…③keyAndAddress 三兄弟+Sub2API+NewAPI ④其余
- 注意上游各文件的 nextExpiry（bonus pack）、plan 名、`reports_length` 语义、Estimate 标记要如实带上

已完成：V2EX、Moonshot、Hugging Face、Venice、DeepInfra、sub2api、New API。剩余 profiled provider 仍按上游文件逐个移植。

### M6：浏览器会话凭据基础设施 + cookie 型 provider（15+4 个）
- 新模块（core 或 win 注入式）：读 Chromium 系（Chrome/Edge）`User Data\<profile>\Network\Cookies` SQLite：`encrypted_value` = `"v10"/"v11"` 前缀 + AES-256-GCM，key 在 `Local State` 的 `os_crypt.encrypted_key`（DPAPI 解开）——需要加 `aes-gcm` crate（DPAPI 复用 secrets.rs 注入机制）；Firefox `cookies.sqlite` 是明文 SQLite 可直接读
- **已知风险（要写进文档）**：Chrome 127+ 的 App-Bound Encryption 会让部分 cookie 解不开（Edge 受影响较小）——解不开就报 sessionMissing，诚实降级，符合项目"不编造"原则
- localStorage（Windsurf/Devin 第三路）需要 leveldb 读取器（`rusty-leveldb` 纯 Rust），建议**先搁置**，只做 cookie 型 15 个 + Devin/Qoder/StepFun/Xiaomi/Kiro 的非 localStorage 路由
- Kiro：std::process 起 `kiro` CLI，ndjson JSON-RPC：initialize → 等 setConnection → `_kiro/account/getUsage`，20s 超时，stderr 排空

### M7：UI 收尾
- 托盘 dashboard（上游 f45f143 菜单栏 dashboard 的 Windows 对应物）：托盘菜单顶部加逐账号 "名字 42%" 禁用菜单项（数据来自 App.readings，不新取数）；可加 "Open usage page" 子菜单（上游 `UsagePages.swift` 有 18 个服务商的用量页 URL 表）
- 设置页重组（上游 0c350ed/def983f/5465fe5）：订阅型与 API 型分组、key 输入框可显示明文（PasswordBox 换 TextBox 或加眼睛按钮）、Accounts 页搜索
- 每账户 balance basis 选择器：API 型 provider 行加 "Ring measures: Since top-up / Balance only / My budget"（写 `settings.balance_bases/balance_budgets`）+ Warn below（`low_balance_alerts` 已存在，接 UI）
- per-account 详细卡开关（1b72ff7）与卡片增强视余量

### M8：文档 + 版本 + 发布
- `Docs/extensions.md`（从上游 Docs/extensions.md 改写：manifest 名、环境变量、目录、PATH 语义）；`Docs/providers/` 每个新 provider 一页（上游 Docs/providers/ 同名 md 可改写）；`Docs/providers/README.md` 全表更新；README 特性/服务商清单更新
- `windows/Cargo.toml` workspace version 1.1.1 → **建议 1.2.0**（上游对齐 1.6.x 没有意义，Windows 版本独立）；CHANGELOG.md 加条目（Windows 版独立写）
- `cargo fmt --all -- --check && cargo test --workspace && cargo build --release`；真机跑 `quotascope.exe` 看 rail/托盘/设置（可用 presentations:visual-judge 对渲染截图做验收）
- 全部完成后**按用户指示决定是否提交**（本次会话未提交任何东西）

## 6. 本次会话的经验教训（避免重复踩坑）
- windows-reactor 的 `children` 元组上限、`IntoViews` 只认数组/元组、动态列表必须 `keyed_children`
- `UsageWindow` 加字段时 `new()` 构造器要同步（有大量调用点）
- `alerts.rs::standing()` 是 Unavailability 的穷尽 match，加 case 必改
- `settings::mutate` 每次落盘——高频路径别用它做轮询写
- 事件常量：`WAIT_OBJECT_0` 在 `Win32::Foundation`
- 测试里 `let window = window(...)` 会自遮蔽，用 `w`
- 中文注释/文案：项目所有用户可见字符串走 localization 表（en/zh 双语）；代码注释全英文、陈述"为什么"

## 8. 第二轮遗留的小差距（低优先级）
1. `Kind` 枚举缺上游 `daily`/`credits`/`messages`/`topUp`/`sharedCredits` —— 各波按 crate 惯例映射为 `Other(seconds)`（stated 长度保留 `reports_length=true`）或 `Spend`，`--json` kind token 因此与上游不完全一致；加 Kind 需同步 `localized_name` 与报告契约测试。
2. `Unavailability` 缺 `noPlan`（Chutes/IBM Bob/Alibaba/DevPass 的已知无订阅态）——现都落在 `NoLimitsReported`；加 case 必须同步 `alerts.rs::standing()`。
3. Warp 的 bonus pack 只带时间（`next_expiry_ms`），上游 `Expiry` 还合计当天过期的数额——本地无字段。
4. Codex 内部模型（`codex-auto-review`、`omen-alpha` 等）无公开价目，账本如实计入未定价——符合"不编造"铁律，非缺陷。
5. `provides_history()` 仍是 false 占位——z.ai/智谱统计历史 + 详细卡 UI（M7）落地时改。

## 7. 明确搁置项（与用户确认后再做）
1. **Codex reset credits**（bbf1507+d547e08）：需移植 `codex app-server` JSON-RPC 子进程客户端（initialize→`account/rateLimits/read`→`rateLimitResetCredits{availableCount|credits[].expiresAt}`），上游是后台增强不阻塞 ring，默认关。wham/usage HTTP 端点**不含**此数据（已核实）
2. **Window starter**（0688fe4）：窗口重置后向 CLI 发一条消息让 5h 窗口立即开始（opt-in，带确认开关和时段限制）——`Docs/window-starter.md` + `WindowStarter.swift`/`WindowPrimer.swift`
3. **Token spend 体系**（ae7ce62）：55 个 agent reader 的本机记账+定价（models.dev）+图表，是独立大子系统；Claude/Codex 的 transcript 在 Windows 路径相同（`~/.claude`、`~/.codex`），若做先做这两个
4. **z.ai/智谱服务器端 30 天历史**（上游 ZaiUsageService statistics 段）：数据源明确（见 2.3），卡片加"近期用量"区块，中等工作量
5. **52 个 provider 的 ring 图标**：目前全部 monogram 回退；上游 icon 是 macOS 资源（SVG），需逐个转位图嵌入 assets.rs
6. **localStorage 型路由**（Windsurf 全部、Devin 第三路）：需 leveldb 读取器
7. **附加账号（added accounts）**：Windows 版的 `fetch_account` 仍是 None 占位，上游 1.1.1 时也未完成对等，非本次范围
