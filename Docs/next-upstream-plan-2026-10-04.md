# 下一轮上游跟进计划

核对日期：2026-10-04（Asia/Shanghai）。状态：**N0、N1a、N1b 已验证，N1c 为当前下一项**。

建议顺序：对齐执行基线 → 性能与现有功能验收 → DeepSeek 官网历史 → Codex 本地异常线索 → 按样本补齐原生来源与登录。
当前唯一下一项是 **N1c：整仓回归、最终性能样本和 Windows 生命周期**。N0 同步、N1a 查价与缓存写盘、N1b 聚合后台接线已完成；每阶段满足退出条件后才开始下一阶段。版本号为建议，实施时再确认。

执行分支：`codex/upstream-followthrough-20261004`，由 `db92d3e` 创建。主目录是 `D:/Projects/QuotaScope`，Cargo 工作目录是其 `windows/` 子目录。
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
4. 54 个 Token 来源是目录规模，不等于全部原生支持。当前状态文档记录 34 项已知格式读取（含导出），20 项仅标准导入；真正压缩的 DSH Zstandard 仍缺解压。
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
- `start/end` 为秒、范围是当地今天及之前 29 天，到明天零点结束；`tz` 按上游处理整小时偏移及余数。固定 +08、半小时区和 DST 转换样本，day bucket 不能直接假定 UTC 日期。
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

N4a DSH 的实际入口是 `additional_spend.rs::read`：读到 magic `28 B5 2F FD` 时直接 partial + skip；`.zstd` 扩展名本身不证明压缩。实现时按 magic 选择 streaming decoder，送入已有 `scan::LineReader`，同时覆盖实际压缩、伪扩展名纯文本、损坏/截断/拼接 frame、超大行和取消。保留 source partial 标记，解码错误不能将前半文件保存成完整缓存。新增依赖时同步 Cargo.lock，使用 `--locked` 重新核验。

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
| N1c 最终回归 | 当前下一项 | 完整 workspace 检查、三轮 after 性能、托盘生命周期、隔离 payload/安装；发布与用户安装版升级不由本地检查自动完成 |
| N2 / N3 / N4 | 待实现 | 不使用 fixture 成功替代实际账号/客户端验收 |

当前产品改动状态以此表和源码为准。主仓库同步与本地构建均不会自动更新正在运行的用户安装版。

N0 明细：fmt 3.87 s、Clippy 104.70 s、workspace tests 120.77 s、release 215.78 s。测试累计 644 passed、0 failed、6 ignored；Clippy 仍有已有告警。三轮流式 UI 完整读取分别 5209/5642/7418 ms，中位数 5642 ms；追加中位数 939 ms、重启中位数 520 ms、改写中位数 752 ms。存在系统调度波动，保留 `baseline-stream-summary.json` 及逐轮原始 JSON，不把单次最小值当整体性能。

N1a-1 合成 probe：`cargo run -p quotascope-core --example price-lookup-benchmark --release --locked`，1001 条价格、每类 10000 次查找，断言命中数和价率。三轮同一 Release 树的 single/indexed 中位数：别名 765050/1304 µs、未计价 780399/2519 µs、精确 ID 1426/1609 µs；索引包含构建和首次解析。该测量是查价路径对比，不是整应用 CPU、内存或实际日志扫描提速。原始旧树三轮保存为 `price-before-*.jsonl`，改后两种路径为 `price-after-*.jsonl`；定向检查日志为 `n1a1-{prices-tests,ledger-tests,clippy}.log`。

N1a-2：`scan_cached` 跟踪 entry 的 stamp/新增/删除及失败移除，原始缓存未变时不调用 save；重新计价继续作用于新价格表。隔离 UI 命令为 streaming probe 的 `-Records 2000 -VerifyUnchangedCache`，结果 `n1a2-cache-ui-result.json` 的 `UnchangedCacheVerified=true`；该开关通过重新启动进程排除旧屏幕结果造成的假通过，不属于前后性能测量。完整/追加/重启/改写为 150/450/600/750 Token，SHA-256 `1A0F291B1EE4308B5839729150DA4C1FB4AB8309913618745156A3DBD9EF38D3`。单独修改了测试脚本可选参数，普通 before/after 的测量流程保持原有四阶段。

