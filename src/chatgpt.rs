//! ChatGPT plan access through the public Responses API.

use std::collections::BTreeMap;

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    chatgpt_auth::ChatGptAuth,
    runtime::{AssistantMessage, Message, Model, ToolCall, ToolDefinition},
};

const RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const MODELS_URL: &str = "https://api.openai.com/v1/models";
const TOOL_NAMESPACE: &str = "rig";

pub struct ChatGpt {
    client: reqwest::Client,
    auth: ChatGptAuth,
    model: String,
}

impl ChatGpt {
    pub fn new(auth: ChatGptAuth, model: String) -> Result<Self, String> {
        Ok(Self {
            client: client()?,
            auth,
            model,
        })
    }
}

impl Model for ChatGpt {
    type Continuation = Vec<Value>;

    async fn complete(
        &self,
        messages: &[Message<Self::Continuation>],
        tools: &[ToolDefinition],
    ) -> Result<AssistantMessage<Self::Continuation>, String> {
        let token = self.auth.access_token().await?;
        send_response(
            &self.client,
            RESPONSES_URL,
            &token,
            request(&self.model, messages, tools),
        )
        .await
    }
}

pub async fn available_models(auth: &ChatGptAuth) -> Result<Vec<String>, String> {
    let token = auth.access_token().await?;
    list_models(&client()?, MODELS_URL, &token).await
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("cannot create ChatGPT HTTP client: {error}"))
}

fn request(model: &str, messages: &[Message<Vec<Value>>], tools: &[ToolDefinition]) -> Value {
    let mut input = Vec::new();
    for message in messages {
        match message {
            Message::User(content) => input.push(json!({"role": "user", "content": content})),
            Message::Assistant(message) => {
                for item in &message.continuation {
                    let mut item = item.clone();
                    // Responses provenance is output-only; all context fields are replayed.
                    item.as_object_mut().unwrap().remove("created_by");
                    input.push(item);
                }
            }
            Message::Tool(result) => {
                let output = if result.is_error {
                    json!({"error": result.content}).to_string()
                } else {
                    result.content.clone()
                };
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": result.call_id,
                    "output": output,
                }));
            }
        }
    }
    let mut request = json!({"model": model, "input": input, "store": false, "stream": true});
    if !tools.is_empty() {
        let tools: Vec<_> = tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                    "strict": true,
                })
            })
            .collect();
        request["tools"] = json!([{
            "type": "namespace", "name": TOOL_NAMESPACE,
            "description": "Tools available to Rig for this task.", "tools": tools,
        }]);
    }
    request
}

async fn checked(response: reqwest::Response) -> Result<reqwest::Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let mut error = format!("ChatGPT HTTP {status}");
    if let Some(request_id) = response.headers().get("x-request-id") {
        let request_id = request_id
            .to_str()
            .map_err(|error| format!("invalid ChatGPT request ID: {error}"))?;
        error.push_str(&format!(" (request {request_id})"));
    }
    if let Ok(body) = response.json::<Value>().await {
        append_code(&mut error, body["error"]["code"].as_str());
    }
    Err(error)
}

fn append_code(error: &mut String, code: Option<&str>) {
    if let Some(code) = code {
        error.push_str(&format!(": {code}"));
    }
}

async fn send_response(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    request: Value,
) -> Result<AssistantMessage<Vec<Value>>, String> {
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("ChatGPT request failed: {error}"))?;
    let mut events = checked(response).await?.bytes_stream().eventsource();
    let mut output = BTreeMap::new();
    while let Some(event) = events.next().await {
        let event = event.map_err(|_| "cannot read ChatGPT response stream")?;
        let event: Value = serde_json::from_str(&event.data)
            .map_err(|error| format!("invalid ChatGPT response event: {error}"))?;
        match event["type"].as_str() {
            Some("response.output_item.done") => {
                #[derive(Deserialize)]
                struct DoneItem {
                    output_index: usize,
                    item: Value,
                }
                let item: DoneItem = serde_json::from_value(event)
                    .map_err(|error| format!("invalid completed ChatGPT output item: {error}"))?;
                output.insert(item.output_index, item.item);
            }
            Some("response.completed") => {
                let mut response: Response = serde_json::from_value(event["response"].clone())
                    .map_err(|error| format!("invalid completed ChatGPT response: {error}"))?;
                response.output = output.into_values().collect();
                return response.into_message();
            }
            Some("response.failed") => {
                let mut error = "ChatGPT response failed".to_owned();
                append_code(&mut error, event["response"]["error"]["code"].as_str());
                return Err(error);
            }
            Some("response.incomplete") => {
                let mut error = "ChatGPT response incomplete".to_owned();
                append_code(
                    &mut error,
                    event["response"]["incomplete_details"]["reason"].as_str(),
                );
                return Err(error);
            }
            Some("error") => {
                let mut error = "ChatGPT response error".to_owned();
                append_code(&mut error, event["code"].as_str());
                return Err(error);
            }
            _ => {}
        }
    }
    Err("ChatGPT response stream ended before completion".into())
}

