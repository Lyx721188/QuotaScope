# QuotaScope for Windows

QuotaScope 是一个原生 Windows 屏幕边缘 AI 编码额度监视器，用 Rust 编写，运行在
Win32、Direct2D 和 WinUI 3 之上。它直接读取各服务商自己的客户端通道，展示
剩余额度、重置倒计时、消耗速率和耗尽预测；无 QuotaScope 后端、无 QuotaScope 账号、无遥测。

[![Windows 构建](https://github.com/Lyx721188/QuotaScope/actions/workflows/windows.yml/badge.svg)](https://github.com/Lyx721188/QuotaScope/actions/workflows/windows.yml)
![Windows 10/11](https://img.shields.io/badge/Windows-10%201809%2B%20%2F%2011-0078D4?logo=windows&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-stable-DEA584?logo=rust&logoColor=white)
[![许可证](https://img.shields.io/badge/许可证-Apache%202.0-blue)](LICENSE)

## 特性

- 屏幕边缘停靠栏、悬停详情卡、托盘图标和 WinUI 3 设置窗口
- 真 Mica、DWM 圆角与边框、深浅色跟随系统、系统强调色
- 自适应刷新、缓存、通知和 `quotascope.exe --json` 状态输出
- API Key 和会话凭据使用 Windows DPAPI 加密，仅当前系统用户可解密
- Provider 图标和本地化资源随 Windows 应用一起发布
- 托盘逐账户用量摘要和用量页入口；设置页支持账户搜索、订阅/API 分组和凭据明文切换
- 每账户余额口径、预算与低余额提醒；扩展程序可报告自己的用量
- 可选的 Claude Code/Codex/Antigravity 本机 token 消耗分析，以及 z.ai/智谱近 30 天用量历史

## 支持的服务商

内置目录包含 77 个 provider，其中 67 个已实现 Windows 读取路由。API key、
自建网关和 18 个浏览器会话路由均可在账户设置中配置。路由实现和解析测试
不代表所有服务商都经过真实账号验证；未实现的 10 个路由仍在设置中明确标注。

Windows 完整清单见 [`Docs/providers/windows-ports.md`](Docs/providers/windows-ports.md)，
新功能操作说明见 [`Docs/windows-1.2.md`](Docs/windows-1.2.md)，
扩展契约见 [`Docs/extensions.md`](Docs/extensions.md)。

## 下载与构建

从 [Releases](https://github.com/Lyx721188/QuotaScope/releases) 下载 Windows x64 压缩包，
解压后运行 `quotascope.exe`。每次推送都会由
[Windows build](https://github.com/Lyx721188/QuotaScope/actions/workflows/windows.yml)
构建；推送 `windows-v*` 标签时自动发布 Release。

本地构建需要 Rust stable 和 MSVC 工具链：

```bash
git clone https://github.com/Lyx721188/QuotaScope.git
cd QuotaScope/windows
cargo fmt --all -- --check
cargo test --workspace
cargo build --release
```

输出位于 `windows/target/release/quotascope.exe`。`quotascope.exe --json` 只读取缓存，不发起
网络请求；JSON 字段约定见 [`Docs/json-output.md`](Docs/json-output.md)。

本地 EXE 安装包可通过 Inno Setup 6 构建，提供当前用户安装、开始菜单入口和系统卸载入口。
安装及打包步骤见 [`Docs/installing-windows.md`](Docs/installing-windows.md)。

## 隐私与安全性

- QuotaScope 不提供自有服务器、账号或遥测服务。
- 应用请求已配置服务商的用量接口，或读取本机已登录工具的状态。
- 启用“Token spend”后会在本机扫描已支持的 Claude Code、Codex、Qwen Code、Gemini CLI、
  Pi、Oh My Pi、OmO Native、Kimchi、Amp、Droid、Prime Agent、OpenClaw、Mux、
  Junie、Augment、JCode、Gajae Code、Codebuff、FX、Reasonix、LM Studio
  会话或日志文件，
  只解析 token 计数、
  模型和时间；不会上传会话正文。开启账户详细卡后，Antigravity 还会读取本机
  `~/.gemini/antigravity/conversations` 会话数据库中的用量元数据，同样不读取正文。
  定价功能会下载 models.dev 的公开价目表，超过 24 小时后在下次读取时自动更新，
  离线时使用本地缓存；models.dev 没有公开价目的模型单独标记，不会用相近模型的价格代替。
- 浏览器 cookie 仅在点击“从浏览器导入”时读取，并用 DPAPI 保存；扩展程序只在启用后运行。
- QuotaScope 不上传源码、Prompt 或模型生成内容。启用的扩展程序使用自己的网络和登录通道。

## 许可

本项目遵循 [Apache 2.0](LICENSE) 开源许可。第三方图标和依赖的许可见
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。服务商名称和商标归其所有者所有，
仅用于标识兼容服务，不构成背书。
