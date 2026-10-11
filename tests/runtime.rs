use std::{
    collections::VecDeque,
    future::{Future, pending, poll_fn},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use rig::runtime::{
    AssistantMessage, Message, Model, RunEvent, RunOutcome, ToolCall, ToolDefinition, ToolResult,
    Tools, run,
};
use serde_json::json;

struct ScriptedModel {
    responses: Mutex<VecDeque<Result<AssistantMessage, String>>>,
    contexts: Mutex<Vec<Vec<Message>>>,
}

impl ScriptedModel {
    fn new(responses: Vec<Result<AssistantMessage, String>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            contexts: Mutex::new(Vec::new()),
        }
    }
}

impl Model for ScriptedModel {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<AssistantMessage, String> {
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "read");
        self.contexts.lock().unwrap().push(messages.to_vec());
        self.responses.lock().unwrap().pop_front().unwrap()
    }
}

struct ScriptedTools {
    results: Mutex<VecDeque<Result<String, String>>>,
    calls: Mutex<Vec<ToolCall>>,
}

impl ScriptedTools {
    fn new(results: Vec<Result<String, String>>) -> Self {
        Self {
            results: Mutex::new(results.into()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl Tools for ScriptedTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: "read".into(),
            description: "Read a file".into(),
            parameters: json!({"type": "object"}),
        }]
    }

    async fn execute(&self, call: &ToolCall) -> Result<String, String> {
        self.calls.lock().unwrap().push(call.clone());
        self.results.lock().unwrap().pop_front().unwrap()
    }
}

fn call(id: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "read".into(),
        arguments: arguments.into(),
    }
}

fn response(content: Option<&str>, tool_calls: Vec<ToolCall>) -> AssistantMessage {
    AssistantMessage {
        content: content.map(str::to_owned),
        tool_calls,
    }
}

