# 下一轮上游跟进计划

更新日期：2026-10-04（Asia/Shanghai）。本页记录公开开发计划、已实现行为和验收边界。功能状态见[移植进度](upstream-implementation-status.md)，具体任务见[发布验收与下一轮实施任务](release-acceptance-2026-10-04.md)。

代码基线：N1/N2/N3/N4a 已随 `3c69d2f` 汇总到 main；R1 自动化和文档改动位于 `ef9292d`。已发布版本仍为 `windows-v1.3.1`，main 的新增功能列在 CHANGELOG 的 Unreleased 中。上游参考固定为 Pulse `3696a65`，后续上游变化需另行比较。

## 1. 已完成的功能

| 阶段 | 已实现行为 | 验证与限制 |
|---|---|---|
| N1 性能 | 批量查价复用价格索引及命中/未命中缓存；未变日志的解析缓存不重写；Token 聚合移到单个可取消 worker | 价格优先级和未计价语义保留；筛选、排序、追加、改写、删除、重启续读和取消已有回归 |
| N2 DeepSeek 官网 | 近 30 个自然日的 Token 与实际账单，人民币/美元分别显示；官网会话导入、余额回退、账户隔离和刷新 | 合成协议、HTTP、DPAPI 与隔离 UI 已验证；真实账户同口径对照仍待完成，见[使用说明](providers/windows-deepseek-console.md) |
| N3 Codex 线索 | 读取本地 rollout 中设置与请求参数的差异，并单独展示启发式线索、样本分母与不可判断项 | 有界可取消 reader 和隔离 UI 已验证；记录参数不证明服务端实际运行模型，见[使用说明](providers/windows-codex-signals.md) |
| N4a DSH | 按 magic 选择有界 Zstandard 解压，支持拼接帧；缺计数、损坏、截断和变动传播 partial | 纯解析、取消、隔离 UI 和冻结样本独立复算已验证；仅计助手消息的已报告用量，见[使用说明](providers/windows-dsh-local-usage.md) |
| N4b ZCode | 固定 SQLite `model_usage` 的归一化计数、只读 WAL 快照、有界扫描与内存缓存 | 18 项定向回归、冻结用量独立复算及隔离 Debug UI 通过；[格式与边界](providers/windows-zcode-local-usage.md) |
| R1 自动验收 | ZIP 携带许可证；CI 使用同一 Release payload 执行独立身份安装、托盘/设置生命周期和卸载 | [ef9292d 的 Windows CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37171576671) 成功；[dcaa37c 的 CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37184221991) 另通过 DSH/ZCode Release Token 页；不覆盖原 AppId 升级、真实账户或多 DPI 人工验收 |

Windows 路由表有 77 项。Token 目录有 54 项，其中 35 项读取已知格式（含导出缓存），19 项只支持标准导入；ZCode 现在读取原生 SQLite 用量。目录注册不代表原生格式支持，Kiro 额度 ACP 支持也不代表其本地 Token 格式已支持。

## 2. 必须保留的行为

- 不从金额反推 Token，不猜缺失额度分母或百分比；缺模型、价格、时间或计数按各来源语义保留未知/partial。
- 价格变化只重新计价；未变的原始记录不因此重写解析缓存。缓存 key 包含路径、长度、精确时间戳和解析版本，覆盖追加、改写、同大小替换、截断、删除与修复。
- 原始快照 revision 与任务 generation 分开。聚合缓存有界，页码和排序复用结果；不同筛选不能短暂显示其他筛选的旧统计。
- 扫描与聚合都检查取消。关闭、离页或取消后，迟到结果不回填，半成品不覆盖最后一个完整快照或持久化缓存。
- DeepSeek 的实际费用携带币种；缺币种不显示金额。凭据、续读、缓存、删除与清除按账户隔离，附加账户不自动借用主账户登录。
- Codex 线索只比较有可比记录的请求；旧版本、辅助任务、fork 回放与未知 effort 保守处理，不据此修改账单或额度。
- DSH 仅计 `assistant/message`；reasoning 已包含在 output，不再次相加。分叉前缀和有稳定身份的重放排除，缺 usage 不补零。
- 保持现有 Claude 单文件回复合并范围；不新增跨文件回复去重。已取消的悬浮行为维持现状，外观扩展在读取与账户正确性之后。

## 3. 验证方式

