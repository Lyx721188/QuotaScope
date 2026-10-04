# Windows 安装包

本地打包命令生成 `QuotaScope-<版本>-windows-x64-Setup.exe`，适用于 Windows 10 1809+
和 Windows 11 x64。安装包包含 WinUI 运行库、PRI 索引、语言资源和字体。

## 安装与使用

1. 双击 `QuotaScope-<版本>-windows-x64-Setup.exe`，按向导完成安装。
2. 默认安装到 `%LOCALAPPDATA%\Programs\QuotaScope`，使用当前用户权限。
3. 完成时可勾选 `Launch QuotaScope` 打开设置。也可在开始菜单中打开 `QuotaScope`
   或 `QuotaScope Settings`；桌面快捷方式是可选项。
4. 运行后可以右键托盘图标进入设置；再次启动程序也会打开已有实例的设置。
5. 开机启动在应用设置中启用。

从 Windows 设置的“已安装的应用”中选择 QuotaScope 卸载。卸载会移除程序和快捷方式，
并移除指向本次安装程序的 QuotaScope 开机启动项。账户设置和加密凭据保留在
`%APPDATA%\QuotaScope`，便于重装。

当前本地安装包未使用商业代码签名证书。若 Windows 显示发布者未知，先确认文件来源；
同目录 `.sha256` 文件用于校验下载或复制是否完整。

## 本地打包

需要 Rust stable、MSVC 工具链和 [Inno Setup 6](https://jrsoftware.org/isdl.php)。
在仓库根目录运行 PowerShell：

```powershell
./windows/installer/build-installer.ps1
```

脚本先编译当前源码，核对 EXE 版本与 Cargo 工作区版本，再复制运行资源并生成安装器。
默认产物位于 `windows/target/installer`，含安装 EXE、SHA256 文件和用于核验的 payload 目录。
需要指定编译器或输出目录时：

```powershell
./windows/installer/build-installer.ps1 -CompilerPath 'C:/Tools/Inno Setup 6/ISCC.exe' -OutputDirectory 'D:/Builds/QuotaScope'
```

`-SkipBuild` 仅适用于刚完成当前源码的 release 构建；版本号核对不能判断同版本二进制
是否包含最近改动。安装器源码为 `windows/installer/QuotaScope.iss`。

此命令生成本地产物。`windows-v*` 标签的 GitHub Actions 同时生成安装包和免安装 ZIP，
执行打包后的设置窗口生命周期回归，并发布两种包及各自的 SHA-256 文件。

## 分发包自动验收

Windows CI 分别验证便携 ZIP 和安装后的 Release payload：

- ZIP 解压后核对 LICENSE、THIRD_PARTY_NOTICES.md 与源码中的哈希，再运行设置窗口生命周期。
- 使用同一 Release payload 和 Inno 脚本生成独立 AppId/目录的验收安装器；核对安装后 EXE 的版本与哈希、WinUI 资源、字体和许可证，运行托盘/设置回归，再卸载。
- 卸载后检查程序和独立卸载注册项已清理，原有安装文件未修改。CI 的 `quotascope-installer-validation` artifact 保存已有日志和成功时的 result.json。

独立身份安装验证相同程序与安装脚本，不能替代用户原安装身份的升级/回退、真实账户、多 DPI 交互和可选协议注册验收。当前状态与后续任务见[发布验收计划](release-acceptance-2026-10-04.md)。
