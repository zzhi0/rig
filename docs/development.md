# 开发与验收

## 环境与命令

只在 macOS 与 Linux 开发和验证，不维护 Windows 支持。

- Rust `1.99.0`，通过 rustup 安装；`rust-toolchain.toml` 声明版本及 rustfmt、Clippy。
- Node.js `22.14.0`，版本记于 `.node-version`；npm 随 Node 安装。
- Git；与 GitHub 交互时使用已登录的 `gh`。

在仓库根目录执行。Cargo 使用提交的 `Cargo.lock`，npm 使用 `web/package-lock.json`，验收不更新锁文件。验证在本地进行，不接入或依赖 CI。

| 目的 | 命令 |
| --- | --- |
| Rust 格式检查 | `cargo fmt --all -- --check` |
| Rust 静态检查 | `cargo clippy --all-targets --locked -- -D warnings` |
| Rust 行为测试 | `cargo test --all-targets --locked` |
| 指定 Rust 行为测试 | `cargo test --locked --test runtime TEST_NAME` 或 `cargo test --locked --lib TEST_NAME` |
| Rust 构建 | `cargo build --locked` |
| 安装前端锁定依赖 | `npm ci --prefix web` |
| 前端类型、静态与格式检查 | `npm run check --prefix web` |
| 前端行为测试 | `npm run test --prefix web` |
| 前端构建 | `npm run build --prefix web` |
| 前后端构建集成检查 | `node scripts/smoke.mjs` |
| 完整工程检查（先安装前端依赖） | `./scripts/check.sh` |

自动格式化使用 `cargo fmt --all` 和 `npm run format --prefix web`。Rust 构建与测试可独立执行，无需 Node 或前端静态资源。

首次克隆或前端锁文件更新后，先执行 `npm ci --prefix web`。完整工程检查保留给阶段性集中验证或 review 后的合并验收。集成检查启动 `target/debug/rig`，使用操作系统分配的端口和构建后的绝对静态目录，从本次子进程读取监听地址，验证应用信息、HTML 与引用的 JavaScript/CSS 资源；检查完成后发送 SIGINT 并验证正常退出。脚本不验证浏览器中的实际组件渲染，修改 UI 后还需在浏览器查看。

本地使用与前端开发的启动命令见 [README](../README.md)。`serve` 默认从当前工作目录下的 `web/dist` 提供静态文件。开发模式使用 Vite 的 API 代理；构建模式由 Rust 直接提供页面。

## Runtime 验收

机制与当前范围见 [Runtime](runtime.md)。`cargo test --all-targets --locked` 包含确定性 Loop 测试、合成 OAuth 凭据与本地 HTTP 模型适配测试、真实临时文件工具测试及 CLI 测试，无需账号、凭据或外网；前端仍按现有工程验收。

真实模型验证按 [README](../README.md) 登录 ChatGPT 并选择账户可用的模型：先检查无工具的回答，再明确要求使用 `read_file` 读取一个提供的文件，确认 stderr 出现工具进展、最终答案使用文件内容。用 Ctrl-C 确认取消及退出码 `130`。登录需要维护者在浏览器完成授权；离线测试不能证明真实账号的授权或模型可用性。不要把 token、真实请求日志或临时输入文件提交到仓库。

## 初始化验收

- 新克隆仓库按文档安装依赖后，可以运行完整工程检查。
- CLI 的帮助、版本和本地服务启动可用。
- Web 构建产物由 Rust 提供，页面通过真实 `/api/info` 接口显示名称与版本。
- Vite 开发页面可以读取相同的 API。
- 本地完整检查覆盖 Rust 格式、Clippy、测试与构建，以及前端检查、测试、构建与构建产物的 HTTP 集成；在 macOS 或 Linux 上记录实际验证的平台。
- 文档、实现、数据契约和实际命令保持一致；没有伪装已实现的 Agent 功能或预占未来能力的空模块。

开发中先检查 diff、格式和受影响的行为；合并一批相关改动后，再按需编译或运行针对性测试，避免每次编辑都触发 Rust 编译、Clippy、构建或全量测试。文档等低影响改动只检查内容与 diff。

阶段性集中验证或 review 完、准备合并时，再运行完整工程检查。跨 Rust/Web 边界、依赖或工程入口的改动在该阶段补齐集成验证。测试应能发现行为错误，而非复述实现；已有通过结果只在新增改动、失败或未解决问题需要时复查，交付时如实说明未运行的检查。

## 实现与 Review

以维护者能读懂和审查为标准，遵循 [AGENTS.md](../AGENTS.md) 的 KISS 原则：

- 每个模块和文件承担明确职责，依赖方向、状态归属和执行流程可直接追踪；按实际职责划分，不机械限制文件大小或给每个类型添加接口。
- review 核心设计及行为，检查是否用临时特例掩盖职责或数据模型问题；在已确认范围内把问题解决清楚。
- 新功能和机制必须对应当前需求。只保留必要的输入检查、错误处理与资源清理，不为假想场景增加分支。
- 参考 [Linux 内核编码风格](https://docs.kernel.org/process/coding-style.html) 中短函数、单一职责、浅嵌套与清楚控制流的要求；Rust 的具体格式由 rustfmt 决定。
- 删去重复代码和解释显然操作的注释，保留影响正确性或理解的契约与理由。代码标识、文件名使用英文，注释和 Rustdoc 可用中文，不批量翻译已有英文注释。

## 设计与协作

遵循 [AGENTS.md](../AGENTS.md)。核心设计、数据模型、模块边界、重要依赖与阶段范围需先解释取舍并取得维护者确认；常规实现自主完成。当前有效的设计结论更新到 [架构](architecture.md)，不保存设计过程和 ADR 流水记录。

关键设计确认后，为独立任务写明职责、接口与验收标准，再分配给不同 worktree 中的子 Agent。子任务按影响范围验证，任务负责人集中执行阶段或合并验收；不能以各子任务通过检查替代集成验证，也不让各子任务重复跑全量。

## Git 与 PR

以 `main` 为主分支，开发在独立 worktree 的主题分支完成。示例：

```sh
git fetch origin
git worktree add -b feat/example ../rig-example origin/main
```

按主题组织提交，遵循 Conventional Commits，如 `feat: ...`、`fix: ...`、`docs: ...`、`chore: ...`。不添加 Codex 联名，不暂存或提交 `docs/superpowers/` 产物。

允许一个 PR 包含主题一致的较大变更；必要时用开发分支集成相关工作后整体交付。PR 描述说明具体问题、最终行为、关键设计与验证结果，标明实际限制。通过 PR 审查和本地合并验收后合入 `main`，合入后清理完成工作的 worktree 与主题分支。