在仓库的 `windows/` 目录执行，Cargo 构建顺序运行：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked
cargo test --workspace --locked
cargo build --release --workspace --locked
```

基线源码回归为 706 passed / 6 ignored，fmt、普通 Clippy、Release 与 JSON CLI 完成；已有 Clippy 告警仍存在，不能称为严格 `-D warnings` 通过。后续源码变动需要重新验证，旧结果不能覆盖新改动。

账户、UI、安装和分发分别验收。合成 fixture 证明解析规则，隔离窗口证明指定交互，CI 包验证指定 payload；这些结果不能替代真实账户、原 AppId 升级或多 DPI 人工操作。每项结果记录源码 commit、命令、退出码、程序/包 SHA 与遗留问题。

原始会话、数据库、凭据、个人工作指令、机器路径和真实用量统计只保留在本地忽略目录 `work/` 或 `windows/target/`。公开材料仅保留脱敏格式说明、合成 fixture、可复现命令和验证结论。普通删除不会清除 Git 历史中的旧内容，历史处理需单独评估分支、标签和协作引用。

## 9. main 合并后的下一阶段

N4b-1 至 N4b-4 的格式、解析、缓存、冻结样本与 Debug UI 已完成。**R1d 已完成**：dcaa37c 的 CI 使用实际 ZIP 内同一份 Release 程序，通过 DSH/ZCode Token 页面、刷新、修复及关闭重开验收。R2 前提检查尚未找到受支持的 Kiro 原生 CLI；DeepSeek 实账对照仍未完成。标准导入的严格计数、零值覆盖、partial/未计价 UI 已随 `53bd960` 完成，[对应 Windows CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37186053767) 成功。各来源的有界遍历、读取流、记录预算和流式 JSON 数组随 `f08ee5a` 通过 734 项本地测试、隔离 Debug UI 及 [Release CI](https://github.com/Lyx721188/QuotaScope/actions/runs/37188857492)。浏览器回退副本的路径隔离与清理已实现，新增三项合成回归，整仓 737 passed / 6 ignored；该批 CI 随提交单独核对。之后处理账本异常日期跨度与取消；升级与发布任务继续单独保留验收状态，详见[任务卡](release-acceptance-2026-10-04.md)。

| 顺序 | 范围与依赖 | 验收标准 | 退出产物 |
|---|---|---|---|
| N4b-1 格式取证 | 只读确认格式版本、数据位置、请求身份、时间、模型及 input/output/cache/total 语义；检查数据库 WAL/一致性或 JSONL 追加/轮转 | 可信计数与缓存包含关系明确；无法判断的情形列出；无可靠计数则保留标准导入 | 脱敏字段映射与计数规则、独立复算方法 |
| N4b-2 纯解析 | 依赖字段语义确认；先构造合成 fixture，再实现 reader | 正常/零值/缺值/负数/异常大数、未知模型/时间、重复/累计重置、fork/resume、损坏/截断测试通过 | 纯解析模块与独立期望值 |
| N4b-3 扫描与缓存 | 依赖纯解析通过；接入现有注册和 scan 控制器，不增加重复目录项 | 深度/数量/读取量有界；热读复用；追加/替换/删除/修复可见；取消不发布完整结果 | 有界读取、缓存失效及取消回归 |
| N4b-4 样本与 UI | 依赖扫描和缓存通过；冻结相同字节独立聚合，隔离 Token 页核对 | input/output/cache/total、模型/日期桶和缺口一致；筛选、刷新、关闭重开通过 | 脱敏结论、UI 记录与程序 SHA；完成后才更新原生支持数量 |
| R2 真实账户 | DeepSeek、Kiro 每个服务商独立验收；所需登录由账户本人完成 | 同账号/日期/币种对照、过期/空态、主附加账户隔离通过；缺登录独立记录 | 每项验证状态与差异复现 |
| R3 多账户与体验 | 读取和账户基线稳定后，先 OAuth/续期与 Codex 独立进程，再快捷键/多 DPI/翻译，最后 BotMark | 凭据、缓存、撤销与进程隔离可复现；交互和文字覆盖分别验收 | 分批实现与验证记录 |

每个 reader 或账户链路单独提交。发布前完成 R1 剩余检查，再确定版本、更新锁文件与发布说明，核对 tag、EXE、包名和校验文件；本计划不把未发布功能写成已发布版本。