async fn list_models(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Catalog {
        models: Vec<CatalogModel>,
    }
    #[derive(Deserialize)]
    struct CatalogModel {
        slug: String,
        visibility: String,
    }
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|error| format!("ChatGPT model listing failed: {error}"))?;
    let catalog: Catalog = checked(response)
        .await?
        .json()
        .await
        .map_err(|error| format!("invalid ChatGPT model catalog: {error}"))?;
    Ok(catalog
        .models
        .into_iter()
        .filter(|model| model.visibility == "list")
        .map(|model| model.slug)
        .collect())
}

#[derive(Deserialize)]
struct Response {
    status: String,
    output: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputItem {
    Message {
        role: String,
        content: Vec<OutputContent>,
    },
    FunctionCall {
        namespace: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    Reasoning {},
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputContent {
    OutputText { text: String },
    Refusal { refusal: String },
}

impl Response {
    fn into_message(self) -> Result<AssistantMessage<Vec<Value>>, String> {
        if self.status != "completed" {
            return Err(format!("ChatGPT response is not complete: {}", self.status));
        }
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        for item in &self.output {
            match serde_json::from_value(item.clone())
                .map_err(|error| format!("unsupported or invalid ChatGPT output item: {error}"))?
            {
                OutputItem::Message {
                    role,
                    content: parts,
                } => {
                    if role != "assistant" {
                        return Err("ChatGPT returned a non-assistant output message".into());
                    }
                    for part in parts {
                        content.push_str(match &part {
                            OutputContent::OutputText { text } => text,
                            OutputContent::Refusal { refusal } => refusal,
                        });
                    }
                }
                OutputItem::FunctionCall {
                    namespace,
                    call_id,
                    name,
                    arguments,
                } => {
                    if namespace != TOOL_NAMESPACE {
                        return Err(format!(
                            "ChatGPT returned an unknown tool namespace: {namespace}"
                        ));
                    }
                    tool_calls.push(ToolCall {
                        id: call_id,
                        name,
                        arguments,
                    });
                }
                OutputItem::Reasoning {} => {}
            }
        }
        Ok(AssistantMessage {
            content: (!content.is_empty()).then_some(content),
            tool_calls,
            continuation: self.output,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        convert::Infallible,
        sync::{Arc, Mutex},
    };

    use axum::{
        Json, Router,
        body::Body,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::Response as HttpResponse,
        routing::{get, post},
    };

    use super::*;
    use crate::runtime::ToolResult;

    struct Fixture {
        status: StatusCode,
        chunks: Vec<Vec<u8>>,
    }

    #[derive(Default)]
    struct ServerState {
        requests: Vec<(String, Value)>,
        responses: VecDeque<Fixture>,
    }

    async fn server(
        responses: impl IntoIterator<Item = Fixture>,
    ) -> (String, Arc<Mutex<ServerState>>, tokio::task::JoinHandle<()>) {
        async fn respond(
            State(state): State<Arc<Mutex<ServerState>>>,
            headers: HeaderMap,
            request: Option<Json<Value>>,
        ) -> HttpResponse {
            let mut state = state.lock().unwrap();
            state.requests.push((
                headers["authorization"].to_str().unwrap().into(),
                request.map(|Json(request)| request).unwrap_or(Value::Null),
            ));
            let fixture = state.responses.pop_front().unwrap();
            HttpResponse::builder()
                .status(fixture.status)
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(futures_util::stream::iter(
                    fixture.chunks.into_iter().map(Ok::<_, Infallible>),
                )))
                .unwrap()
        }
        let state = Arc::new(Mutex::new(ServerState {
            responses: responses.into_iter().collect(),
            ..ServerState::default()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/responses", post(respond))
            .route("/models", get(respond))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (url, state, task)
    }

    fn event(value: Value) -> String {
        format!(
            "event: {}\r\ndata: {value}\r\n\r\n",
            value["type"].as_str().unwrap()
        )
    }

    fn stream(events: impl IntoIterator<Item = Value>) -> Fixture {
        Fixture {
            status: StatusCode::OK,
            chunks: vec![
                events
                    .into_iter()
                    .map(event)
                    .collect::<String>()
                    .into_bytes(),
            ],
        }
    }

    fn completed(output: Vec<Value>) -> Vec<Value> {
        output.into_iter().enumerate().map(|(output_index, item)| {
            json!({"type": "response.output_item.done", "output_index": output_index, "item": item})
        }).chain([json!({"type": "response.completed", "response": {"status": "completed", "output": []}})]).collect()
    }

    fn answer(text: &str) -> Value {
        json!({
            "type": "message", "id": "msg-1", "role": "assistant", "status": "completed",
            "phase": "final_answer",
            "content": [{"type": "output_text", "text": text, "annotations": []}],
        })
    }

    fn tool() -> ToolDefinition {
        ToolDefinition {
            name: "read_file".into(),
            description: "Read a selected file.".into(),
            parameters: json!({
                "type": "object", "properties": {"path": {"type": "string"}},
                "required": ["path"], "additionalProperties": false,
            }),
        }
    }

    #[tokio::test]
    async fn collects_completed_response_across_byte_fragments() {
        let mut fixture = stream(
            [json!({"type": "response.output_text.delta", "delta": "partial"})]
                .into_iter()
                .chain(completed(vec![answer("A UTF-8 answer: 中文")])),
        );
        fixture.chunks = fixture.chunks[0].iter().map(|byte| vec![*byte]).collect();
        let (url, state, task) = server([fixture]).await;
        let response = send_response(
            &client().unwrap(),
            &format!("{url}/responses"),
            "test-token",
            request("test-model", &[Message::User("hello".into())], &[]),
        )
        .await
        .unwrap();
        assert_eq!(response.content.as_deref(), Some("A UTF-8 answer: 中文"));
        assert!(response.tool_calls.is_empty());
        assert_eq!(response.continuation, vec![answer("A UTF-8 answer: 中文")]);
        let state = state.lock().unwrap();
        assert_eq!(state.requests[0].0, "Bearer test-token");
        assert_eq!(
            state.requests[0].1,
            json!({
                "model": "test-model", "input": [{"role": "user", "content": "hello"}],
                "store": false, "stream": true,
            })
        );
        task.abort();
    }

    #[tokio::test]
    async fn replays_reasoning_phase_and_call_identity_after_tool_result() {
        let reasoning = json!({
            "type": "reasoning", "id": "rs-1", "summary": [],
            "encrypted_content": "opaque-reasoning", "status": "completed",
        });
        let mut commentary = answer("Checking the file.");
        commentary["phase"] = json!("commentary");
        commentary["created_by"] = json!("output-provenance");
        let call = json!({
            "type": "function_call", "id": "fc-item-1", "call_id": "call-1",
            "namespace": "rig", "name": "read_file", "arguments": "{\"path\":\"selected.txt\"}",
            "status": "completed",
        });
        let first_output = vec![reasoning.clone(), commentary.clone(), call.clone()];
        let mut first_events = completed(first_output.clone());
        first_events.swap(0, 2);
        let (url, state, task) = server([
            stream(first_events),
            stream(completed(vec![answer("done")])),
        ])
        .await;
        let client = client().unwrap();
        let tools = [tool()];
        let mut context = vec![Message::User("Read the file".into())];
        let response = send_response(
            &client,
            &format!("{url}/responses"),
            "test-token",
            request("test-model", &context, &tools),
        )
        .await
        .unwrap();
        assert_eq!(response.continuation, first_output);
        assert_eq!(response.tool_calls[0].id, "call-1");
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(
            response.tool_calls[0].arguments,
            "{\"path\":\"selected.txt\"}"
        );
        context.push(Message::Assistant(response));
        context.push(Message::Tool(ToolResult {
            call_id: "call-1".into(),
            content: "file contents".into(),
            is_error: false,
        }));
        let response = send_response(
            &client,
            &format!("{url}/responses"),
            "test-token",
            request("test-model", &context, &tools),
        )
        .await
        .unwrap();
        assert_eq!(response.content.as_deref(), Some("done"));
        let state = state.lock().unwrap();
        let first = &state.requests[0].1;
        assert_eq!(first["tools"][0]["type"], "namespace");
        assert_eq!(first["tools"][0]["name"], "rig");
        assert_eq!(
            first["tools"][0]["tools"][0],
            json!({
                "type": "function", "name": "read_file", "description": tools[0].description,
                "parameters": tools[0].parameters, "strict": true,
            })
        );
        commentary.as_object_mut().unwrap().remove("created_by");
        assert_eq!(
            state.requests[1].1["input"],
            json!([
                {"role": "user", "content": "Read the file"},
                reasoning, commentary, call,
                {"type": "function_call_output", "call_id": "call-1", "output": "file contents"},
            ])
        );
        task.abort();
    }

    #[test]
    fn encodes_tool_failure_as_call_output() {
        let request = request(
            "test-model",
            &[Message::Tool(ToolResult {
                call_id: "call-1".into(),
                content: "cannot read selected file".into(),
                is_error: true,
            })],
            &[],
        );
        assert_eq!(
            request["input"][0],
            json!({
                "type": "function_call_output", "call_id": "call-1",
                "output": "{\"error\":\"cannot read selected file\"}",
            })
        );
    }

    #[tokio::test]
    async fn failed_incomplete_and_interrupted_streams_are_not_answers() {
        let fixtures = [
            stream([json!({"type": "response.failed", "response": {"error": {
                "code": "model_unavailable", "message": "private prompt or token",
            }}})]),
            stream([
                json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": "budget"}}}),
            ]),
            stream([
                json!({"type": "error", "code": "server_error", "message": "private prompt or token"}),
            ]),
            stream([
                json!({"type": "response.output_text.delta", "delta": "partial answer"}),
                json!({"type": "response.output_item.done", "output_index": 0, "item": {
                    "type": "function_call", "namespace": "rig", "call_id": "call-1",
                    "name": "read_file", "arguments": "{}", "status": "completed",
                }}),
            ]),
            stream([
                json!({"type": "response.completed", "response": {"status": "incomplete", "output": [answer("partial")]}}),
            ]),
        ];
        let (url, state, task) = server(fixtures).await;
        for _ in 0..5 {
            let result = send_response(
                &client().unwrap(),
                &format!("{url}/responses"),
                "test-token",
                request("test-model", &[Message::User("hello".into())], &[]),
            )
            .await;
            assert!(!result.unwrap_err().contains("private prompt or token"));
        }
        assert_eq!(state.lock().unwrap().requests.len(), 5);
        task.abort();
    }

    #[tokio::test]
    async fn reports_http_and_malformed_event_errors_without_retry() {
        let (url, state, task) = server([
            Fixture {
                status: StatusCode::TOO_MANY_REQUESTS,
                chunks: vec![
                    json!({"error": {
                        "code": "usage_limit_reached", "message": "private prompt or token",
                    }})
                    .to_string()
                    .into_bytes(),
                ],
            },
            Fixture {
                status: StatusCode::OK,
                chunks: vec![b"data: {broken}\n\n".to_vec()],
            },
        ])
        .await;
        let response = send_response(
            &client().unwrap(),
            &format!("{url}/responses"),
            "test-token",
            json!({}),
        )
        .await
        .unwrap_err();
        assert!(response.contains("429"));
        assert!(response.contains("usage_limit_reached"));
        assert!(!response.contains("private prompt or token"));
        let response = send_response(
            &client().unwrap(),
            &format!("{url}/responses"),
            "test-token",
            json!({}),
        )
        .await
        .unwrap_err();
        assert!(response.contains("invalid ChatGPT response event"));
        assert_eq!(state.lock().unwrap().requests.len(), 2);
        task.abort();
    }

    #[test]
    fn rejects_unknown_namespace_output_type_and_non_assistant_message() {
        for output in [
            json!({"type": "function_call", "namespace": "other", "call_id": "call-1", "name": "read_file", "arguments": "{}"}),
            json!({"type": "web_search_call", "id": "search-1"}),
            json!({"type": "message", "role": "user", "content": []}),
        ] {
            assert!(
                Response {
                    status: "completed".into(),
                    output: vec![output]
                }
                .into_message()
                .is_err()
            );
        }
    }

    #[test]
    fn returns_model_refusal_as_assistant_text() {
        let response = Response {
            status: "completed".into(),
            output: vec![json!({"type": "message", "role": "assistant", "content": [{"type": "refusal", "refusal": "Cannot do that."}]})],
        }.into_message().unwrap();
        assert_eq!(response.content.as_deref(), Some("Cannot do that."));
    }

    #[tokio::test]
    async fn lists_only_visible_models_from_oauth_catalog() {
        let (url, state, task) = server([Fixture {
            status: StatusCode::OK,
            chunks: vec![
                json!({"models": [
                    {"slug": "visible-model", "visibility": "list"},
                    {"slug": "hidden-model", "visibility": "hide"},
                ]})
                .to_string()
                .into_bytes(),
            ],
        }])
        .await;
        let models = list_models(&client().unwrap(), &format!("{url}/models"), "test-token")
            .await
            .unwrap();
        assert_eq!(models, vec!["visible-model"]);
        assert_eq!(state.lock().unwrap().requests[0].0, "Bearer test-token");
        task.abort();
    }
}
