# Windows 浏览器 Cookie 读取

Windows 的“从浏览器导入”可读取指定网站和指定 Cookie 名称。适用路径见 `windows/quotascope-core/src/browser_cookies.rs`；localStorage 的读取走独立模块，规则见各服务商说明。

优先只读打开浏览器数据库。打开失败需要副本时，每次读取都会原子创建独立临时目录，目录名称不复用 profile 名称。主库以及存在的 WAL/SHM 各自复制成功后才尝试读取；任何复制失败均停止，不读取此前残留的副本。

副本由读取作用域持有：数据库连接先关闭，再删除该次副本的数据库、WAL、SHM、journal 和目录。清理只处理自身固定文件，不递归删除其他临时材料，也不修改原浏览器数据库。进程异常终止可能留有临时文件；后续读取使用新的目录，不复用它们。

复制活跃数据库的多个文件不能据此称为一致的事务快照。Chromium 的直接数据库读取兼容 `v10`/`v11` AES-GCM，Firefox 使用普通 Cookie 读取。

2026-11-13 增加已连接 OpenCLI Browser Bridge 的可选通道。选择“所有浏览器”后，直接数据库读取失败时会通过本机 `127.0.0.1:19825` 向已连接浏览器请求指定网站的 Cookie。浏览器自身处理受保护或锁定的 Cookie，适用于本机无法直接读取的 Chrome / Edge 登录。单独选择默认浏览器或某个浏览器时沿用其数据库通道。

桥接请求固定浏览器 profile 和网站 URL，返回后再次检查域名、Cookie 名、有效期和请求头字符。响应有容量和时间上限，桥接回复排除在 HTTP 诊断捕获之外。导入结果写入当前 Windows 用户的 DPAPI 凭据文件，保存后读回确认。

设置中的导入在后台运行，显示持续的进行中、成功或失败消息。切换浏览器、站点或保存凭据会使在途结果失效。

合成回归覆盖同名 profile 的重叠读取、复制失败/部分副本清理、sidecar 内容与生命周期，以及原有 wildcard、子域和 App-Bound 规则。只使用临时假数据库，不读取真实浏览器凭据：

```powershell
cargo test --workspace --lib browser_cookies::tests --locked -j 1
```

本说明对应 main 的未发布改动；实际发布与安装升级单独验收。
