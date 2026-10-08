# 开发与验收

## 环境与命令

只在 macOS 与 Linux 开发和验证，不维护 Windows 支持。

- Rust `1.99.0`，通过 rustup 安装；`rust-toolchain.toml` 声明版本及 rustfmt、Clippy。
- Node.js `22.14.0`，版本记于 `.node-version`；npm 随 Node 安装。
- Git；与 GitHub 交互时使用已登录的 `gh`。

在仓库根目录执行。Cargo 使用提交的 `Cargo.lock`，npm 使用 `web/package-lock.json`，CI 和验收均不更新锁文件。

| 目的 | 命令 |
| --- | --- |
| Rust 格式检查 | `cargo fmt --all -- --check` |
| Rust 静态检查 | `cargo clippy --all-targets --locked -- -D warnings` |
| Rust 行为测试 | `cargo test --all-targets --locked` |
| Rust 构建 | `cargo build --locked` |
| 安装前端锁定依赖 | `npm ci --prefix web` |
| 前端类型、静态与格式检查 | `npm run check --prefix web` |
| 前端行为测试 | `npm run test --prefix web` |
| 前端构建 | `npm run build --prefix web` |
| 前后端构建集成检查 | `node scripts/smoke.mjs` |
| 完整工程检查（先安装前端依赖） | `./scripts/check.sh` |

自动格式化使用 `cargo fmt --all` 和 `npm run format --prefix web`。Rust 构建与测试可独立执行，无需 Node 或前端静态资源。

首次克隆或锁文件更新后，先执行 `npm ci --prefix web`，再运行完整工程检查。集成检查启动 `target/debug/rig`，使用操作系统分配的端口和构建后的绝对静态目录，从本次子进程读取监听地址，验证应用信息、HTML 与引用的 JavaScript/CSS 资源；检查完成后发送 SIGINT 并验证正常退出。脚本不验证浏览器中的实际组件渲染，修改 UI 后还需在浏览器查看。

本地使用与前端开发的启动命令见 [README](../README.md)。`serve` 默认从当前工作目录下的 `web/dist` 提供静态文件。开发模式使用 Vite 的 API 代理；构建模式由 Rust 直接提供页面。

## 初始化验收

- 新克隆仓库按文档安装依赖后，可以运行完整工程检查。
- CLI 的帮助、版本和本地服务启动可用。
- Web 构建产物由 Rust 提供，页面通过真实 `/api/info` 接口显示名称与版本。
- Vite 开发页面可以读取相同的 API。
- macOS 与 Linux CI 检查 Rust 格式、Clippy、测试与构建，以及前端检查、测试、构建与构建产物的 HTTP 集成。
- 文档、实现、数据契约和实际命令保持一致；没有伪装已实现的 Agent 功能或预占未来能力的空模块。

改动后执行与影响范围相称的检查。跨 Rust/Web 边界、依赖或工程入口的变更执行完整检查，并验证实际前后端连接。测试应能发现行为错误，而非复述实现。必要检查通过后，只有新的变更、失败或未解决问题才需要重复或扩展验证。

## 设计与协作

遵循 [AGENTS.md](../AGENTS.md)。核心设计、数据模型、模块边界、重要依赖与阶段范围需先解释取舍并取得维护者确认；常规实现自主完成。当前有效的设计结论更新到 [架构](architecture.md)，不保存设计过程和 ADR 流水记录。

关键设计确认后，为独立任务写明职责、接口与验收标准，再分配给不同 worktree 中的子 Agent。任务负责人集成变更并执行整体检查；不能以各子任务通过检查替代集成验证。

## Git 与 PR

以 `main` 为主分支，开发在独立 worktree 的主题分支完成。示例：

```sh
git fetch origin
git worktree add -b feat/example ../rig-example origin/main
```

按主题组织提交，遵循 Conventional Commits，如 `feat: ...`、`fix: ...`、`docs: ...`、`chore: ...`。不添加 Codex 联名，不暂存或提交 `docs/superpowers/` 产物。

允许一个 PR 包含主题一致的较大变更；必要时用开发分支集成相关工作后整体交付。PR 描述说明具体问题、最终行为、关键设计与验证结果，标明实际限制。通过 PR 审查和 CI 后合入 `main`，合入后清理完成工作的 worktree 与主题分支。