N1b：新增 `spend_analysis.rs`。缓存仅持有同一 `Arc<Snapshot>` 的最多四个结果，以指针身份和完整筛选 key 判断复用，不全量 hash。仅排序变化克隆并排序 Analysis，分页继续切片；日期、范围、来源、模型、分组和新快照均按依赖隔离。模型列表、小时、日图 bins 和来源覆盖在后台计算。Settings 内只有一个串行聚合 worker，换筛选取消旧 Control 并更新 generation，完成时检查 key、原始快照、开关和窗口状态；计算在 mutex 外，隐藏清空前台结果。新快照加载时保留已选模型，防止临时下拉框重置筛选。

N1b 定向结果：`n1b-core-tests-rerun.log` 17 passed（含 4 项新增缓存测试）、`n1b-scan-tests.log` 14 passed、`n1b-win-tests-final.log` 16 passed、`n1b-clippy.log` exit 0（保留原有告警）。`streaming_ui_benchmark.ps1 -Records 2000 -VerifyUnchangedCache -VerifyFilters`：150/450/600/750 与磁盘一致；模型 300/150、空来源 0、Codex 450、全部 450，分组/排序/范围均正确。测试先将文件追加至 600，筛选仍用持有的 450 快照，缓存 mtime/哈希不变，重启后才读取新增记录。结果 `n1b-filter-ui-result.json`；Debug SHA-256 `B84A46BCE348852CFAD64A8A40230D468A4B5BF487E99C2932CC064ADE239174`。

同一 Debug 的 `spend_ui_smoke.ps1` 通过：40 万行中取消延迟 175 ms，不发布半成品或保存部分缓存；离页/关闭取消、同宿主重开、完整重读 150、手动刷新取消后保留旧完整结果均通过，见 `n1b-cancel-ui-result.json`。开发期间保留了失败日志：首次 Rust 构建是 summary 名称覆盖，UI 脚本曾使用错误 TreeWalker 方法、追加步骤放错阶段、旧来源进度断言和读取正在替换的控件；已修复后按最终脚本重跑，失败不算通过记录。

合成聚合 probe：`cargo run --release -p quotascope-core --example spend-analysis-benchmark --locked`，16 来源 × 180 天 × 24 模型，总计 11681280 Token，30 次重复。三轮 reaggregate/exact-cache/sort-reuse 中位数 704703/4/413 µs；首次完整 Summary 中位数 37238 µs。断言总计、完整 Analysis、小时分布及复用标记。它只量化内存快照上的重复聚合路径，精确缓存路径的微秒值接近计时分辨率，不据此计算整应用提速倍数，也不是文件扫描或 UI 响应时间。原始 `n1b-analysis-{1,2,3}.log`。

## 参考证据

- [Windows 1.3.1 Release](https://github.com/Lyx721188/QuotaScope/releases/tag/windows-v1.3.1)
- [Windows 1.3.1 标签 CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37131049528)
- [Windows 固定提交的移植状态](https://github.com/Lyx721188/QuotaScope/blob/db92d3e9687d4db8d959c30ce5ee4a3e7b702c44/Docs/upstream-implementation-status.md)
- [Pulse 1.7.1 Release](https://github.com/qunqin24/Pulse/releases/tag/v1.7.1)
- [上游性能优化 3696a65](https://github.com/qunqin24/Pulse/commit/3696a65b428272aa25c3ba611de8df2536983515)
- [DeepSeek 控制台说明（固定提交）](https://github.com/qunqin24/Pulse/blob/3696a65b428272aa25c3ba611de8df2536983515/Docs/providers/deepseek.md)
- [Codex 线索说明（固定提交）](https://github.com/qunqin24/Pulse/blob/3696a65b428272aa25c3ba611de8df2536983515/Docs/providers/codex.md)
