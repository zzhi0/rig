//! DeepSeek 的非流式 Chat Completion 适配，不启用思考模式或自动重试。

use serde::Deserialize;
use serde_json::{Value, json};

use crate::runtime::{AssistantMessage, Message, Model, ToolCall, ToolDefinition};

pub const DEFAULT_MODEL: &str = "deepseek-flash";
const CHAT_COMPLETIONS_URL: &str = "https://api.deepseek.com/chat/completions";

pub struct DeepSeek {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl DeepSeek {
    pub fn new(api_key: String, model: String) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| format!("cannot create DeepSeek HTTP client: {error}"))?;
        Ok(Self {
            client,
            api_key,
            model,
        })
    }

    async fn complete_at(
        &self,
        url: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<AssistantMessage, String> {
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&self.request(messages, tools))
            .send()
            .await
            .map_err(|error| format!("DeepSeek request failed: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("cannot read DeepSeek response: {error}"))?;
        if !status.is_success() {
            return Err(format!("DeepSeek HTTP {status}: {body}"));
        }
        let response: ChatResponse = serde_json::from_str(&body)
            .map_err(|error| format!("invalid DeepSeek response: {error}"))?;
        response.into_message()
    }

    fn request(&self, messages: &[Message], tools: &[ToolDefinition]) -> Value {
        let messages: Vec<_> = messages.iter().map(encode_message).collect();
        let mut request = json!({
            "model": self.model,
            "messages": messages,
            "stream": false,
            "thinking": {"type": "disabled"},
        });
        if !tools.is_empty() {
            request["tools"] = tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        },
                    })
                })
                .collect();
        }
        request
    }
}

impl Model for DeepSeek {
    type Continuation = ();

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<AssistantMessage, String> {
        self.complete_at(CHAT_COMPLETIONS_URL, messages, tools)
            .await
    }
}

fn encode_message(message: &Message) -> Value {
    match message {
        Message::User(content) => json!({"role": "user", "content": content}),
        Message::Assistant(message) => {
            let mut encoded = json!({"role": "assistant", "content": message.content});
            if !message.tool_calls.is_empty() {
                encoded["tool_calls"] = message
                    .tool_calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "type": "function",
                            "function": {"name": call.name, "arguments": call.arguments},
                        })
                    })
                    .collect();
            }
            encoded
        }
        Message::Tool(result) => {
            let content = if result.is_error {
                json!({"error": result.content}).to_string()
            } else {
                result.content.clone()
            };
            json!({"role": "tool", "tool_call_id": result.call_id, "content": content})
        }
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    finish_reason: String,
    message: ApiMessage,
}

#[derive(Deserialize)]
struct ApiMessage {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<ApiToolCall>>,
}

#[derive(Deserialize)]
struct ApiToolCall {
    id: String,
    r#type: String,
    function: ApiFunction,
}

#[derive(Deserialize)]
struct ApiFunction {
    name: String,
    arguments: String,
}

