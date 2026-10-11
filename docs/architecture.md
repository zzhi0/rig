# 架构与当前设计

## 定位与阶段边界

Rig 是本地运行、面向个人使用的 Agent harness。CLI 承担启动和管理，Web UI 是首个完整交互入口。首批用户偏开发者，但内核保持任务中立，不内置编程任务的假设；通用个人任务与开发任务的工具组合、profile 或 mode，在真实需求出现后设计。

当前交付工程基础与最小 Agent Runtime。CLI 可登录 ChatGPT 并使用套餐额度调用官方 Responses API，按顺序执行明确提供的只读文件工具，完成一次委托或报告失败、取消。页面仍从真实 HTTP API 读取应用信息，尚未接入 Agent 执行。会话模型、持久化和完整产品交互留到后续阶段；运行机制与验收见 [Runtime](runtime.md)。

仅支持 macOS 与 Linux，永不支持 Windows。采用 MIT 许可证。

## 工程组成

| 位置 | 职责 |
| --- | --- |
| `src/application.rs` | 与 HTTP、UI 无关的应用信息展示数据 |
| `src/http.rs` | HTTP 路由、传输数据转换与静态文件服务 |
| `src/main.rs` | CLI 参数、运行时与监听器的组装 |
| `src/runtime.rs` | 与 HTTP、UI 无关的单次 Agent Loop、模型与工具调用契约 |
| `src/chatgpt_auth/` | ChatGPT OAuth 登录、凭据保存、刷新及退出 |
| `src/chatgpt.rs` | 官方 Responses API、流式结果与核心消息之间的转换 |
| `src/tools.rs` | 明确提供的文件范围及只读工具执行 |
| `web/src/application/model.ts` | 应用信息类型与客户端状态 |
| `web/src/application/api.ts` | HTTP 应用信息读取 |
| `web/src/application/store.ts` | 不依赖 React 的客户端状态与订阅 |
| `web/src/ui/App.tsx` | 通过 React `useSyncExternalStore` 订阅并渲染状态 |
| `web/src/main.tsx` | 创建客户端状态实例、触发首次读取并挂载 UI |
| `scripts/` 与 `.github/workflows/` | 本地工程检查与 CI |
| `docs/` | 当前有效设计和开发规范 |

Rust 使用 Tokio 与 Axum：Tokio 提供异步运行时，Axum 将 HTTP 请求映射到应用能力。采用一个 Cargo package，减少早期 crate 间接口与发布管理成本；有实际独立复用或发布需求后再考虑拆分。

`src/lib.rs` 暴露应用、HTTP 与 Runtime 等模块；CLI binary 和 library 位于同一 Cargo package。Agent 内核可以独立测试和使用，不依赖本地 HTTP 服务或前端资源。ChatGPT 通过 reqwest 直接接入官方 OAuth 与 Responses API，JSON 转换使用 serde_json；不运行 Codex 子进程或依赖 DevKit。登录凭据独立保存，不属于任务记录或会话持久化。

Web 使用 React、TypeScript 与 Vite，便于复用浏览器组件和可视化生态。代价是维护 Rust 和 Node 两套工具链与检查。基础控件可按需采用 shadcn/ui；初始化不绑定 AI 对话运行时、Agent UI 协议或自动生成接口类型的工具。

## 状态与界面边界

下面是当前及后续功能的职责边界；本版执行模块只服务单次 CLI 委托，尚未实现会话与 Agent 客户端协议。

| 层次 | 拥有的内容 | 约束 |
| --- | --- | --- |
| 内核与应用层 | 业务事实、执行生命周期及合法状态变化 | 不依赖 React、HTTP 展示格式或终端组件 |
| 展示数据 | 面向阅读的普通数据，如进度与时间线 | 不包含组件、颜色和布局；与执行数据通过明确转换连接 |
| 客户端数据与状态 | 读取服务端数据、组织界面可消费的状态 | 独立于 UI 组件，业务事实来自服务端 |
| UI 组件 | 渲染、发出操作、输入内容与展开状态等局部交互 | 不独立维护执行完成状态或工具结果 |

浏览器刷新不应决定任务生命周期；取消执行应成为明确的应用操作。服务进程退出后的恢复行为，留到持久化能力阶段设计。

会话记录、模型上下文和界面展示分别建模。例如，工具读取长文件后，界面可展示摘要和文件入口，模型只消费相关片段，执行记录保留调用事实。它们不应被迫使用同一个聊天消息数组。

第三方 UI 或协议的数据格式在适配边界转换，不能成为内核业务模型。后续 TUI 可以复用应用能力和展示数据，Web UI 也可以更换组件实现。

## 当前运行方式与接口

生产构建后的本地运行方式是一个 Rust 进程同时提供 HTTP API 与 `web/dist` 静态文件，CLI 启动进程，浏览器访问页面。默认地址为 `127.0.0.1:7878`，本阶段不提供远程访问与部署能力。

开发时 Vite 运行于 `127.0.0.1:5173`，将 API 请求代理至 Rust 服务，以支持前端即时更新。Rust 构建与测试不依赖前端构建产物。

当前应用信息读取契约：

```http
GET /api/info
```

```json
{"name":"Rig","version":"0.1.0"}
```

客户端通过独立的数据读取模块消费该接口，状态模块保存读取结果并通知订阅者，组件负责显示状态。启动入口触发首次读取，组件挂载不决定读取生命周期。接口仅用于工程连通性验证，不预设未来 Agent 数据契约。

## 后续设计确认点

最小 Runtime 先验证模型与工具循环。接入完整 Agent 产品交互时，再确认会话、权限与审批、持久化及流式通信的范围。用真实流程验证 UI 选型：发送任务、流式输出、工具执行、等待用户操作、展示结果，并检验刷新与取消行为。

AI SDK UI、assistant-ui、AG-UI 等候选方案需根据该流程评估适配成本；富交互结果如图表、表格、表单，以及 MCP Apps 或声明式 UI 的渲染与信任边界，在相关能力阶段讨论。当前不引入插件系统、后台常驻服务或通用工作流引擎。
