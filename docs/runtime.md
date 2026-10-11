# Agent Runtime

## 当前范围

本版交付可独立运行和测试的 Rust 单 Agent Loop：一次用户委托，调用 DeepSeek，按顺序执行工具并回传结果，直到得到答案、失败或取消。CLI `run` 是测试入口；Web UI 仍提供工程基础页面。

执行状态和上下文只保存在内存中。首个工具是 `read_file`，只读取用户通过 `--read FILE` 明确提供的 UTF-8 文件，不写文件或执行命令。[产品文档](product.md) 中的会话、权限模式、成果、知识与远程交互留到对应能力阶段；本版不实现审批等待、运行中追加消息、并行工具、持久化、上下文压缩、插件、多 Agent 或工作流。

## 职责与契约

| 模块 | 职责与边界 |
| --- | --- |
| `src/runtime.rs` | 拥有执行顺序、工具结果关联与终止语义；定义模型和工具的最小调用契约、运行事件及结果。没有 HTTP、CLI 或 UI 依赖。 |
| `src/deepseek.rs` | 将核心消息与工具定义转换为 DeepSeek API 请求，校验完整响应；负责 HTTP、鉴权与提供商格式。 |
| `src/tools.rs` | 定义和执行当前只读文件工具，检查调用参数及明确提供的文件范围。 |
| `src/main.rs` | 解析 CLI、读取环境变量、组装模型与工具，把 Ctrl-C 转为取消信号，把运行结果转为输出和退出码。 |

`Model::complete` 接收上下文及工具定义，返回完整 `AssistantMessage` 或错误。`Tools::definitions` 发布实际可执行的工具；`Tools::execute` 接收一个 `ToolCall`，返回文本结果或工具错误。这两个契约同时用于真实实现与确定性测试替身，不构建通用插件注册体系。

核心消息为 `User`、`Assistant`、`Tool`，没有提供商的 role 字段或 UI 协议。`AssistantMessage` 包含可选文本和工具调用；调用包含 ID、名称及原始 JSON 参数；`ToolResult` 包含调用 ID、内容和 `is_error`。工具参数的 JSON schema 属于工具定义，具体参数由工具解析。

## 一次执行

```mermaid
stateDiagram-v2
    [*] --> CallingModel: 用户委托
    CallingModel --> ExecutingTool: 完整响应含工具调用
    ExecutingTool --> ExecutingTool: 顺序执行下一项
    ExecutingTool --> CallingModel: 本批结果全部回传
    CallingModel --> Completed: 完整最终答案
    CallingModel --> Failed: 网络或协议失败
    CallingModel --> Cancelled: 取消
    ExecutingTool --> Cancelled: 取消
    Completed --> [*]
    Failed --> [*]
    Cancelled --> [*]
```

工具调用失败仍生成结果，继续同批剩余调用，再交给模型决定下一步。模型自行修正参数属于正常循环，不是 Runtime 自动重试。没有默认次数上限；需要结束尚未完成的委托时取消。

```mermaid
sequenceDiagram
    participant C as CLI
    participant R as Runtime
    participant M as DeepSeek 适配器
    participant T as 文件工具
    C->>R: 委托、明确提供的文件、取消信号
    R->>M: 用户消息与工具定义
    M-->>R: read_file 调用（ID、参数）
    R->>T: 执行调用
    T-->>R: 内容或错误
    R->>M: 用户消息、Assistant 调用、关联的 Tool 结果
    M-->>R: 最终答案
    R-->>C: Completed 与运行上下文
```

Runtime 在执行整批工具前检查调用 ID 非空且在本批内唯一；未知工具不会被分派，生成明确的错误结果。每项调用完成后立即把结果追加到上下文，全部完成后才发起下一次模型请求。工具顺序等于模型返回的顺序，部分失败不会跳过其他项。

`RunOutcome` 只有 `Completed`、`Failed`、`Cancelled`。最终响应没有答案和工具调用时失败；适配器把截断等未完成响应视为失败，不把部分答案当作成功。网络或协议失败不重试。

