# 下一轮上游跟进计划

核对日期：2026-10-04（Asia/Shanghai）。状态：**N0/N1/N2/N3/N4a 本地已验证，改动已合入本地 main；下一项为 R1 发布前验收**。

建议顺序：对齐执行基线 → 性能与现有功能验收 → DeepSeek 官网历史 → Codex 本地异常线索 → 按样本补齐原生来源与登录。
当前唯一下一项是 **R1：main 的发布前验收**，任务与退出条件见[第 9 节](#9-main-合并后的下一阶段)。N4a 后重新检查得到 706 passed / 6 ignored，fmt、普通 Clippy、Release 构建与 JSON CLI 通过；DSH 真实压缩样本独立复算、Debug 隔离 UI/安装记录与当前二进制哈希一致。真实账号、多 DPI 人工交互、Release 桌面、最终 CI/发布及用户安装版升级分别保留验收边界。版本号在准备发布时确定。

原执行分支：`codex/upstream-followthrough-20261004`，由 `db92d3e` 创建；2026-10-04 按用户要求，将其 14 个已有提交及 N4a 提交 `40a5b5f` 快进合入 `main`。所有本地命名分支的提交均已被 main 包含，旧 worktree 保留。主目录是 `D:/Projects/QuotaScope`，Cargo 工作目录是其 `windows/` 子目录。
2026-10-04 用户要求设置目标并持续推进到 5h 额度限制；已设置持续工作目标，优先 N0/N1，每批保存当前状态与证据。不要将消耗额度本身作为产物。

后续 agent 先读本页、`Docs/upstream-implementation-status.md`、`Docs/windows-resource-update-verification.md`，然后查看实际 `git status` 与最近提交。本页记录固定基线，不把后来的远端变化自动算作已验收。

## 1. 本次确认的基线

| 对象 | 当前证据 | 对规划的影响 |
|---|---|---|
| 当前主目录 | `D:/Projects/QuotaScope` 的 `main` 已从 `d1f1f11` 快进到 `db92d3e`，与本次 fetch 后的 `origin/main` 相同；`windows/Cargo.toml` 为 1.3.1 | 同步完成，后续实现位于执行分支；保留原有忽略文件 |
| Windows 远端 | `origin/main` = `db92d3e9687d4db8d959c30ce5ee4a3e7b702c44`；最新稳定 Release 为 `windows-v1.3.1` | 本计划以此提交中的 Rust 实现为准 |
| Windows CI | 1.3.1 标签运行 `37131049528` 成功；测试、安装包构建、包内设置生命周期步骤均成功 | 是已发布提交的证据，不替代下一轮改动后的验证 |
| 上游稳定版 | Pulse `v1.7.1`，提交 `cf258d9` | 新增 DeepSeek 控制台与 Codex 异常迹象；另有面板布局修复 |
| 上游最新 main | `3696a65b428272aa25c3ba611de8df2536983515` | 在 1.7.1 之后新增减少扫描、查价和面板重复工作的优化 |
| Windows 上次大范围移植 | `Docs/upstream-implementation-status.md` 记录基线为 Pulse `b396306` | 需比较其后的实际差异，不从旧 P5 重新开始 |
| 本地旧交接 | 当前主目录没有 `HANDOFF.md`；`PORT_PLAN.md` 是被忽略的本地旧文档，在远端 main 中不存在 | 旧文档的“下一项 P5”不再作为执行依据 |

本次已 fetch 两个 remote、核对 GitHub 发布与 CI，并完成主目录快进与执行分支创建。本地基线检查的结果目录为 `windows/target/upstream-followthrough-20261004/`。当前安装版进程独立运行；代码同步不等于安装版升级。

## 2. 已有能力与实际缺口

1. Windows 1.3.0 已接入此前缺少的十个额度路由、附加账户、底部及横向停靠、长卡滚动、托盘概览、快捷键、代理、CLI/CSV/深链接等。1.3.1 又以专用 ACP 读取器完善 Kiro；这些不能再列为从零开发。
2. 用量估算已经实现，包含 5% 下限、额度读取时刻截止、边界桶比例和外部消耗保护。新一轮应做回归与差异核对，不重写另一套估算。
3. 已有流式读取、Codex 追加读取、可取消扫描、磁盘预算、五分钟快照和可选后台统计；性能工作应补足现有链路。
4. 54 个 Token 来源是目录规模，不等于全部原生支持。当前状态文档记录 34 项已知格式读取（含导出），20 项仅标准导入；N4a 已补齐 DSH Zstandard 解压，不增加来源目录数量。
5. Kiro 真实账户、其他新路由的逐账号验证，以及部分多 DPI 拖拽、快捷键冲突、真实代理、实际安装后的协议注册仍有验收缺口。
6. 保留现有排除：**不新增 Claude Code 跨文件回复去重**。这是当前项目状态文档明确记录的范围决定。macOS 原生外观和更新机制继续用 Windows 对应实现。

## 3. N0：建立可执行基线

N0a 已完成：主目录快进到 `db92d3e`，执行分支已创建。N0b 已完成：源码检查与三轮隔离流式 UI 基线。接续时先核对正在运行的 Cargo 进程与日志，不同时启动多份 Cargo 抢占同一 target。

- 在干净工作区将主目录快进到已核实的远端，或复用合适工作树；用 `codex/` 分支承载后续实现。保留本地被忽略的交接文件，不以 reset/clean 清理。
- 若仍以本次固定提交为基线，明确记录；若远端又有更新，先补做差异核对。
- 从 `windows/` 执行 fmt、CI 同款 Clippy、workspace tests 和 release build，记录退出码。现有 CI 没有启用 `-D warnings`，不得把其成功写成零告警。
- 建立统一性能样本：价格命中/别名/未计价模型，冷读/热读/追加/截断/删除日志，Token 页筛选和进出，设置关闭重开。
- 记录耗时、CPU、工作集/私有内存、缓存写次数和字节、扫描及聚合次数。上游 Swift 合成数据的倍数不能当 Windows 提升承诺。

验收与退出产物：固定 commit、可复现命令、基线结果与仍失败的项目清单。没有新基线，不进入性能实现。

### 3.1 基线命令与记录方式

在 PowerShell 7 中，从 `D:/Projects/QuotaScope/windows` 顺序执行；每条保留标准输出、错误输出、退出码和耗时。不要并发执行 Cargo 构建。

```powershell
Set-Location 'D:/Projects/QuotaScope/windows'
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked
cargo test --workspace --locked
cargo build --release --locked
```

当前执行器生成 `baseline-fmt.log`、`baseline-clippy.log`、`baseline-tests.log`、`baseline-release.log` 与 `baseline-results.json`。只有实际完成的检查才会出现在结果 JSON 中；缺项不能当成成功。测试总数以本次各个 `test result:` 之和为准，不能照抄 1.3.0 的 631 或 1.3.1 历史文档的 644。

性能基线使用已有 `quotascope-win/tests/streaming_ui_benchmark.ps1`，先 `cargo build --locked` 生成 Debug 程序，再运行：

```powershell
pwsh -NoProfile -File ./quotascope-win/tests/streaming_ui_benchmark.ps1 -Executable ./target/debug/quotascope.exe -Label before -Records 400000
```

该脚本创建独立合成 profile，覆盖完整读取、追加、进程重启后追加、改写；输出 `target/stream-before-result.json` 与 `target/stream-before-samples.json`。先将程序及同目录所需运行时文件保存为基线 payload，并记录 SHA-256，避免后续构建覆盖 before 程序。修改代码后用同样 Records、工具链、构建 profile 测 after，最后把两轮结果复制到本次验证目录。

脚本还不能证明查价、聚合缓存或所有来源的性能。另建合成查价/聚合样本，计时不包含生成 fixture，先断言结果再报告耗时。每种实现至少三次、报告中位数和原始记录，不承诺固定提速倍数。内存指标注明 WorkingSet 与 PrivateBytes 的区别；CPU 用进程 CPU 时间增量除以观察时长，不直接把累计 CPU 秒当百分比。

### 3.2 已有测试入口及限制

| 入口 | 用途 | 接续注意事项 |
|---|---|---|
| `quotascope-core/src/model_prices.rs` 的 tests | 别名、厂商命名空间、未知模型 | 保留全部现有优先级断言；新索引与旧算法做差分 |
| `quotascope-core/src/ledger.rs` 的 tests | 流式、追加、checkpoint、计数、估算 | 保留跨文件 Claude 不去重的范围；取消不得发布半成品 |
| `quotascope-core/src/scan.rs` 的 tests | 逐块取消、UTF-8、读取错误、线程作用域 | 新 reader/worker 应复用这套机制 |
| `quotascope-win/src/settings_app.rs` 的 tests | 隐藏设置释放快照、取消保留完整快照 | 同时回归新聚合任务的迟到结果拒绝 |
| `quotascope-win/tests/settings_lifecycle.rs` | 真实托盘开关、设置重开、二次启动、退出 | 需解锁桌面，默认 ignored；Debug 支持隔离单实例 |
| `quotascope-win/tests/spend_ui_smoke.ps1` | 扫描取消、离页、关闭、重开与中文 UI | 已按现有目录修正为 Codex 前完成 3 个来源、总计 54；增加来源时更新断言 |
| `quotascope-win/tests/maintenance_ui_smoke.ps1` | 存储预算、清理和脱敏诊断 | 阅读参数和合成 profile 设置后再运行 |
| `installer/test-installer.ps1` | 独立身份安装、生命周期、卸载 | 输入是真实完整 payload 与 Inno 编译器；隔离安装不能替代用户安装版升级验收 |

检查桌面前先保存正在运行的安装版路径、版本与哈希。`QUOTASCOPE_TEST_INSTANCE` 目前仅 Debug 构建读取；Release 包可能碰到当前安装版的单实例锁。不要因为 Release 测试退出就停止所有同名进程。优先隔离 Debug 测试；Release 包验收应安排不与安装版争用的桌面，并记录真实测试路径。

## 4. N1：性能收尾与现有功能验收（建议 1.3.2）

依赖 N0。依据上游 `3696a65`，按 N1a → N1b → N1c 顺序小批提交。

### N1a：消除重复查价与无变化写盘

已确认的 Windows 对应位置：

- `windows/quotascope-core/src/model_prices.rs`，`price_for` / `first_party`：大小写和别名回退多次遍历价格表。
- `windows/quotascope-core/src/ledger.rs`：重建结束设置 `cache.files = fresh` 后直接 `cache.save(provider)`；底层写盘没有内容未变化短路。

工作：为单份不可变价格表建立索引，缓存 `(model, vendor)` 的命中与未命中；更换价格表后丢弃旧索引。保留厂商优先级、别名规则和未计价语义。原始解析缓存仅在记录或 schema 等输入变化时持久化，价格更新只触发必要重算。

验收：与旧算法在固定样本上的 Token、价格、未计价集合完全一致；同一快照重复查价不再重复全表遍历；日志未变时对应解析缓存 mtime/写次数不变；追加、编辑、截断、删除以及旧缓存升级仍生效。

#### N1a-1：单份价格表的索引和命中/未命中缓存

按下面顺序实施，完成本卡后单独提交：

1. 在 `model_prices.rs` 添加借用不可变表的读取期 lookup（名称由实现决定），精确 key 查找保持快速；大小写索引只需构建一次。缓存 key 必须包含 model 与可选 vendor，显式记录未命中。
2. 完整保持现有优先级：第一方精确/大小写/别名全部先于 vendor 回退；vendor 只能命中其命名空间。大小写碰撞的选择要保持现有 `BTreeMap` 遍历顺序，不让 HashMap 随机选择改变价格。
3. 将 `ledger::priced` 等批量计价调用改为每份价格表复用一份 lookup，保留 `price_for` 的兼容入口。不要为每条记录新建整个索引。其他调用点先 `rg 'price_for\('` 查全后逐个判断。
4. 索引随本次读取结束释放；新价格表，包括由空表更新为有价表，必须创建新 lookup，不保留进程全局的未计价结论。
5. 增加有业务意义的差分样本：同名第一方/vendor、大小写碰撞、别名/服务臂、vendor A/B 隔离、真实零价格、未知模型及价格表替换。先用这些样本锁住结果，再测量循环查价。

定向检查：`cargo test -p quotascope-core model_prices --locked`，以及使用 `ledger::priced` 的计价/估算测试。退出产物包含 before/after 命中、未命中计数和耗时，不仅是“新函数测试通过”。

#### N1a-2：未变化的解析缓存不重写

入口：`ledger.rs` 的 `FileCache` / `CachedEntry` / `scan` / `read_entry` 与 `statistics_cache.rs` 的 `write`。当前 `scan` 会从旧 map 移除 entry，再建立 fresh map；不能在旧 map 已被消耗后简单比较它是否为空来判断变化。

建议先追踪实际 entry 变动或保存可比较的旧状态：文件新增/删除、stamp 或已解析字段/checkpoint/title/cwd 改变才需要保存。也可以比较规范序列化字节，但需测量序列化开销；不能为了少写磁盘而额外重读整份大缓存。

保留这些行为：

- 旧缓存没有 Codex checkpoint 时仍能加载；必要时重扫并持久化新的完整结果。
- 模型价格变化会重新计价，但不改变原始 Token entry，因此不应仅为重新定价改写解析缓存。
- 缓存已被磁盘预算淘汰或用户清理时，随后允许重新创建；`statistics_cache_limit_mb=0` 时遵守禁用持久化。
- 取消、读错误、写失败不能把新状态当成已经持久化，也不能更新“已保存”比较基准；取消不得覆盖前一份完整缓存。
- 文件删除确实从后续缓存和总计移除；失败的文件沿用或移除何种状态需与现有错误语义一致，不能顺便改成猜测值。

测试观察真实临时文件 mtime/内容，覆盖无变化复扫、仅价格更换、追加、改写、截断、删除、旧缓存、取消、写入失败、预算禁用与重新启用。针对文件系统时间精度，优先结合写次数或内容证据，不用极短 sleep 碰运气。

完成后运行 `cargo test -p quotascope-core ledger --locked`、`cargo test -p quotascope-core statistics_cache --locked`、格式检查。新日志和结果更新本次验证表，再单独提交。

### N1b：缓存聚合并移出界面构建路径

已确认 `windows/quotascope-win/src/settings_app.rs` 的 `spend_view` 直接调用 `snapshot.analyze(...)`。现有快照缓存没有消除这一步的重复聚合。

工作：在后台产生可取消、可复用的统计结果，按快照版本、日期/范围、agent/model 等输入隔离；排序分页尽量复用聚合结果。更新期间只展示语义相同的上一份结果；不同筛选不能短暂借用别的统计。对 `model_details.rs` 的近期计时读取先测量，再决定是否复用文件级解析结果。

验收：同一快照与筛选命中缓存，筛选不启动新的文件扫描；跨日、数据变更、价格更新按依赖失效；取消或关闭后迟到结果不会回填；连续切换和关闭重开不发生后台任务叠加、内存无界增长。

#### N1b 的状态与并发约束

入口：`spend.rs` 的 `Snapshot::analyze` / `Analysis::page`；`settings_app.rs` 的 `SettingsSnapshot`、`Shared::request_spend`、`release_spend`、`cancel_spend`、`SpendChoice`、`spend_view`；`spend_warmer.rs` 的 `snapshot` / `clear` / `tick`。

1. 先给可发布的原始快照一个稳定 revision。已有 `spend_generation` 用于使扫描任务失效，不能假定它与内容 revision 永远相同。后台 warmer 与前台取得同一快照时，应能复用内容 revision。
2. 建立聚合输入 key：snapshot revision、本地日期、Span、source、model、Group；排序与方向决定是否单独缓存。页码仅切片，不能重扫或重做整个聚合。价格已固化在 snapshot 中时，更新价格必须通过新快照 revision 传播。
3. 持有 `Arc<Snapshot>` 的后台任务计算 Analysis，采用有界结果缓存（先一份当前结果和少量可复用结果即可），分别维护任务 generation 与 Control。不要对 Snapshot 全量 hash 作为每次 UI 重建的 key。
4. 在遍历 sources/days/models 中检查取消。`scan` 的取消作用域是线程局部；新聚合 worker 不能只复用外层 Control 却没有接入检查点。
5. 发布前检查读取开关、页面/窗口存活、输入 key、任务 generation；换筛选/离页/关闭/取消时使旧结果失效。不要在持有全局 settings/snapshot mutex 时聚合，也不要让 worker 直接构建 WinUI View。
6. 同一 key 的安静刷新可显示旧完整 Analysis；输入 key 变化时显示加载或空态，不能把 agent A 的总计贴在 agent B 的筛选上。读取完成但结果为空也是可缓存成功状态。
7. 关闭设置释放前台聚合结果；后台统计只在现有显式开关开启时保留原始快照，不能恢复 1.2.3 那种无限保留页面控件的行为。

按“纯聚合缓存及状态测试 → 后台接线 → UI”三批完成。样本至少覆盖 A/B 筛选快速交替、日期跨日、快照替换、排序分页、空结果、取消途中、旧 worker 迟到、关闭后重开和后台关闭。

定向命令：`cargo test -p quotascope-core spend --locked`、`cargo test -p quotascope-core scan --locked`、`cargo test -p quotascope-win --bin quotascope --locked`；测试过滤名以新实现实际名字补写。UI 验收须核对屏幕总计与固定 fixture，记录扫描次数和聚合次数以证明复用。

### N1c：真实 Windows 交互与发布包验收

- 测量托盘静置、Token 页热切换、重复打开设置的 CPU/内存和延迟；仅在有实测收益时继续移植面板重绘优化，不照搬 SwiftUI 结构。
- 补测横向与底部停靠、长卡滚动、百分比/倒计时组合、多 DPI/多屏；上游 `60df396` 的布局问题只在 Windows 可复现时修复。
- 检查快捷键注册冲突是否需要可见反馈；核实开关、语言和取消状态。
- 最终提交重跑测试与包内托盘打开/关闭/重开/第二次启动/退出。升级安装验证应先备份用户配置，并单列实际二进制路径、版本和哈希。

退出产物：同机器同样本的前后性能报告、功能回归结果、剩余真实账号缺口、对应最终提交的 CI 和发布包验收记录。没有证据的收益不写进 Release。

N1c 短生命周期命令（在已解锁桌面执行）：

```powershell
cargo test -p quotascope-win --test settings_lifecycle --locked -- --ignored --nocapture --test-threads=1
pwsh -NoProfile -File ./quotascope-win/tests/spend_ui_smoke.ps1 -Executable ./target/debug/quotascope.exe
pwsh -NoProfile -File ./quotascope-win/tests/streaming_ui_benchmark.ps1 -Executable ./target/debug/quotascope.exe -Label after -Records 400000
```

长空闲验收设置 `QUOTASCOPE_TEST_IDLE_SECONDS=310`，并在 finally 中恢复此前值；测试中有两次空闲等待，耗时会超过 10 分钟。短回归已完成且没有新生命周期风险时不要反复重跑长等待。每次记录对应的程序哈希与源码 commit；本地产物和 CI 产物无需字节相同，但分别验收。

发布前检查 `.github/workflows/windows.yml` 的当前步骤、`installer/build-installer.ps1` 和包内 DLL/PRI/字体资源；只有准备发布时才更新版本和 changelog。源码/文档阶段不预先打 tag。用户安装版升级与远端发布分别记录，不能拿隔离安装测试代替二者。

## 5. N2：DeepSeek 官网历史与无 Key 余额（建议下一功能版）

依赖 N1。参考上游 `6423f0d`，同时纳入 `73b5d62` 的审查修复。当前 Windows `providers/deepseek.rs` 只有 API Key 余额路由。

实现范围：

- 复用现有有界 Chromium localStorage reader；在 DeepSeek 设置提供显式读取入口。控制台 session 与 API Key 分槽、DPAPI 保存，按账户隔离。
- API Key 成功时仍优先；仅对核实的错误类型提供控制台余额回退。过期认证与网络/HTTP 拒绝分开处理，不能见 403 就刷新或替换凭据。
- 读取最近 30 天账号级 Token、模型占比、缓存命中率和实际消费；金额按实际币种显示，多币种不相加。
- 先扩展历史模型的来源与币种，再接解析、缓存和 UI。控制台历史不并入本机日志总计，避免重复计算；日级聚合不生成猜测的小时分布。
- 保留完整缺失语义：没有费用不是 0 元；summary 未报告 `is_available` 时不能认定账户耗尽。凭据或币种变化使相关缓存失效。

验收：上游修复后的解析样本、无 Key/Key 拒绝/会话过期/服务失败、空值和异常数字、币种切换、时区与日界、凭据隔离、取消与缓存回归；最终与同一账号官网的同一日期范围比对。没有可用账号时记为“已实现、外部账户未验证”，不写全链路通过。

退出产物：独立模块及测试、设置入口、provider 文档、真实账号比对结果或明确的外部缺口。

### N2 分批入口与协议契约

| 批次 | 现有入口 / 建议新增模块 | 本批完成条件 |
|---|---|---|
| N2a 历史数据表达 | `ledger.rs`、`history.rs`、`card.rs`；可用专门的控制台 history 类型避免本地账本兼容风险 | 明确 actual cost、currency、account-wide、仅日级 timing；本地历史仍按原有 API 估算 |
| N2b 纯解析 | 建议 `deepseek_console.rs`；上游 `DeepSeekConsole.swift` 和对应 Tests | range/envelope/token/amount/cost/summary 的合成样本通过 |
| N2c 读取与凭据 | `browser_storage.rs`、`secrets.rs`、`providers/mod.rs::KeyRing`、`accounts.rs`、`providers/deepseek.rs`、`store.rs` | API Key/console 分槽且隔离；明确主账户/附加账户范围及回退状态 |
| N2d UI 与真实比对 | `settings_app.rs`、`history.rs::read`、`card.rs`、`localization.rs` | 入口、清除/重读、错误/空态、币种、官方同范围比对或外部未验证标注 |

`HistoryRead::Answered` 已有 `account_wide` 和 `actual_costs`；`UsageLedger` 当前没有 currency 字段。先确定最小兼容扩展，检查所有 struct literal、缓存序列化和 `$` 格式化，不把 CNY 填进现有默认美元字段。可将币种留在 provider history envelope；不要为此强制迁移所有本机日志。

固定上游协议（实施时仍需与已 fetch 的源码核对）：

- Origin `https://platform.deepseek.com`，localStorage key `userToken`，其值包装为 `{"value":"...","__version":"0"}`，null 代表退出；不把 API Key 当成控制台 token。
- `GET /api/v0/usage/by_api_key/amount?start=&end=&tz=`：每个 API key/model 的 `PROMPT_CACHE_HIT_TOKEN`、`PROMPT_CACHE_MISS_TOKEN`、`RESPONSE_TOKEN`；仅聚合已报告计数。
- `GET /api/v0/usage/by_api_key/cost?start=&end=&tz=`：按 currency 的实际费用；缺少钱包/费用与实际 0 费用分别表达。
- `GET /api/v0/users/get_user_summary`：同币种 normal + bonus wallet；不展示 token_estimation 推算 Token，也不据此标记耗尽。
- `start/end` 为秒、范围是当地今天及之前 29 天，到明天零点结束；`tz` 是整小时偏移**以秒表示**（+08 = 28800），向下取整，偏移余数加入 start/end。固定 +08、正负半小时区和 DST 转换样本，day bucket 不能直接假定 UTC 日期。
- HTTP 200 envelope 中外层 `code` 与内层 `biz_code` 分别校验；40000–40099 和 HTTP 401 是认证类别，403 单独保持网络/服务拒绝，不触发认证替换。
- 认证过期后只尝试读取一次不同的浏览器 token；拿到同一个 token/没有新 token 时结束，避免循环。缓存错误结果以限制重复请求，缓存 key 包含账户、凭据 revision、币种、范围；请求退出/取消不发布半成品。
- API Key 路由成功优先；没有 Key 或上游允许的拒绝/不可达/服务错误/不可读回复才尝试控制台余额。限流等其他原因不能随意降级。

N2 parser 的每个空值、负数、NaN/Inf、超界值、币种分支、错误 envelope 必须有明确结果；未知字段可忽略，缺少关键字段不能当成功。测试只使用合成或已脱敏样本，浏览器读取不输出 token 或整份账户正文到日志。

## 6. N3：Codex 本地异常线索（后于确定性功能）

依赖 N2；技术上可复用 N1 的扫描/缓存。参考 `daae75d` 和 `73b5d62`，建议 UI 使用“本地异常线索”，给出样本数与依据，不下“已降智”的结论。

- 只读本机记录，不发送探测请求。
- 分开显示可记录的模型/推理设置变化、上下文变化，以及推理 Token 分布的启发式异常。
- 用户设置在下一轮生效的时序、子代理/auto-review、恢复与分叉回放、重复 token_count 都需专门样本。
- 上游当前 effort 表仅到 `xhigh`；遇到其他或未来值保持未知，必须依据实际格式扩展，不能一律映射为更低强度。
- 上游的 `518n−2`、样本下限和 5% 阈值是启发式规则，展示算法版本、分母与样本不足状态；不把它们当服务端路由证据。

验收：正常用户切模型不误报；旧版或缺少设置事件的会话不作确定判断；取消读取不保存部分缓存；本地样本结论可复算。退出产物为解析器、误报回归、只读 UI 和限制说明。

### N3 解析、时序与接线清单

建议新增独立 `codex_signals.rs`，由 `lib.rs` 注册；不要把启发式标签混入额度或 Token 账单模型。接线参考 `model_details.rs`、`scan.rs` 和 `settings_app.rs` 的账户详情后台读取。根目录是 `~/.codex/sessions` 与 `archived_sessions`，只读 `rollout-*.jsonl`。

按照上游 `CodexSignals.swift` 和 `CodexSignalsTests.swift` 逐项构建状态机：

1. 识别 session_meta 版本/来源/父线程/分叉字段；旧版或无法确定用户设置变更的会话保持不可判断。
2. `thread_settings_applied` 仅在下一个 `task_started` 生效；运行中修改不能反向影响当前 turn 的判定。
3. `turn_context` 的 model/effort 与该轮开始时已生效的用户设置比较；上下文窗口只在可比较的同模型轮次判断。resume header 重置对应比较，helpers/subagents/auto-review 排除。
4. token_count 使用 `last_token_usage.reasoning_output_tokens`，累计 total_token_usage 去重；fork 回放窗口按上游规则与保守样本验证。
5. 上游格点规则 `reasoning >= 516 && (reasoning + 2) % 518 == 0`：分母为 reasoning 至少达到 516 的响应；不足 20 个样本不分类，至少 5 次命中且比例达到 5% 才给启发式标签。保留计数及算法版本，不能自动写为“服务器使用了弱模型”。
6. 努力等级中未知的新值不自动排序；需要可复现的实际设置格式才扩充。数据行缺失的 reasoning 不按 0 替代。
7. 文件缓存按路径/size/mtime 与解析算法版本隔离；取消不能保存部分 facts。切换时段和离页取消；页面关闭释放 UI 结果。

先纯 parser/tests，再后台读取与缓存，最后 UI/文案。检查有用户切换的会话、没有 applied event、新旧版本、mid-turn 修改、fork 重放、resume、重复计数、缺字段、未知 effort、恰好边界样本数。真实样本只验证可复算性，不拿社区阈值作为服务端模型证明。

## 7. N4：按实际 Windows 样本补齐，逐项退出

依赖 N3。不要把“剩余 20 个来源”作为一个大提交；每一项完成 reader、fixture、取消/缓存、真实样本核对后才进下一项。

候选顺序：

1. DSH 真正的 Zstandard 流式解压，保留仅 assistant/message 计数及分叉/重放语义；验证损坏/截断/取消和有界内存。
2. ZCode，以及有本机可复现样本的其他原生格式；无计数的格式维持未知，不能从金额反算 Token。Kiro 的额度 ACP 支持不等于其本地 Token 格式已支持。
3. 根据已有可用账户补测新 provider 路由，优先闭合实际使用链路；Kiro 需要支持的 Windows 原生 CLI 与用户完成登录。
4. 再补附加账户交互式 OAuth、续期和 Codex 独立 app-server 登录，重点验收凭据、缓存与进程隔离。
5. Windows 专有翻译、完整 BotMark 和其他外观项目放在最后。浏览器不支持的加密或存储形式继续明确报不可用。

实际执行到此阶段时，按可取得的样本重排 1–3 的内部次序，并记录唯一下一项。

N4a DSH 已完成：`additional_spend.rs::read` 按 magic `28 B5 2F FD` 选择有界 streaming decoder，再送入 `scan::LineReader`；`.zstd` 扩展名本身不证明压缩。回归覆盖实际压缩、伪扩展名纯文本、损坏/截断/拼接 frame、超大行和取消。source partial 贯穿聚合与 UI，依赖和 Cargo.lock 同步；细节与证据见第 8.8 节。

N4b reader 的注册通常涉及 `additional_spend.rs` 的 CATALOG、`native_supported`、roots/collect/parse、`spend.rs` 与来源文档。catalog 行存在不能当支持完成；必须验证 Windows 路径、文件锁/WAL（数据库来源）、模型归属、重放去重、来源空/失败状态及缓存失效。无格式样本时只补研究记录，保留标准导入能力。

## 8. 持续记录与验证规则

每项状态独立使用：`待实现`、`实现未验证`、`本地已验证`、`外部账户未验证`、`发布包已验证`。发布包通过不抹掉真实账户或人工交互缺口。

每次交接写明当前 commit、上游参考、唯一下一项、范围、依赖、验收、退出产物。先更新最新版 `Docs/upstream-implementation-status.md`，避免继续维护多份互相矛盾的旧计划。

### 8.1 每批提交与交接模板

每批完成定向检查后保留一个可审查 commit；提交消息写实际行为，不提前写发布版本。涉及账户/UI/并发链路的批次再跑相应跨模块回归。最终源码批次执行完整 workspace 检查，不将旧基线检查标记成新代码通过。

本地原始日志放 `windows/target/upstream-followthrough-20261004/`；仓库文档写结论、命令、退出码、源码 commit、二进制路径/哈希及验收缺口。日志目录不提交。更新本页的执行记录，并更新 `Docs/upstream-implementation-status.md`；旧忽略 `PORT_PLAN.md` 只作历史，不复制其 P5 下一项。

后续交接直接填以下字段：

```text
日期 / 时区：
工作目录 / 执行分支 / HEAD：
上游参考 commit：
已完成任务卡：
当前唯一下一项：
本批改动文件及行为：
检查命令 / 退出码 / 日志：
测试计数 / ignored 与原因：
真实账户 / UI / 包内 / 安装版各自状态：
before / after 性能样本、构建 profile 与程序哈希：
未解决问题及复现方式：
下一项的范围、前置与退出条件：
```

### 8.2 本次执行记录

| 任务卡 | 状态 | 当前证据 / 下一动作 |
|---|---|---|
| 交接计划细化 | 已完成，commit `e970842` | 本页增加任务卡、协议契约、测试入口与执行约束 |
| N0a 主目录同步 | 已验证 | `main == origin/main == db92d3e`，快进 13 提交；执行分支由此创建 |
| N0b 源码基线 | 已验证 | fmt、普通 Clippy、workspace tests（644 passed / 6 ignored）、release 均 exit 0；见 `baseline-results.json` |
| N0b 流式 UI 基线 | 已验证 | 3 轮 40 万行合成记录：150/450/600/750 Token 与磁盘缓存一致；已固定 Debug before payload |
| N1a-1 查价优化 | 本地已验证 | 单表惰性大小写索引、model/vendor 命中及未命中缓存；ledger 批量复用；10 项价格、45 项 ledger 定向测试通过，普通 Clippy exit 0 |
| N1a-2 解析缓存写盘 | 本地及隔离 UI 已验证 | 47 项 ledger、8 项 disk-cache 定向测试与 Debug build/fmt/普通 Clippy 通过；两次原进程结束后的热读取保持缓存 mtime/哈希，追加/重启/改写仍正确 |
| N1b 聚合后台与缓存 | 本地及隔离 UI 已验证 | 4 项聚合缓存测试、14 项 scan、16 项 Windows 单元检查、普通 Clippy；筛选使用同一快照、关闭/重开和取消 UI 通过 |
| N1c 最终本地回归 | 本地已验证，外部验收保留 | commit `4400e55`：fmt、普通 Clippy、655 passed / 6 ignored、Release build、CLI、三轮 after；完整 Debug payload 的隔离安装/托盘生命周期/卸载通过，Release 安装包构建成功 |
| N2a 历史表达 | 本地已验证 | HistoryRead::Answered 增加 currency；本机估值仍为美元估算，OpenCode 实际费用显式 USD，实际费用无币种时隐藏金额；新增 CNY/USD/EUR、实际零额/缺失及估值回归 |
| N2b 控制台解析 | 本地已验证 | 独立 token/envelope/range/amount/cost/summary；9 项固定合成样本通过，普通 core Clippy exit 0；此批无网络或凭据读取 |
| N2c-1 传输与凭据 | 本地定向已验证，未联网 | deepseek_session 的固定 route、有界 body、401/403 分类、一次不同 token 重读；KeyRing 仅传各账户自己的 console 槽 |
| N2c-2 缓存与接线 | 本地定向已验证，未联网 | 60 秒/4 项缓存（含错误）、单个并发读取、账户/token hash/币种/range key、取消与清除拒绝迟到缓存；账户历史与 API Key 优先余额 fallback |
| N2d 设置与验收 | 本地已验证，真实账户未验证 | 独立 WinUI/浏览器 fixture/DPAPI 通过；679 passed / 6 ignored，fmt/Clippy/Release 通过；完整 Debug 安装生命周期/卸载通过，Release 包仅构建 |
| N3a 纯解析 | 本地定向已验证 | 9 项合成误报/边界测试与普通 core Clippy 通过；没有文件扫描、后台或 UI 接线 |
| N3b 缓存与后台 | 定向已验证，UI 未接入 | 15 项 parser/reader、2 项 worker 状态测试；单个 worker、关闭/离页失效、文件与模型容量上限 |
| N3c 中文 UI 与验收 | 本地已验证 | 696 passed / 6 ignored、fmt/Clippy/Debug/Release；中文隔离切时段/取消/关闭重开、Debug 独立安装生命周期与卸载通过；Release 包仅构建 |
| N4a DSH 压缩 | 本地已验证，commit `40a5b5f` | 有界解压、缺口展示、706 passed / 6 ignored；12 份真实压缩样本独立复算，隔离 UI/Debug 安装通过 |
| R1 发布前验收 | 当前下一项 | 核对 main CI、正式 Release 包桌面、文档与版本；见第 9 节 |
| N4b 原生来源 | 待实施，R1 后进入 | 优先 ZCode 可复现样本，完成一项再进入下一项 |

当前产品改动状态以此表和源码为准。主仓库同步与本地构建均不会自动更新正在运行的用户安装版。

N0 明细：fmt 3.87 s、Clippy 104.70 s、workspace tests 120.77 s、release 215.78 s。测试累计 644 passed、0 failed、6 ignored；Clippy 仍有已有告警。三轮流式 UI 完整读取分别 5209/5642/7418 ms，中位数 5642 ms；追加中位数 939 ms、重启中位数 520 ms、改写中位数 752 ms。存在系统调度波动，保留 `baseline-stream-summary.json` 及逐轮原始 JSON，不把单次最小值当整体性能。

N1a-1 合成 probe：`cargo run -p quotascope-core --example price-lookup-benchmark --release --locked`，1001 条价格、每类 10000 次查找，断言命中数和价率。三轮同一 Release 树的 single/indexed 中位数：别名 765050/1304 µs、未计价 780399/2519 µs、精确 ID 1426/1609 µs；索引包含构建和首次解析。该测量是查价路径对比，不是整应用 CPU、内存或实际日志扫描提速。原始旧树三轮保存为 `price-before-*.jsonl`，改后两种路径为 `price-after-*.jsonl`；定向检查日志为 `n1a1-{prices-tests,ledger-tests,clippy}.log`。

N1a-2：`scan_cached` 跟踪 entry 的 stamp/新增/删除及失败移除，原始缓存未变时不调用 save；重新计价继续作用于新价格表。隔离 UI 命令为 streaming probe 的 `-Records 2000 -VerifyUnchangedCache`，结果 `n1a2-cache-ui-result.json` 的 `UnchangedCacheVerified=true`；该开关通过重新启动进程排除旧屏幕结果造成的假通过，不属于前后性能测量。完整/追加/重启/改写为 150/450/600/750 Token，SHA-256 `1A0F291B1EE4308B5839729150DA4C1FB4AB8309913618745156A3DBD9EF38D3`。单独修改了测试脚本可选参数，普通 before/after 的测量流程保持原有四阶段。

N1b：新增 `spend_analysis.rs`。缓存仅持有同一 `Arc<Snapshot>` 的最多四个结果，以指针身份和完整筛选 key 判断复用，不全量 hash。仅排序变化克隆并排序 Analysis，分页继续切片；日期、范围、来源、模型、分组和新快照均按依赖隔离。模型列表、小时、日图 bins 和来源覆盖在后台计算。Settings 内只有一个串行聚合 worker，换筛选取消旧 Control 并更新 generation，完成时检查 key、原始快照、开关和窗口状态；计算在 mutex 外，隐藏清空前台结果。新快照加载时保留已选模型，防止临时下拉框重置筛选。

N1b 定向结果：`n1b-core-tests-rerun.log` 17 passed（含 4 项新增缓存测试）、`n1b-scan-tests.log` 14 passed、`n1b-win-tests-final.log` 16 passed、`n1b-clippy.log` exit 0（保留原有告警）。`streaming_ui_benchmark.ps1 -Records 2000 -VerifyUnchangedCache -VerifyFilters`：150/450/600/750 与磁盘一致；模型 300/150、空来源 0、Codex 450、全部 450，分组/排序/范围均正确。测试先将文件追加至 600，筛选仍用持有的 450 快照，缓存 mtime/哈希不变，重启后才读取新增记录。结果 `n1b-filter-ui-result.json`；Debug SHA-256 `B84A46BCE348852CFAD64A8A40230D468A4B5BF487E99C2932CC064ADE239174`。

同一 Debug 的 `spend_ui_smoke.ps1` 通过：40 万行中取消延迟 175 ms，不发布半成品或保存部分缓存；离页/关闭取消、同宿主重开、完整重读 150、手动刷新取消后保留旧完整结果均通过，见 `n1b-cancel-ui-result.json`。开发期间保留了失败日志：首次 Rust 构建是 summary 名称覆盖，UI 脚本曾使用错误 TreeWalker 方法、追加步骤放错阶段、旧来源进度断言和读取正在替换的控件；已修复后按最终脚本重跑，失败不算通过记录。

合成聚合 probe：`cargo run --release -p quotascope-core --example spend-analysis-benchmark --locked`，16 来源 × 180 天 × 24 模型，总计 11681280 Token，30 次重复。三轮 reaggregate/exact-cache/sort-reuse 中位数 704703/4/413 µs；首次完整 Summary 中位数 37238 µs。断言总计、完整 Analysis、小时分布及复用标记。它只量化内存快照上的重复聚合路径，精确缓存路径的微秒值接近计时分辨率，不据此计算整应用提速倍数，也不是文件扫描或 UI 响应时间。原始 `n1b-analysis-{1,2,3}.log`。

N1c 对 `4400e55` 重跑 fmt、普通 Clippy、workspace tests、Release（exit 0；1.15/2.57/29.93/106.69 s），655 passed / 6 ignored。Release `--json` 在空隔离 profile 验证结构，未读真实账户。隔离安装输入为完整 **Debug payload**，版本 1.3.1、SHA-256 与 N1b 相同；安装哈希/必要 DLL、PRI、字体、真实托盘打开/关闭/同宿主重开/第二次启动/隐藏退出、卸载与独立注册表身份均通过。日志 `n1c-debug-installer.log`、`n1c-debug-installer-result.json` 和 `target/installer-validation-e524ba7d7e8f41599cfe8a62f2165077/`。独立安装完成后已卸载，当前用户安装版的文件和进程保持。

Release 安装包已由 Inno Setup 构建，见本次目录 `release-installer/`；安装器 SHA-256 `C23B1703BA69566F786FEEEE59D7D08B51C215A5B59C2AC90FE67F9E869CE3D6`。**没有运行这个 Release 的桌面/安装回归**：其单实例命名不支持 Debug 隔离变量，会与用户安装版争用。Debug 隔离成功不能替代此项；下一次用户授权升级/发布时在不争锁的桌面验证真实 Release 包。尚未推送、创建 PR、tag、CI 或发布，也没有替换用户安装版。

三轮同条件 40 万行 after 全部 150/450/600/750 正确；完整/追加/重启/改写 UI 时间中位数为 4987/755/716/710 ms（before 5642/939/520/752）。对应采样的 PrivateBytes 峰值增量中位数为 10.17/3.56/8.15/4.90 MiB（before 7.66/5.09/6.98/3.44）。数值有好有坏，样本少且受 UI Automation/调度影响，不宣称整应用 CPU 或内存改善；确定性收益是重复查价、无变化写盘和复用聚合。原始 `n1c-stream-{1,2,3}-{result,samples}.json` 与 `n1c-stream-summary.json`。未追加长空闲/所有真实账户/多 DPI 人工测试；本轮没有变更 dock 几何或快捷键，保留这些原有验收缺口，允许开始 N2 的本地数据表达。

N2a：`n2a-win-tests.log` 17 passed，`n2a-history-tests.log` 6 passed，随后 `n2a-card-tests-final.log` 定向复核显式零额与本机估值。金额币种放在 HistoryRead envelope，不修改 UsageLedger 和磁盘解析 schema；卡片实际收费去掉估算符号，以明确币种呈现，缺少币种不显示金额。控制台尚未接入，当前成功不构成 DeepSeek 账号验证。

N2b：`deepseek_console.rs` 纯解析和 `lib.rs` 注册；token wrapper 只接受有界非空可用于 header 的字符串。两个 envelope code 都要求显式成功；40000–40099 映射会话过期，缺层/错误类型保持 unreadable。Range 用各端点本地零点、当前 offset floor 与余数，日桶取中点以避免 DST 错一天。9 项测试覆盖 +08、+5:30、-3:30、测试用 DST zone、两 Key 同模型计数、30 天补零、无小时猜测、人民币/美元不相加、显式零额与缺费用、非有限/超界/负数/小数 Token、未知模型、同币种多行和钱包。

N2b 比上游更严格的边界：没有关键数组/code 不当作空历史；不完整 Token 标记 partial；只有至少一条显式收费（可为零）的可读币种才显示实际金额，选定币种有坏费用桶时隐藏金额，避免缺失变零；同币种重复 purse 合并，跨币种始终分开。Summary 不包含 is_available 或 token_estimation，余额无可读钱包为空列表。当前无 HTTP、浏览器、DPAPI 或 UI 接入；定向 9 passed（`n2b-parser-tests-final.log`）和普通 Clippy（`n2b-clippy.log`）通过，不替代外部账户验证。

N2c-1：新增 `deepseek_session.rs`，固定三个 HTTPS route、最多 8 MiB body、系统代理及原有 20 秒 timeout；HTTP 401 为 SessionExpired，403/重定向/服务拒绝保持 ServerError、429 RateLimited。`renewing` 的闭包接口只在 SessionExpired 时尝试一次不同 token，第二次再失败即停止。`secret(account)` 产生 `deepSeek:console` / `deepSeek#1:console`，KeyRing load/for_account 隔离这些槽，移除附加账户同时清 console。`browser_storage::find` 复用现有只读 active-manifest reader，Default 优先，最多 32 个 profile/浏览器；只解码目标 origin/key。自动浏览器续读仅主账户允许，附加账户必须显式导入，不借用主账户会话。新 secrets 写锁串行读/改/写，防止后台续期与另一个账户编辑丢失键。

N2c-1 验证：session 3 passed、accounts 3 passed、browser_storage 3 passed、Windows 单元 17 passed、普通 workspace Clippy exit 0；日志 `n2c-{session-tests-final,accounts-tests,browser-tests}.log` 和 `n2c1-{win-tests,clippy}.log`。尚未在 provider 中调用 transport，也没有执行浏览器导入或真实凭据请求；DPAPI 隔离持久化与设置入口将在 N2d 的独立 profile 进程中验收。此时旧安装包/旧整仓绿灯不能覆盖新 transport 的后续接线。

## 参考证据

- [Windows 1.3.1 Release](https://github.com/Lyx721188/QuotaScope/releases/tag/windows-v1.3.1)
- [Windows 1.3.1 标签 CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37131049528)
- [Windows 固定提交的移植状态](https://github.com/Lyx721188/QuotaScope/blob/db92d3e9687d4db8d959c30ce5ee4a3e7b702c44/Docs/upstream-implementation-status.md)
- [Pulse 1.7.1 Release](https://github.com/qunqin24/Pulse/releases/tag/v1.7.1)
- [上游性能优化 3696a65](https://github.com/qunqin24/Pulse/commit/3696a65b428272aa25c3ba611de8df2536983515)
- [DeepSeek 控制台说明（固定提交）](https://github.com/qunqin24/Pulse/blob/3696a65b428272aa25c3ba611de8df2536983515/Docs/providers/deepseek.md)
- [Codex 线索说明（固定提交）](https://github.com/qunqin24/Pulse/blob/3696a65b428272aa25c3ba611de8df2536983515/Docs/providers/codex.md)

N2c-2：新增 `deepseek_history.rs`。缓存键包括账户、token SHA-256（缓存不留明文）、币种与完整 range；60 秒 TTL 同样缓存错误，最多 4 个结果，网络与解析在锁外，单个生产者、Condvar 有界等待。等待者共享本次旧 token 续读结果，后续按新 token key 命中；取消、清除或 panic 释放 worker，清除前进入的等待者不能重新缓存旧凭据。附加账户删除同时 forget。3 项缓存测试覆盖 TTL/依赖/LRU、并发续期合并及取消/迟到/panic。

Provider 接线保持 API Key 优先，仅 ApiKeyRefused/Unreachable/ServerError/UnreadableReply 允许控制台余额成功接替；控制台失败保留原 Key 错误，RateLimited/NoLimitsReported 不切换。无 Key 时传回真实 session 错误，不制造余额零。余额峰值保留主账户旧币种 key，附加账户加入 id 隔离；串行更新防并发账户丢失。非有限、负数和超界金额不当作余额。4 项策略测试通过。Store 调用实际 AccountKey/自己的 basis，官网历史不受本机 Token 开关控制，DeepSeek 附加账户也可显示历史。App 历史 worker 新增取消作用域，generation 继续拒绝旧完成结果；阻塞 HTTP 最多仍等待原有 20 秒 timeout。

N2c-2 检查：`n2c2-core-tests-final.log` 共 20 passed（含 9 parser、3 session、3 cache、4 provider 策略与 1 既有集成），Windows 单元 17 passed；`n2c2-clippy-final.log` exit 0，仍有既有告警。首轮 Clippy 发现新 App 控制句柄的 import/可见性编译错误，已修正并保存失败日志，不把它算成成功。这一批没有执行网络、读取真实浏览器或安装程序；N1 包哈希不能覆盖此批新代码。

N2d：设置页后台导入和清除 DeepSeek 官网会话，附加账户在用量概览有自己的导入、清除和详细卡片开关。导入不输出 token；没有找到会话不清除旧凭据，主/附加账户与 API Key 分开。删除账户使进行中的导入失效；自动续期持久化前在互斥范围确认原 token 仍存在，清除或替换后不能复活旧凭据。余额 fetch 每次取得该账户当前加密会话，解决自动续期后 KeyRing 仍留旧 token 的问题。手动刷新清除相应官网历史缓存。用户配置见 `Docs/providers/windows-deepseek-console.md`。

N2d 当前验证：`n2d-workspace-tests-current.log` 679 passed / 6 ignored；`n2d-clippy-current.log` exit 0，Windows 仍为既有 83/84 告警；fmt、Debug 与 Release 构建完成。新增真实 loopback HTTP 测试验证 Bearer GET、JSON、401/403/429 和不跟随 302。`deepseek_console_ui_smoke.ps1` 使用 Edge Default 的其他站点、Profile 1 的目标站点和删除记录；真实 WinUI 操作及 DPAPI 解密确认两个账户导入/清除互不覆盖，API Key 保留；官网请求仅经不转发的本机代理，无真实账户请求。

N2 整仓首轮失败追溯：旧 Stamp 的 f64 秒在 JSON 回读时可差 1 ULP；独立 `stamp-roundtrip-probe.log` 固定例为 1791051801.0000021，读回少 0.0000002384185791015625 秒。`9615472` 改用整数纳秒；旧缓存保留可读，缺少新纳秒字段会安全重读一次。48 项 ledger 定向与完整工作区回归通过。首次 UI 脚本文案/页面切换等待问题已修正；首个 Debug 验证 payload 缺失 WinUI 资源目录导致 idle 退出，补齐全部非 Cargo 运行时目录后通过。保存失败日志，不把失败轮计作成功。

N2 冻结程序：Debug `529F9268E8A03073711244C182480E0F62AA75B56202B52DB2EA893281588272`，Release `E82B54B741FD55143E2B17C9D9D3F4CE1E01200121498E05C1C6BFB50EB21B1A`。最新 UI 证据 `n2d-console-ui-current.log` / `n2d-console-ui-result.json`；完整 Debug 安装、托盘关闭/重开/二次启动、隐藏退出与卸载在 `target/installer-validation-e7f64eda9d4142d9b1a8076c33f154f4/` 全部通过，用户安装版保留。Release 包 `n2-release-installer-current/QuotaScope-1.3.1-windows-x64-Setup.exe` SHA-256 `C68C6E8F665C840A78D8B481620646FA9434F353DC1B8BCC034000CC1CE7947C` 仅构建，无 Release 桌面/安装或发布验证。

整数时间戳后的 40 万行流式 UI 样本见 `n2d-stream-result.json`：150/450/600/750 Token、筛选复用、跨进程缓存不变均通过；5037/902/732/775 ms 为这一轮观察，非性能承诺。该轮 Debug SHA 为中途 `4A96B4C...`，后续只更改官网凭据刷新/手动历史缓存接线；最新程序另经整仓和官网 UI/安装检查，避免将旧 SHA 的流式样本冒充最新 SHA。冻结的 `n2-debug-payload` / `n2-release-payload` 均包含所有 DLL、PRI、Fonts 和语言/WinUI 资源目录以及许可证。`n2d-results.json` 汇总本地检查；真实 DeepSeek 统计与多币种账户、Release 桌面、用户升级及 CI/发布保持未验证。

后续 N3 先验证纯 parser 再注册后台/缓存/UI。只读取本机 rollouts，分离请求设置事实与格点分布启发式；不发送模型探测，不把任何标签写成服务端实际模型结论。未知 max/ultra effort 不排序；缺累计计数无法去重时跳过并标记部分记录，不以 0 补齐。无 applied 事件、旧版本、helper/reviewer 和 fork 回放必须固定误报样本。完成后才能进入 N4a DSH 实际 Zstandard 解码。

N3a：新增 `codex_signals.rs` 的纯解析与 `lib.rs` 注册，算法名 `codex-signals-v1`。每日/模型计数保留正 reasoning 的 responses、至少 516 的 reached、格点 hits；使用 remainder 规则避免 reasoning + 2 溢出，分母 20、命中 5、比例 5% 三个条件缺一不可。缺累计身份时跳过并 partial，累计字段缺值不补 0，重复 totals 和重复 context 不重复记样本/变动。

设置判断比上游更保守：只依据该轮 task_started 前已 applied 的模型/effort；不从前一轮实际请求推断用户选择，第一次 applied 很晚时不追溯此前轮次。显式 null 清除可比较选择；缺 task_started 的 context 保持不可判断。未知 max/ultra/未来 effort 不排序；0.144 之前、无真正可比字段、helpers/source/parent、auto-review 与 fork 前两秒回放排除或保留未知。上下文窗口仅在已锚定同一记录模型时比较，resume header 重置窗口比较。数据是请求侧记录，不含服务端实际模型证据。

`n3a-parser-tests-final.log`：9 passed（格点与样本边界、模型归属和去重、用户/中途切换、迟到和清除设置、未知 effort/版本、helpers/reviewer/缺 context、同模型窗口与 resume、fork/缺计数）。`n3a-clippy.log` exit 0，31/39 为既有告警。尚未扫真实目录，没有磁盘/后台缓存或 UI；当前 N2 冻结程序和安装包不包含 N3。下一项 N3b 必须按 path/size/整数 mtime/算法版本区分 facts，缓存设文件数与内存上限，删除/改写/追加失效，取消和改写中的文件不缓存；时段切换复用 facts，离页/关闭释放 UI 与 reader，之后才实现 N3c 中文展示。

N3b：新增 codex_signal_reader.rs 与 Windows codex_signal_state.rs。只流式读取 rollout-*.jsonl，不保存完整对话；按路径/size/SystemTime 精确时间戳/算法版本缓存完整 facts，时区 offset 改变时清空。删除清除、追加/改写重读；文件读取前后 stamp 不同、UTF-8/IO 失败或取消均不缓存。缓存最多 1,024 文件、8 MiB 保守结构记账预算（并非进程 RSS），聚合最多 1,024 模型，UI summary 在后台排序并只保留前 64 行与最新 8 个变动。目录深度/枚举也有上限，超出显示 partial。

时段采用本地自然日 30/90/全部，切换复用 facts；同一个串行 worker 更新最新 query，旧 generation 的完成结果拒收。手动刷新替换 reader、保留已有完整快照直到新结果；取消保留已有完整结果；关闭/离页释放 query/result/reader，重开等待旧 worker 退出。后台入口已实现但未挂 UI，不能宣称真实窗口验证。新增初始 null usage 回归：没有已完成用量的 context 消息既不是零样本，也不额外制造记录缺失。

n3b-reader-tests-final.log：15 passed（10 parser + 5 reader），覆盖自然日/扩大范围复用、追加/改写/删除、同大小精确时间戳、损坏修复、容量和中途取消后重读；n3b-worker-tests.log：2 passed，验证时段复用/刷新替换/旧结果拒收和关闭/取消状态。当前 N2 程序与包不包含此批；唯一下一项 N3c 将挂入 Codex 展开卡片，提供中文时段、手动刷新与取消、样本分母和不可判断状态，然后完成整仓与隔离 Windows 验收。

N3c：Codex 主账户展开页接入中文本地线索，包括 30/90/全部自然日、手动刷新与取消、算法标识、格点计数/分母/样本不足/部分记录、可比较与未知会话及最近八个参数差异。记录不完整时不提供格点集中分类。可比较轮次没差异也不证明未知轮次一致。收起、搜索隐藏、离页、关窗及关闭本地读取都会释放该页面的 reader/result；信号区域使用独立稳定 key，避免其他详情完成时重建控件。提供 Docs/providers/windows-codex-signals.md 与聚合数字专用 codex-signals-read example。

N3 当前检查：n3c-workspace-tests.log 为 696 passed / 6 ignored；最后 UI/规则标识改动经 n3c-win-tests-current.log（20 passed）、fmt、普通 Clippy、Debug/Release 构建和最新隔离 UI 重跑，均 exit 0。Clippy 仍为既有 core 31/39、Windows 83/84 告警。UI 初轮通过，随后发现详情切换时的 UIA 空 selector/关闭后过期 root；稳定 key 与等待/重取 root 已修正，保留失败日志，不把失败轮当成功。最新版 n3c-ui-current.log / n3c-ui-result.json：30/90/all、20 回复 6/20、少样本 3/3、参数差异、手动更新 21、40 万行取消保留完整结果、离页/关闭重开均通过，取消提示 98 ms 为观察值。

只读本机 n3c-local-read.log：30 天读取 122 文件、122 会话、52 个含可比较设置，4,700 正推理回复、750 分母、330 格点，partial=false；扩大 90 天复用 122 文件并新读 68，190 会话、82 个可比较、6,648 正推理回复、1,105 分母、378 格点，partial=true。日志仅聚合数字，没有对话、文件路径、凭据或模型名；这是实际格式读取 smoke，尚未独立逐条复算，不据此推断服务端实际模型。

冻结 N3 Debug SHA-256 3E4806BB23DB4928D65C02EB139AC60EBECACEE04E982FF9137559B545D0D0EE；Release 656D78175E54A1467E11D2A8DC3A7BB992D50C249320BCED13B3095FD331ACCF。完整 n3-debug-payload 的独立安装/托盘/关闭重开/二次启动/隐藏退出/卸载全部通过，target/installer-validation-9faa78c28a2b499d9891218bb44acb23/，用户安装版保留。n3-release-installer/QuotaScope-1.3.1-windows-x64-Setup.exe SHA-256 E24EE140D47FD0C909A11646B9F3254E00318B1F90109EE91B046C12AF93AD02 仅构建，无 Release 桌面、真实账户、升级或发布验证。

当时的下一项 N4a：已有本机 DSH session.jsonl.zstd 的真实 28B52FFD magic，并用 Python 3.14 compression.zstd 只读检查了三份实际文件的事件/usage 键形状。实现需同步 Cargo.lock，采用有界 streaming decoder 与 LineReader，覆盖伪扩展名、拼接帧、损坏/截断、解码量/行/窗口上限和取消。保存来源 partial，保留 assistant/message 的计数边界及 fork seed/replay 规则，再用同一份临时冻结字节做独立解码/计数核对。真实对话不能进入 Git 或日志。N3 的二进制和包不包含后续 N4。

### 8.8 N4a 完成与 main 合并检查

2026-10-04，N4a 及工作区已有改动提交为 `40a5b5f`，与前面 14 个提交一起快进到本地 main，无合并冲突。其余本地命名分支已全部包含在 main；没有删除分支或 worktree。一个旧 detached worktree 仍有历史未提交改动，保持原状，不作为本次活动分支的待合并提交。

DSH 读取保留 assistant/message、fork seed、重放去重和 reasoning 不重复计数的边界，严格区分缺计数与零计数。原始/解码量各 64 MiB、单行 4 MiB、history window 8 MiB；取消、截断、损坏、超限、文件变化及非法计数都有回归。Token 页和来源覆盖区显示 partial，筛选与排序保留，修复后刷新消除已解决的缺口。依赖 zstd 已锁定并附带分发许可。

合并前在当前源码重新执行的检查，日志位于 `windows/target/main-integration-20261004/`：

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo test --workspace --locked` | exit 0；706 passed / 6 ignored |
| `cargo clippy --workspace --all-targets --locked` | exit 0；仍有告警，没有启用 `-D warnings` |
| `cargo build --release --locked` | exit 0 |
| Release `quotascope.exe --json` | exit 0；含 generatedAt/accounts，输出未记录账户内容 |

现有 N4a 定向验证记录位于 `windows/target/upstream-followthrough-20261004/`，本次核对其程序哈希与当前构建一致，没有把这些记录标成新一轮桌面运行：

- `n4a-live-compare-current.log` 与 `n4a-live-release-compare.log`：12 份真实压缩文件、183 条去重记录，独立 Python 与 Debug/Release Rust 均得到 10,170,898 Token；1 条缺 usage，双方均保留 partial。仅保存聚合和 corpus digest，不提交真实记录。
- `n4a-ui-result.json`：真实压缩、追加、来源筛选、缺口展示、修复重读通过，使用隔离合成 profile。
- `n4a-debug-installer.log`：独立身份 Debug 安装、托盘生命周期、卸载通过，用户原安装版保留。
- Debug `quotascope.exe` SHA-256：`67784BA2F573CDBEF1A8B75F76DEBE3223B6CB6BFA65F34A83B636BCAB786119`。
- Release `quotascope.exe` SHA-256：`E55E341AAD1B98769842EAB49B1E02729E2A6FF87EF7EAE13DB934358906D2ED`。
- `n4a-release-installer/` 中的安装包构建通过，SHA-256：`B086270B83D5E1687355926C669E4C32350F6F9AE260464DDD47D6A05F93563E`；Release 桌面/用户升级尚未验证。

本次合并保持版本号 1.3.1，不创建发布标签。GitHub Actions 必须以推送后 main 的实际 SHA 对应运行判断；历史成功不能替代本次运行。

## 9. main 合并后的下一阶段

本节是后续执行顺序；前文各阶段的“下一项”和测试数字作为历史记录保留。用户继续要求推进并补充细节后，当前执行 **R1 发布前验收**，细化任务、依赖与退出标准见[发布验收与下一轮实施任务](release-acceptance-2026-10-04.md)。先形成可交付基线，再扩充来源。

| 顺序 | 工作 | 完成标准与产物 |
|---|---|---|
| R1 发布前验收 | 核对 main Windows Actions；用该提交的完整 Release payload 在不争用现有安装版的桌面完成安装、托盘、设置重开、Token 页和卸载；核对版本、CHANGELOG、README 的旧路由数量和 Windows README 旧支持表 | 最终提交对应 CI 成功；Release 程序与包 SHA、桌面/安装结果及遗留问题齐全；发布说明明确账户验证边界。需要用户安装版升级或正式发布时再落实对应操作范围 |
| N4b ZCode 原生来源 | 先定位可复现 Windows 样本和格式版本，再实现只读 reader；按源计数，缺模型/Token 保持未知，覆盖追加、改写、分叉/重放、锁文件、损坏和取消 | 固定脱敏 fixture；真实冻结样本独立聚合一致；来源筛选、partial、刷新与缓存失效通过。没有可信计数样本时保留标准导入并记录缺口，不从金额反推 Token |
| R2 常用服务商实账验证 | 优先 DeepSeek 官网历史和 Kiro ACP，再按实际使用选择其他已注册路由；逐个核对同账号、同日期、同币种的官网结果，覆盖过期/空态及主附加账户隔离 | 每个 provider 单独记录代码状态、真实账户结果和问题复现；无法登录的条目保留待验收，不阻塞其他可验证条目 |
| R3 多账户登录与体验收尾 | 在上述基线稳定后补交互式 OAuth/续期、Codex 独立登录进程隔离；随后处理快捷键冲突、多 DPI 交互和 Windows 专有翻译，完整 BotMark 排在最后 | 登录/刷新/撤销/移除后凭据、缓存和子进程隔离可复现；真实交互验收和文案覆盖分别有记录 |

R1 的具体执行拆分：

1. 核对 main Actions 的 fmt、Clippy、测试、Release、JSON、安装包与包内生命周期结果；若失败，先修复主线，不开启新来源开发。
2. 清理发布文档口径：以现有 Windows 路由表的 77 个注册路由为准；54 个 Token 来源仍拆分为 34 个已知格式与 20 个标准导入。明确 N1/N2/N3/N4a 的新增行为及真实账户限制。
3. 为正式 Release 安排独立桌面或明确的升级窗口，避免其单实例锁与运行中的安装版争用；保留旧版本和数据，验证启动、重开、统计、退出及卸载。Debug 隔离安装的结果仅用于 Debug 验收。
4. 根据最终变更确定版本号、更新说明和发布包；形成可审查的发布记录后结束 R1。当前已继续推进 CI 安装/卸载自动验收和文档收尾；正式发布、用户安装版升级仍单独记录执行范围。

新一轮实现从更新后的 main 创建 `codex/` 分支，每个 reader 或账户链路单独提交。延续不猜百分比、不从金额反推 Token、不把 fixture 成功当实账通过，以及不新增 Claude 跨文件回复去重的既有范围。
