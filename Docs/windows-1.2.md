# QuotaScope Windows 1.2 操作与边界

本页按当前 Rust 实现编写。版本尚未发布；实现、解析测试、真实账号和交互界面验收是不同证据。
完整路由清单见 [Windows provider 表](providers/windows-ports.md)。

## 托盘与设置

右键托盘图标可以看到已启用账户的用量摘要。这里复用缓存/刷新引擎的读数，不额外发起请求。
旧读数会标记可能过时；失败读数显示原因，不补成 0%。“打开用量页”只列有已知页面的账户。

关闭设置会隐藏窗口，随后从托盘或再次启动程序打开时复用同一个 WinUI 宿主，
不会再次初始化 XAML；在托盘选择“退出”才结束宿主。发布流程对打包后的程序执行
打开、关闭、重开、重复启动和隐藏状态退出的真实进程回归。
本机可在 `windows/` 设置 `QUOTASCOPE_TEST_IDLE_SECONDS=310` 后运行
`cargo test -p quotascope-win --test settings_lifecycle -- --ignored --nocapture --test-threads=1`，
验证首次打开前和关闭后各空闲超过五分钟的路径。测试使用临时空配置，不读取真实账户。
具体触发条件与最小对照见 [设置生命周期取证](windows-settings-lifecycle.md)。

设置 → 账户：按订阅账户、API 账户分组，搜索名称或 raw id。API key 是一种鉴权方式，
不决定收费分组；例如 z.ai 仍是订阅账户。未移植的路由有说明。

凭据默认隐藏；“显示凭据明文”可查看当前输入。切页、搜索、保存、导入和重开设置会隐藏明文。
自建网关填写地址和 key 后保存。公网地址要求 HTTPS；本机/私有地址可用 HTTP，地址中不能含账号密码、查询或片段。

## 浏览器会话

18 个 provider 提供“从浏览器导入”；先在浏览器登录目标站点，再点击导入。
导入按默认浏览器优先查找支持的 Chrome、Edge、Firefox profile，只选择该 provider 需要的 cookie。
也可以直接粘贴 Cookie header。导入成功表示找到并保存了会话，刷新成功才证明接口接受它。

Chromium 明文 cookie 和可解开的 v10/v11 AES-GCM cookie 可读取；App-Bound/v20 值会跳过。
找不到匹配会话时明确报错。浏览器数据库锁定、cookie 过期或加密不兼容都可能导致无法读取。
Cookie 保存在 `%APPDATA%\QuotaScope\keys.dat`，使用当前 Windows 用户的 DPAPI 加密。

## 余额口径

提供余额的账户可选“自充值以来”“仅显示余额”“我的预算”。
这个选择只作用于无服务商自身限额的余额读数，不能替代服务商报告的配额。
自充值以来使用本机观察到的最高余额；我的预算使用填写的正数金额；余额币种与预算币种必须一致。
余额环有推算标签，不把预算的 100% 判作服务商耗尽。

“余额低于此数时提醒”留空关闭。金额必须为有限正数，还需要在通知页启用通知。
DeepSeek 旧的全局余额口径/预算仍兼容；明确保存账户设置后使用该账户的新值。

## 详细用量卡与 token 价值

账户行的“详细用量卡”按账户保存，默认关闭。开启后悬停用量环显示 plan、更新时间，
以及有实际长度和 reset 的窗口时钟。后台读数变化后会重新测量悬停卡高度。

Claude Code、Codex 和 Antigravity 的历史还需要常规 → Token spend → 读取 token 消耗。
开启后会扫描本机 Claude Code、Codex 的会话文件，或 Antigravity IDE 的会话数据库，
解析 token 计数、模型和时间；只读取 Antigravity 的用量元数据，不读取会话正文。
Antigravity 中含义未确认的计数会省略，部分缺少调用时间的记录按会话开始时间归日，
并在卡片上标明。模型价目下载自 `https://models.dev/api.json`。
Antigravity 记录只显示在账户详细卡，不纳入 Token spend 总览。
数值是公开 API 价格下的价值估算，不能当成订阅账单，也不能反推出服务商额度百分比。
没有公开价格的 token 单独标记；被删除或不可读的会话不会出现在本机历史中。

窗口下面还可以显示“价值推算 ≈$220 · 已用 ≈$57”：把本机这段时间记到的金额除以服务商报出的
已用百分比，服务商自己从不报这个数，所以它永远是推断而不是读数。Claude Code 和 Codex 只有
不带模型范围的窗口参与推算；带范围的窗口（只约束某一个模型的限额）一律不算，因为本机记录里
那段时间的金额是所有模型加在一起的，用它去除以单个模型的百分比会把结果抬高到荒谬的程度。
Antigravity 的额度分 Gemini 与 Claude and GPT 两组，卡上的窗口都带着组名，于是按组推算：
只把该组模型的金额算进这个窗口，归属看模型自身的名字（本机记录到的写法是
`gemini-3.8-flash-control`、`claude-sonnet-4-6` 这一类）；这段里出现一个名字说明不了归属的
模型时，整个窗口就不推算，而不是把说不清的那部分当成零。Antigravity 报的是七位小数的
`remainingFraction`，不是整数百分比，因此可以整除的下限比其他服务商低（0.2%，其他家 2%）。
任何推算还要满足：窗口已经开启、本机记录从窗口开启之前就开始了、这一段记到的金额不少于
$0.20——一条不满足就什么都不显示，宁可空着。