#[tokio::test]
async fn executes_tools_in_order_and_returns_correlated_results_to_the_model() {
    let calls = vec![
        call("first", r#"{"path":"a"}"#),
        call("second", r#"{"path":"b"}"#),
    ];
    let request = response(Some("Reading both files."), calls.clone());
    let answer = response(Some("The files agree."), Vec::new());
    let model = ScriptedModel::new(vec![Ok(request.clone()), Ok(answer.clone())]);
    let tools = ScriptedTools::new(vec![Ok("A".into()), Ok("B".into())]);
    let mut events = Vec::new();

    let report = run(&model, &tools, "Compare files".into(), pending(), |event| {
        events.push(match event {
            RunEvent::ModelStarted => "model started".into(),
            RunEvent::ModelFinished(_) => "model finished".into(),
            RunEvent::ToolStarted(call) => format!("tool started: {}", call.id),
            RunEvent::ToolFinished(result) => format!("tool finished: {}", result.call_id),
            RunEvent::Finished(_) => "finished".into(),
        });
    })
    .await;

    let first_context = vec![Message::User("Compare files".into())];
    let mut next_context = first_context.clone();
    next_context.extend([
        Message::Assistant(request),
        Message::Tool(ToolResult {
            call_id: "first".into(),
            content: "A".into(),
            is_error: false,
        }),
        Message::Tool(ToolResult {
            call_id: "second".into(),
            content: "B".into(),
            is_error: false,
        }),
    ]);
    assert_eq!(
        report.outcome,
        RunOutcome::Completed("The files agree.".into())
    );
    assert_eq!(*tools.calls.lock().unwrap(), calls);
    assert_eq!(
        *model.contexts.lock().unwrap(),
        vec![first_context, next_context.clone()]
    );
    next_context.push(Message::Assistant(answer));
    assert_eq!(report.context, next_context);
    assert_eq!(
        events,
        [
            "model started",
            "model finished",
            "tool started: first",
            "tool finished: first",
            "tool started: second",
            "tool finished: second",
            "model started",
            "model finished",
            "finished",
        ]
    );
}

#[tokio::test]
async fn tool_errors_return_to_the_model_without_skipping_other_calls() {
    let mut unknown = call("unknown", "{}");
    unknown.name = "missing".into();
    let invalid = call("invalid", "not json");
    let valid = call("valid", "{}");
    let model = ScriptedModel::new(vec![
        Ok(response(
            None,
            vec![unknown, invalid.clone(), valid.clone()],
        )),
        Ok(response(Some("Handled the failures."), Vec::new())),
    ]);
    let tools = ScriptedTools::new(vec![Err("invalid arguments".into()), Ok("contents".into())]);

    let report = run(&model, &tools, "Read files".into(), pending(), |_| {}).await;

    assert_eq!(
        report.outcome,
        RunOutcome::Completed("Handled the failures.".into())
    );
    assert_eq!(*tools.calls.lock().unwrap(), vec![invalid, valid]);
    let contexts = model.contexts.lock().unwrap();
    assert_eq!(
        &contexts[1][2..],
        &[
            Message::Tool(ToolResult {
                call_id: "unknown".into(),
                content: "unknown tool: missing".into(),
                is_error: true
            }),
            Message::Tool(ToolResult {
                call_id: "invalid".into(),
                content: "invalid arguments".into(),
                is_error: true
            }),
            Message::Tool(ToolResult {
                call_id: "valid".into(),
                content: "contents".into(),
                is_error: false
            }),
        ]
    );
}

#[tokio::test]
async fn model_failure_ends_the_run_without_retry() {
    let model = ScriptedModel::new(vec![Err("connection closed".into())]);
    let tools = ScriptedTools::new(Vec::new());
    let mut finished = 0;

    let report = run(&model, &tools, "Hello".into(), pending(), |event| {
        if let RunEvent::Finished(outcome) = event {
            assert_eq!(outcome, &RunOutcome::Failed("connection closed".into()));
            finished += 1;
        }
    })
    .await;

    assert_eq!(
        report.outcome,
        RunOutcome::Failed("connection closed".into())
    );
    assert_eq!(report.context, vec![Message::User("Hello".into())]);
    assert_eq!(model.contexts.lock().unwrap().len(), 1);
    assert!(tools.calls.lock().unwrap().is_empty());
    assert_eq!(finished, 1);
}

#[tokio::test]
async fn response_without_answer_or_tools_fails() {
    for content in [None, Some("")] {
        let model = ScriptedModel::new(vec![Ok(response(content, Vec::new()))]);
        let tools = ScriptedTools::new(Vec::new());
        let report = run(&model, &tools, "Hello".into(), pending(), |_| {}).await;
        assert!(matches!(report.outcome, RunOutcome::Failed(_)));
        assert!(tools.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn invalid_call_ids_fail_before_any_tool_in_the_batch_executes() {
    for ids in [["first", ""], ["same", "same"]] {
        let model = ScriptedModel::new(vec![Ok(response(
            None,
            ids.map(|id| call(id, "{}")).into(),
        ))]);
        let tools = ScriptedTools::new(Vec::new());
        let report = run(&model, &tools, "Read files".into(), pending(), |_| {}).await;
        assert!(matches!(report.outcome, RunOutcome::Failed(_)));
        assert!(tools.calls.lock().unwrap().is_empty());
        assert_eq!(model.contexts.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn call_ids_can_be_reused_in_a_later_response() {
    let request = response(None, vec![call("same", "{}")]);
    let model = ScriptedModel::new(vec![
        Ok(request.clone()),
        Ok(request),
        Ok(response(Some("Done"), Vec::new())),
    ]);
    let tools = ScriptedTools::new(vec![Ok("first result".into()), Ok("second result".into())]);
    let report = run(&model, &tools, "Read twice".into(), pending(), |_| {}).await;
    assert_eq!(report.outcome, RunOutcome::Completed("Done".into()));
    assert_eq!(tools.calls.lock().unwrap().len(), 2);
    assert_eq!(model.contexts.lock().unwrap().len(), 3);
}

struct Cancellation(Arc<AtomicBool>);

impl Future for Cancellation {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if self.0.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

struct DropMarker(Arc<AtomicBool>);

impl Drop for DropMarker {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct PendingOperation {
    cancel: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

impl PendingOperation {
    async fn wait<T>(&self) -> Result<T, String> {
        let _guard = DropMarker(self.dropped.clone());
        poll_fn(|cx| {
            self.cancel.store(true, Ordering::SeqCst);
            cx.waker().wake_by_ref();
            Poll::Pending
        })
        .await
    }
}

impl Model for PendingOperation {
    async fn complete(
        &self,
        _: &[Message],
        _: &[ToolDefinition],
    ) -> Result<AssistantMessage, String> {
        self.wait().await
    }
}

impl Tools for PendingOperation {
    fn definitions(&self) -> Vec<ToolDefinition> {
        ScriptedTools::new(Vec::new()).definitions()
    }

    async fn execute(&self, _: &ToolCall) -> Result<String, String> {
        self.wait().await
    }
}

#[tokio::test]
async fn cancellation_before_run_starts_no_model_or_tools() {
    let model = ScriptedModel::new(Vec::new());
    let tools = ScriptedTools::new(Vec::new());
    let mut events = Vec::new();
    let report = run(&model, &tools, "Hello".into(), async {}, |event| {
        events.push(matches!(event, RunEvent::Finished(&RunOutcome::Cancelled)));
    })
    .await;
    assert_eq!(report.outcome, RunOutcome::Cancelled);
    assert_eq!(events, [true]);
    assert!(model.contexts.lock().unwrap().is_empty());
    assert!(tools.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancellation_drops_a_pending_model_request() {
    let cancel = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let model = PendingOperation {
        cancel: cancel.clone(),
        dropped: dropped.clone(),
    };
    let tools = ScriptedTools::new(Vec::new());
    let report = run(&model, &tools, "Hello".into(), Cancellation(cancel), |_| {}).await;
    assert_eq!(report.outcome, RunOutcome::Cancelled);
    assert_eq!(report.context, vec![Message::User("Hello".into())]);
    assert!(dropped.load(Ordering::SeqCst));
    assert!(tools.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancellation_drops_a_pending_tool_and_stops_the_batch() {
    let cancel = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let model = ScriptedModel::new(vec![Ok(response(
        None,
        vec![call("first", "{}"), call("second", "{}")],
    ))]);
    let tools = PendingOperation {
        cancel: cancel.clone(),
        dropped: dropped.clone(),
    };
    let mut started = Vec::new();
    let report = run(
        &model,
        &tools,
        "Read files".into(),
        Cancellation(cancel),
        |event| {
            if let RunEvent::ToolStarted(call) = event {
                started.push(call.id.clone());
            }
        },
    )
    .await;
    assert_eq!(report.outcome, RunOutcome::Cancelled);
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(started, ["first"]);
    assert_eq!(report.context.len(), 2);
    assert_eq!(model.contexts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cancellation_after_one_tool_keeps_its_result_and_starts_no_more_work() {
    let cancel = Arc::new(AtomicBool::new(false));
    let model = ScriptedModel::new(vec![Ok(response(
        None,
        vec![call("first", "{}"), call("second", "{}")],
    ))]);
    let tools = ScriptedTools::new(vec![Ok("read successfully".into())]);
    let report = run(
        &model,
        &tools,
        "Read files".into(),
        Cancellation(cancel.clone()),
        |event| {
            if let RunEvent::ToolFinished(_) = event {
                cancel.store(true, Ordering::SeqCst);
            }
        },
    )
    .await;
    assert_eq!(report.outcome, RunOutcome::Cancelled);
    assert_eq!(model.contexts.lock().unwrap().len(), 1);
    assert_eq!(tools.calls.lock().unwrap().len(), 1);
    assert_eq!(
        report.context.last(),
        Some(&Message::Tool(ToolResult {
            call_id: "first".into(),
            content: "read successfully".into(),
            is_error: false,
        }))
    );
}
