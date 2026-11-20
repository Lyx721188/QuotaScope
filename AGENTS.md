# QuotaScope 项目须知

## 构建与发布

**本项目的编译一律通过 GitHub Actions 完成，不在本地构建发布产物。**

- 唯一的构建流水线是 `.github/workflows/windows.yml`（`windows-latest` runner），它依次执行 `cargo fmt --check`、`cargo clippy`、`cargo test`、`cargo build --release`、安装器打包以及一系列 UI/安装生命周期回归。
- 推送到任意分支或 PR（且改动触及 `windows/` 等路径）都会触发完整构建；需要手动跑时用 `workflow_dispatch`。
- 打 `windows-v*` 标签会在 build 之后把 zip 与安装器发布为 GitHub Release；标签必须与 `windows/Cargo.toml` 的 workspace 版本一致。
- 本地只允许做快速验证（`cargo check`/`cargo test -p ...` 之类）；正式 exe、zip、安装器产物以 CI 产出为准，不要把本地 `target/` 结果当作发布件。