价目表在读取时已超过 24 小时才重新下载，离线继续使用本地缓存
（`%APPDATA%\QuotaScope\model-prices-4.json`），下载失败 5 分钟后允许重试。价格是基础档单价：
部分模型超长上下文加价，而会话记录只有 token 数量、没有每次请求的上下文占用，
所以不加价档也不打折。代理写出的模型名常常不是 models.dev 的 id，会按已确认的写法归一：
服务商显示名（`Gemini 3.7 Flash (High)`）、把厂商写进 id 的形式
（`deepseek/deepseek-v4-flash`）、客户端加在模型后面的通道后缀（Antigravity 的
`gemini-3.8-flash-control` 在本机会话库里与 `gemini-3.8-flash` 共用同一个 `model_enum`，
thinking token 按模型自身费率计费）。实验版本（`gemini-3.7-flash-exp-b`）和没有任何
服务商公开价目的 stealth 模型仍计入“无公开价格”，绝不用邻近模型的价格代替。
用 `cargo run -p quotascope-core --example spend-coverage` 可查看本机哪些写法命中了价格。

“Token spend”页可按今天、最近 7/30/90 天或全部记录汇总。页面提供来源、模型、分组、
排序和降序/升序控制，支持按代理或模型钻取，并显示分页表格、日柱状图和来源覆盖状态。
未公开价格的 token 不计入金额；金额是本机记录的估算，不是实际账单。目前分析
Claude Code、Codex、Qwen Code、Gemini CLI、Pi、Oh My Pi、OmO Native、Kimchi、Amp、
Droid、Prime Agent、OpenClaw、Mux、Junie、Augment、JCode、Gajae Code、Codebuff、
FX、Reasonix 和 LM Studio 的本机会话记录；更多本机来源在后续移植计划中。
Prime Agent 按 fork 血统合并副本，并把父会话聚合中已声明的子会话用量扣回；
OpenClaw 同时读取 SQLite 与保留的 JSONL 原件，同一事件只计一次，镜像记录与
zstd 压缩文件跳过。
Mux 的工作区快照按模型只计一次；Junie 只读取模型用量事件，正延迟按调用开始
时间记账；Augment 只计完成的回合，并取最后一个非空 token 用量作为回合总数。
JCode 按显式 schema 标记判断 cache 与 input 的关系，无法判断的 input 不按
新输入计价；Gajae Code 按条目 id 折叠重放，字节相同的镜像文件只读一次。
Codebuff 从同一消息的多份用量副本中逐字段取第一个非零值，不重复计数；
FX 按会话快照每个模型计一次。Freebuff 不持久化 token 计数，只有字符估算，
因此不作为数据源显示。Reasonix 的 `turn` 标记行不计入；LM Studio 只解析
日志中模型和时间都可定位的 usage 块。

z.ai/智谱详细卡使用对应账户服务器的近 30 天统计，与本机 Token spend 开关独立：
国际账号请求 `api.z.ai`，智谱账号请求 `open.bigmodel.cn`。只对启用且打开详细卡的账户读取。
统计覆盖各设备，但没有输入/输出/cache 拆分，因此仅显示 token，不显示费用。
未配置 key、请求失败、成功但无用量分别显示。小时桶汇总成日期，日期空隙保留零值。

历史和估算在工作线程计算，每五分钟更新；手动刷新/保存设置会失效历史快照。
请求返回时检查配置代次，旧 key 或已关闭账户的结果不会覆盖当前卡片。
`--json` 保持只读缓存契约，不扫描账本、不请求统计，也不导入浏览器会话。

## 界面与字体

账户页默认展示名称、状态和启用开关；点击“配置”展开凭据、刷新、详细用量卡和余额设置，
点击“收起”返回摘要。已启用账户排在对应分组前面，搜索仍可找到未启用及未移植账户。
常规页使用分区卡片，开关和下拉选项固定对齐；切换页面会回到顶部。

用量卡按上游 250 点宽度排版。详细活动区展示今天、7 天、30 天摘要和日柱状图，
API 价值估计明确标为估算；无公开价格和服务器 token 统计不伪装成费用。
进度条为单一圆角图形，避免拼接端点的深色小点。鼠标周围不再绘制强调色光晕。

“预计够用至重置”表示按当前周期自开始以来的平均消耗速度，额度预计不会在下次重置前用完。
“预计在重置前用尽”表示按该速度会提前用完；预计两小时内用完时显示约剩多少时间。
这是本机按用量与已过时间做的线性估算，突发重度使用或暂停使用会改变结果。

界面使用随包提供的 HarmonyOS Sans SC（常规、中等、粗体），无需安装系统字体。
解压后保留 exe 旁的整个 `Fonts/` 目录；许可和来源见 [第三方声明](../THIRD_PARTY_NOTICES.md)。
Windows 管理的标题栏及菜单遵循系统字体。

## 未覆盖功能

10 个未移植路由见 provider 表。Codex reset credits、window starter、完整的 55 种 token reader、
浏览器 localStorage 路由、附加账号登录和新 provider 的完整图标仍未实现。
Windows 不实现 macOS 刘海、Liquid Glass、Sparkle 等平台专有功能。
`Kind` 中部分上游类别仍映射为 `other`/`spend`，bonus pack 目前只有到期时间，没有到期金额。

本次历史端点的成功/失败边界通过固定样本验证；未把真实 z.ai/智谱账号取数或所有界面布局称为已验收。
