# Rig

Rig 是一个开源、面向个人使用的本地 Agent harness，也是学习 Agent 开发和积累 AI 产品基础能力的实践项目。采用 Rust 内核，CLI 负责启动与管理，Web UI 是首个交互入口。内核保持任务中立，为后续个人任务和开发任务留出空间。

当前交付工程基础与最小 Agent Runtime：通过 CLI 使用 DeepSeek 官方 API 执行一次委托，按顺序调用只读文件工具，支持失败报告与 Ctrl-C 取消。Web UI 当前仍是应用信息页面，会话与持久化尚未实现。

仅支持 macOS 和 Linux，永不支持 Windows。许可证为 [MIT](LICENSE)。

## 本地运行

安装 Rust `1.99.0`（通过 rustup）和 Node.js `22.14.0`。Rust 工具链由 `rust-toolchain.toml` 指定，Node 版本见 `.node-version`。npm 随 Node 安装。

```sh
npm ci --prefix web
npm run build --prefix web
cargo run --locked -- serve
```

打开 <http://127.0.0.1:7878/>。服务默认只监听本机，静态文件目录为当前工作目录下的 `web/dist`，请在仓库根目录运行上述命令。

```sh
cargo run --locked -- --help
cargo run --locked -- --version
```

Rust 工程可以单独构建与测试，无需安装 Node 或生成 Web 资源。

## 运行 Agent

将 DeepSeek key 保存在仓库外的 `~/.config/rig/deepseek-api-key`（一行 key），再读取到环境变量，避免写进命令历史：

```sh
read -r DEEPSEEK_API_KEY < ~/.config/rig/deepseek-api-key
export DEEPSEEK_API_KEY
cargo run --locked -- run "用中文简要解释 Agent Loop"
cargo run --locked -- run "请用 read_file 读取提供的文件，再概括 Rig 当前的能力" --read README.md
```

默认调用官方 `https://api.deepseek.com/chat/completions`，模型为 `deepseek-flash`，非流式且关闭 thinking。可通过 `--model` 指定官方模型名；重复 `--read FILE` 提供多个可读文件。不提供文件时没有工具，模型直接回答。

答案写入 stdout，进展与错误写入 stderr。Ctrl-C 取消本地等待和后续调度，退出码为 `130`；正常完成为 `0`，失败为 `1`。每次命令是独立委托，不保存会话，也不恢复取消的执行。机制与边界见 [Runtime](docs/runtime.md)。

## 开发与检查

开发 Web UI 时，在两个终端分别运行：

```sh
cargo run --locked -- serve
```

```sh
npm ci --prefix web
npm run dev --prefix web
```

打开 <http://127.0.0.1:5173/>。Vite 将 API 请求代理到 Rust 服务，提供前端即时更新。

完整工程检查：

```sh
npm ci --prefix web
./scripts/check.sh
```

架构与边界见 [架构](docs/architecture.md)，检查细节、验收与 PR 流程见 [开发指南](docs/development.md)。参与开发前请阅读 [AGENTS.md](AGENTS.md)。
