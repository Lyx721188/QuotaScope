# Windows 浏览器 Cookie 读取

Windows 的“从浏览器导入”可读取指定网站和指定 Cookie 名称。适用路径见 `windows/quotascope-core/src/browser_cookies.rs`；localStorage 的读取走独立模块，规则见各服务商说明。

优先只读打开浏览器数据库。打开失败需要副本时，每次读取都会原子创建独立临时目录，目录名称不复用 profile 名称。主库以及存在的 WAL/SHM 各自复制成功后才尝试读取；任何复制失败均停止，不读取此前残留的副本。

副本由读取作用域持有：数据库连接先关闭，再删除该次副本的数据库、WAL、SHM、journal 和目录。清理只处理自身固定文件，不递归删除其他临时材料，也不修改原浏览器数据库。进程异常终止可能留有临时文件；后续读取使用新的目录，不复用它们。

复制活跃数据库的多个文件不能据此称为一致的事务快照。锁、读取失败或无法解密时仍保持未找到/未知状态。Chromium 的 `v10`/`v11` AES-GCM 和 Firefox 的普通 Cookie 读取沿用现有实现；`v20` App-Bound 数据仍不支持。导入找到凭据不等于服务商请求或真实账户已验收。

合成回归覆盖同名 profile 的重叠读取、复制失败/部分副本清理、sidecar 内容与生命周期，以及原有 wildcard、子域和 App-Bound 规则。只使用临时假数据库，不读取真实浏览器凭据：

```powershell
cargo test --workspace --lib browser_cookies::tests --locked -j 1
```

本说明对应 main 的未发布改动；实际发布与安装升级单独验收。
