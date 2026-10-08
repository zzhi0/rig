# Rig

Rig 是一个开源、面向个人使用的本地 Agent harness，也是学习 Agent 开发和积累 AI 产品基础能力的实践项目。采用 Rust 内核，CLI 负责启动与管理，Web UI 是首个交互入口。内核保持任务中立，为后续个人任务和开发任务留出空间。

当前交付工程基础：可构建的 Rust 与 Web 工程、本地 HTTP 服务、应用信息页面，以及统一的工程检查。模型接入、Agent 执行、工具调用和会话持久化尚未实现。

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