impl ChatResponse {
    fn into_message(self) -> Result<AssistantMessage, String> {
        let [choice] = <[Choice; 1]>::try_from(self.choices)
            .map_err(|_| "DeepSeek must return exactly one choice")?;
        if choice.message.role != "assistant" {
            return Err("DeepSeek returned a non-assistant message".into());
        }
        let calls = choice.message.tool_calls.unwrap_or_default();
        match choice.finish_reason.as_str() {
            "stop" if calls.is_empty() => {}
            "tool_calls" if !calls.is_empty() => {}
            reason => {
                return Err(format!(
                    "DeepSeek incomplete or inconsistent response: {reason}"
                ));
            }
        }
        let tool_calls = calls
            .into_iter()
            .map(|call| {
                if call.r#type != "function" {
                    return Err("DeepSeek returned an unsupported tool call type".into());
                }
                Ok(ToolCall {
                    id: call.id,
                    name: call.function.name,
                    arguments: call.function.arguments,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(AssistantMessage {
            continuation: (),
            content: choice.message.content,
            tool_calls,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
    };

    use super::*;
    use crate::runtime::ToolResult;

    #[derive(Default)]
    struct ServerState {
        requests: Vec<Value>,
        responses: VecDeque<(StatusCode, Value)>,
    }

    async fn server(
        responses: impl IntoIterator<Item = (StatusCode, Value)>,
    ) -> (String, Arc<Mutex<ServerState>>, tokio::task::JoinHandle<()>) {
        let state = Arc::new(Mutex::new(ServerState {
            responses: responses.into_iter().collect(),
            ..ServerState::default()
        }));
        let router = Router::new()
            .route(
                "/chat/completions",
                post(
                    |State(state): State<Arc<Mutex<ServerState>>>,
                     headers: HeaderMap,
                     Json(request): Json<Value>| async move {
                        assert_eq!(headers["authorization"], "Bearer test-key");
                        let mut state = state.lock().unwrap();
                        state.requests.push(request);
                        let (status, body) = state.responses.pop_front().unwrap();
                        (status, Json(body))
                    },
                ),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/chat/completions", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (url, state, handle)
    }

    fn model() -> DeepSeek {
        DeepSeek::new("test-key".into(), DEFAULT_MODEL.into()).unwrap()
    }

    fn response(reason: &str, content: Value, calls: Value) -> Value {
        json!({"choices": [{
            "finish_reason": reason,
            "message": {"role": "assistant", "content": content, "tool_calls": calls},
        }]})
    }

    #[tokio::test]
    async fn tool_calls_and_results_preserve_association_across_requests() {
        let (url, state, handle) = server([
            (StatusCode::OK, response("tool_calls", Value::Null, json!([
                {"id": "call-1", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"a.txt\"}"}},
                {"id": "call-2", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"b.txt\"}"}},
            ]))),
            (StatusCode::OK, response("stop", json!("done"), Value::Null)),
        ]).await;
        let model = model();
        let tools = vec![ToolDefinition {
            name: "read_file".into(),
            description: "Read text".into(),
            parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}),
        }];
        let mut messages = vec![Message::User("Read the files".into())];
        let assistant = model.complete_at(&url, &messages, &tools).await.unwrap();
        assert_eq!(assistant.content, None);
        assert_eq!(assistant.tool_calls[0].id, "call-1");
        assert_eq!(assistant.tool_calls[1].arguments, "{\"path\":\"b.txt\"}");
        messages.push(Message::Assistant(assistant));
        messages.push(Message::Tool(ToolResult {
            call_id: "call-1".into(),
            content: "hello".into(),
            is_error: false,
        }));
        messages.push(Message::Tool(ToolResult {
            call_id: "call-2".into(),
            content: "file missing".into(),
            is_error: true,
        }));
        let answer = model.complete_at(&url, &messages, &tools).await.unwrap();
        assert_eq!(answer.content.as_deref(), Some("done"));
        assert!(answer.tool_calls.is_empty());

        let state = state.lock().unwrap();
        let initial = &state.requests[0];
        assert_eq!(initial["model"], DEFAULT_MODEL);
        assert_eq!(initial["thinking"]["type"], "disabled");
        assert_eq!(initial["stream"], false);
        assert_eq!(
            initial["tools"][0]["function"]["parameters"],
            tools[0].parameters
        );
        let next = &state.requests[1]["messages"];
        assert_eq!(next[1]["tool_calls"][0]["id"], "call-1");
        assert!(next[1]["content"].is_null());
        assert_eq!(
            next[2],
            json!({"role": "tool", "tool_call_id": "call-1", "content": "hello"})
        );
        assert_eq!(next[3]["tool_call_id"], "call-2");
        assert_eq!(
            serde_json::from_str::<Value>(next[3]["content"].as_str().unwrap()).unwrap(),
            json!({"error": "file missing"})
        );
        handle.abort();
    }

    #[tokio::test]
    async fn truncation_is_failure_and_authentication_error_is_not_retried() {
        let (url, state, handle) = server([
            (
                StatusCode::OK,
                response("length", json!("partial answer"), Value::Null),
            ),
            (
                StatusCode::UNAUTHORIZED,
                json!({"error": {"message": "invalid API key"}}),
            ),
        ])
        .await;
        let model = model();
        let messages = [Message::User("hello".into())];
        let error = model.complete_at(&url, &messages, &[]).await.unwrap_err();
        assert!(error.contains("length"));
        let error = model.complete_at(&url, &messages, &[]).await.unwrap_err();
        assert!(error.contains("401"));
        assert!(error.contains("invalid API key"));
        assert!(!error.contains("test-key"));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 2);
        assert!(state.requests[0].get("tools").is_none());
        handle.abort();
    }

    #[test]
    fn inconsistent_response_cannot_dispatch_tools() {
        for value in [
            response("tool_calls", Value::Null, Value::Null),
            response(
                "stop",
                json!("done"),
                json!([{"id": "call-1", "type": "function", "function": {"name": "read_file", "arguments": "{}"}}]),
            ),
            json!({"choices": []}),
        ] {
            assert!(
                serde_json::from_value::<ChatResponse>(value)
                    .unwrap()
                    .into_message()
                    .is_err()
            );
        }
    }
}
