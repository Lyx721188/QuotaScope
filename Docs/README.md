# Docs

这里保留 Windows 版本仍在使用的服务商资料、接口说明和行为约定。应用源码和构建
入口统一位于 [`windows/`](../windows/)；仓库不再包含 macOS/Swift 实现。

## 开始阅读

| 文档 | 内容 |
|---|---|
| [json-output.md](json-output.md) | `quotascope.exe --json` 的状态栏输出约定 |
| [providers/README.md](providers/README.md) | 各服务商的读取通道、鉴权和本地数据来源 |

用户入口是仓库根目录的 [`README.md`](../README.md)。开发、测试和发布以
[`windows/README.md`](../windows/README.md) 及 GitHub Actions 为准。