取消与当前模型或工具 future 竞争，信号已就绪时优先取消。取消会丢弃等待中的 future，停止后续工具和模型调用，保留已经提交到内存上下文的结果。不能据此承诺远端推理停止、文件读取的底层任务立即停止或副作用回滚。未来添加写入或进程工具时，必须先定义它们的取消和收尾契约。

本版不支持继续一个中断的运行。再次执行 `run` 发起新委托，不能直接把取消后的不完整工具批次当作下一次模型输入。

## 上下文、记录与展示

`RunReport` 返回最终结果及本次运行的模型上下文，便于检查。它不是会话历史、持久执行日志或恢复快照。完整工具结果进入本次上下文，没有默认摘要或截断；上下文过长导致 API 失败时明确报错，预算与压缩另行设计。

`RunEvent` 通知模型开始与完成、工具开始与完成、运行结束。事件是同步回调中的借用数据，CLI 用它显示进展；它不承担存储、重放或客户端传输协议。核心事实不依赖终端渲染。接入 Web 时再定义可读取状态和展示数据的转换，不预建广播、重连或历史回放接口。

## DeepSeek 与文件范围

使用官方 `https://api.deepseek.com/chat/completions`，从 `DEEPSEEK_API_KEY` 读取凭据。默认模型为 `deepseek-flash`，可用 `--model` 明确选择官方模型名；不提供第三方 endpoint 或兼容回退。

请求为非流式，明确关闭 thinking，避免在首版引入 token 流装配和推理内容回传。工具错误在适配边界编码为明确的错误内容，并使用 `tool_call_id` 关联。HTTP 客户端关闭自动重试和重定向；不引入自定义超时策略。

`--read` 在启动时解析成规范绝对文件路径，工具定义列出可读路径，执行时只接受这些路径。模型无法用其他路径扩大读取范围。这是本版的明确文件选择契约，不是操作系统隔离；不承诺阻止运行期间外部修改文件或替换符号链接。

## 取舍与验证

选择自研显式循环，便于直接 review 模型、工具和取消如何推进。图驱动或持久执行 Runtime 可以组织复杂等待与恢复，SDK 可以减少提供商或调度实现，但会增加尚未需要的状态、依赖和语义映射。本版用两个实际可测试的调用边界保持模块独立，暂不引入这些机制。

验收关注完整行为：

- 确定性模型和工具验证多轮调用、顺序、结果 ID、部分失败及正常结束。
- 网络或协议错误结束执行，不产生隐式重试；无效的调用 ID 不触发工具。
- 在模型等待、工具等待及同批工具之间取消，后续工作不启动，已完成结果保留。
- 本地 HTTP 测试检查实际请求格式、工具结果回传、鉴权错误及响应结束原因。
- 临时真实文件测试验证明确提供的文件可读，其他路径和无效参数返回错误。
- CLI 检查凭据缺失、文件准备错误与帮助；真实 DeepSeek 验证直接回答及读取文件后再次调用模型。真实验证使用仓库外的凭据，CI 不依赖 key。

实现可按模型适配、工具实现、Runtime 行为测试独立分工，共同依赖 `runtime.rs` 的契约；CLI 集成依赖真实模型与工具，最终工程检查由集成负责人完成。修改调用契约时同步行为测试、适配器和本文。

## 一手依据

资料核实日期：2026-10-12。

- [DeepSeek 官方接口](https://api-docs.deepseek.com/)、[工具调用](https://api-docs.deepseek.com/guides/tool_calls)、[Thinking Mode](https://api-docs.deepseek.com/guides/thinking_mode)：官方地址、模型名、调用 ID 回传与 thinking 开关。
- [Pi 执行循环](https://github.com/earendil-works/pi/blob/ce950d78f424dcaf9f5d6a03ce80ab141130eb1d/packages/agent/src/agent-loop.ts)：直接推进模型和工具的参考。本版没有采用其运行中消息、扩展和会话机制。
- [DSH 工具调度](https://github.com/deepseek-ai/deepseek-harness/blob/5badb15009ae1756c3afe0ae0cef1faafc290ccc/packages/core/agent-loop/src/tool-calls.ts)：工具结果与调度边界的参考。本版采用顺序执行。
