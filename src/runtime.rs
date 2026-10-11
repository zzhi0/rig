//! 单次委托的执行循环；不依赖 HTTP、CLI 或会话存储。

use std::{collections::HashSet, future::Future};

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    User(String),
    Assistant(AssistantMessage),
    Tool(ToolResult),
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssistantMessage {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 模型生成的原始 JSON 字符串，由具体工具解析。
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub call_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// 适配器只返回完整响应；网络或协议错误结束本次运行。
pub trait Model {
    fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> impl Future<Output = Result<AssistantMessage, String>> + Send;
}

/// 只发布当前实际可执行的工具。工具错误作为调用结果回传模型。
pub trait Tools {
    fn definitions(&self) -> Vec<ToolDefinition>;
    fn execute(&self, call: &ToolCall) -> impl Future<Output = Result<String, String>> + Send;
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunOutcome {
    Completed(String),
    Failed(String),
    Cancelled,
}

#[derive(Debug)]
pub struct RunReport {
    pub outcome: RunOutcome,
    /// 本次运行的模型上下文，供检查；不是会话历史或恢复日志。
    pub context: Vec<Message>,
}

#[derive(Debug)]
pub enum RunEvent<'a> {
    ModelStarted,
    ModelFinished(&'a AssistantMessage),
    ToolStarted(&'a ToolCall),
    ToolFinished(&'a ToolResult),
    Finished(&'a RunOutcome),
}

/// 按模型给出的顺序执行工具。取消会丢弃正在等待的 future，停止后续工作。
/// 工具须支持这种取消方式；本版工具只读，不承诺回滚副作用或停止远端推理。
pub async fn run(
    model: &impl Model,
    tools: &impl Tools,
    prompt: String,
    cancellation: impl Future<Output = ()>,
    mut emit: impl FnMut(RunEvent<'_>),
) -> RunReport {
    let definitions = tools.definitions();
    let mut context = vec![Message::User(prompt)];
    tokio::pin!(cancellation);

    let outcome = 'execution: loop {
        let response = tokio::select! {
            biased;
            () = &mut cancellation => break RunOutcome::Cancelled,
            response = async {
                emit(RunEvent::ModelStarted);
                model.complete(&context, &definitions).await
            } => response,
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => break RunOutcome::Failed(error),
        };
        emit(RunEvent::ModelFinished(&response));

        // 在执行整批调用前检查关联 ID，避免产生无法关联的结果。
        let mut call_ids = HashSet::new();
        if response
            .tool_calls
            .iter()
            .any(|call| call.id.is_empty() || !call_ids.insert(call.id.clone()))
        {
            break RunOutcome::Failed("model returned empty or duplicate tool call IDs".into());
        }
        context.push(Message::Assistant(response.clone()));
        if response.tool_calls.is_empty() {
            break match response.content {
                Some(content) if !content.is_empty() => RunOutcome::Completed(content),
                _ => RunOutcome::Failed("model returned no answer or tool calls".into()),
            };
        }

        for call in &response.tool_calls {
            let result = tokio::select! {
                biased;
                () = &mut cancellation => break 'execution RunOutcome::Cancelled,
                result = async {
                    emit(RunEvent::ToolStarted(call));
                    if !definitions.iter().any(|tool| tool.name == call.name) {
                        return Err(format!("unknown tool: {}", call.name));
                    }
                    tools.execute(call).await
                } => result,
            };
            let result = match result {
                Ok(content) => ToolResult {
                    call_id: call.id.clone(),
                    content,
                    is_error: false,
                },
                Err(error) => ToolResult {
                    call_id: call.id.clone(),
                    content: error,
                    is_error: true,
                },
            };
            emit(RunEvent::ToolFinished(&result));
            context.push(Message::Tool(result));
        }
    };

    emit(RunEvent::Finished(&outcome));
    RunReport { outcome, context }
}
