# Kiro Windows 读取与配置

本次基于已合并 Windows 1.2.4（`d9a1644`），移植 Pulse
[`3696a65`](https://github.com/qunqin24/Pulse/tree/3696a65b428272aa25c3ba611de8df2536983515)
的 [KiroUsageService](https://github.com/qunqin24/Pulse/blob/3696a65b428272aa25c3ba611de8df2536983515/Sources/Pulse/Providers/KiroUsageService.swift)
与 [KiroACPClient](https://github.com/qunqin24/Pulse/blob/3696a65b428272aa25c3ba611de8df2536983515/Sources/Pulse/Providers/KiroACPClient.swift)。
Kiro 与 xKiro 是不同 provider，本页只涉及 `kiro`。

## 配置

1. 按 [Kiro 官方安装说明](https://kiro.dev/docs/getting-started/installation/)
   安装原生 Kiro CLI，使 `kiro-cli.exe` 可从 Windows `PATH` 找到。
   官方当前列出的原生 CLI Windows 要求为 Windows 11；QuotaScope 本身的系统要求不变。
2. 在终端运行 `kiro-cli login`，由 Kiro 完成登录。
3. 打开 QuotaScope 设置 → 账户 → Kiro，启用账户；展开配置后点击“刷新”。
   此处没有 API key/Cookie 输入框，不在 QuotaScope 中保存 Kiro 凭据。

读取只支持主账户。沿用 `enabledAccounts` 中的 `kiro`，不增加设置 schema 或附加账户登录。
不会根据 CLI 已安装就自动启用，已有设置继续兼容。
寻找位置依次为 `PATH`、用户 `.local/bin`、用户 `bin` 中的 `kiro-cli.exe`。
仅有 IDE 或 WSL 中的 CLI 不等于 Windows 原生 CLI 路由可用；本实现不会自动启动 WSL。

## 读取与字段

每次刷新启动隐藏的 `kiro-cli acp --agent-engine v3 --auth-method cli`。
参数与标准输入/输出形式符合 [官方 ACP V3 迁移说明](https://kiro.dev/docs/cli/v3/acp-migration/)。
先等待 `initialize` 的响应，再请求 `_kiro/account/getUsage`；不新建会话、不发送 prompt。
每次 RPC 最多等待 20 秒，stdout 单行最多 1 MiB、整次最多 8 MiB，stderr 丢弃以免阻塞或记录敏感信息。
连接结束、失败或超时都会回收子进程；Windows Job 同时关闭 CLI 派生的辅助进程。
运行目录为 `%APPDATA%\QuotaScope\KiroACP`，不以当前项目作为工作目录。

用量方法依据上游实测实现，未宣称它是官方公开、稳定的额度 API。
上游指出该方法未出现在公开 `extensionMethods` 列表中；不支持时显示版本提示，不抓取终端文字作为回退。

严格按上游 envelope 解码 `success`、`message`、`data`，以及以下字段：

| 字段 | Windows 行为 |
|---|---|
| `planName` | 显示套餐原文 |
| `billingCycleReset` | 仅接受 `yyyy-MM-dd` UTC 日期；缺失/非法时保持未知 |
| `usageBreakdowns` | 每个有明确上限的额度池各绘制一个月度窗口 |
| `used`、`limit` | 优先以已报告 `used / limit` 计算；`limit` 必须大于 0 |
| `percentage` | 仅在 `used` 缺失且存在有效 `limit` 时作为上游规定的回退 |
| `hasLimit: false` | 不绘制该额度池；缺少分母不猜百分比 |
| `resourceType`、`displayName` | 稳定 ID 优先使用资源类型，空白时回退显示名；重复资源加序号 |

数字字符串、错误布尔类型或缺失必需结构会令整份回复不可读，不静默使用部分正确字段。
窗口填充限制在 0–100%，耗尽状态依据未截断的 `used >= limit`。
30 天只是月度排序键，`reports_length=false`；不推算周期进度、燃烧速率、Token 数或费用。
明确未登录、未安装和版本不支持时，旧缓存不能覆盖故障提示。
JSON 读数来源为 `kiroACP`，不添加猜测的网页版用量链接。

## 验证边界

- [解析 fixture](../../windows/quotascope-core/tests/fixtures/kiro-pro-plus-usage.json)
  原样来自上述上游提交，重置日期为 `2099-10-01`。它是上游测试样本，未作为本机账号实际额度。
- [解析与注册测试](../../windows/quotascope-core/tests/kiro.rs) 固定读取时刻，覆盖准确比例、日期、空/无限额度、错误类型、耗尽、稳定 ID、旧配置和路由注册。
- [隔离 ACP 测试](../../windows/quotascope-core/src/kiro_acp.rs) 用测试程序充当 CLI，验证初始化顺序、通知/请求/旧 ID、stderr、错误、EOF、超时、重启和 Windows 子进程回收。
- **真实 Kiro 账号：未验证。** 本机当前未发现原生 `kiro-cli.exe`；不安装 CLI、不触发登录，未用真实账号请求。上游的账号验证不能替代 Windows 验证。
- 设置入口已接入并参与 Windows 编译；真实 Kiro 账户的启用/刷新交互尚未人工验收。

2026-10-03 本轮最终源码验证：`cargo fmt --all -- --check`、CI 同款
`cargo clippy --workspace --all-targets --locked`、`cargo test --workspace --locked`
（614 passed、6 ignored）及 `cargo build --release --locked` 均通过。
Clippy 保留既有风格告警，没有 Kiro 新文件告警。
发布版在隔离空账户 profile 的 `--json` 冒烟通过。
Debug 构建的真实托盘设置打开、关闭、重开、重复请求、第二次启动和隐藏退出回归通过（38.14 秒）。
本机发布版桌面测试首次遇到正在运行的安装版单实例锁并正常退出；
1.2.4 只有 Debug 构建支持测试隔离命名空间，因此该次失败保留为独立记录，
不记为发布版桌面验证成功。未打包、安装或发布本次 Kiro 改动。
日志在本工作树 `windows/work/kiro-validation-20261003/`，不纳入 Git。

实际账号验收需要在支持的 Windows 系统安装并登录 Kiro CLI，再在设置中启用/刷新，
把套餐、各额度池和重置日期与同一账号的 Kiro `/usage` 页面比较。
对应缺失/拒绝读数保持未知，不补为 0。

项目相关检查从 `windows/` 运行：

```powershell
cargo test -p quotascope-core --test kiro --locked
cargo test -p quotascope-core kiro_acp::tests --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked
cargo test --workspace --locked
cargo build --release --locked
```
