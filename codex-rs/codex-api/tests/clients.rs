use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use bytes::Bytes;
use codex_api::ApiError;
use codex_api::AuthError;
use codex_api::AuthProvider;
use codex_api::ChatCompletionsClient;
use codex_api::Compression;
use codex_api::Provider;
use codex_api::ResponseEvent;
use codex_api::ResponsesApiRequest;
use codex_api::ResponsesClient;
use codex_api::ResponsesOptions;
use codex_client::HttpTransport;
use codex_client::Request;
use codex_client::RequestBody;
use codex_client::Response;
use codex_client::StreamResponse;
use codex_client::TransportError;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::LocalShellExecAction;
use codex_protocol::models::LocalShellStatus;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use http::HeaderMap;
use http::HeaderValue;
use http::StatusCode;
use pretty_assertions::assert_eq;

fn assert_path_ends_with(requests: &[Request], suffix: &str) {
    assert_eq!(requests.len(), 1);
    let url = &requests[0].url;
    assert!(
        url.ends_with(suffix),
        "expected url to end with {suffix}, got {url}"
    );
}

fn json_body(request: &Request) -> Result<&serde_json::Value> {
    request
        .body
        .as_ref()
        .and_then(RequestBody::json)
        .ok_or_else(|| anyhow::anyhow!("request should have a JSON body"))
}

fn responses_request() -> ResponsesApiRequest {
    ResponsesApiRequest {
        model: "gpt-test".into(),
        instructions: "Say hi".into(),
        input: Vec::new(),
        tools: Vec::new(),
        tool_choice: "auto".into(),
        parallel_tool_calls: false,
        reasoning: None,
        store: false,
        stream: true,
        include: Vec::new(),
        service_tier: None,
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
    }
}

#[derive(Debug, Default, Clone)]
struct RecordingState {
    stream_requests: Arc<Mutex<Vec<Request>>>,
}

impl RecordingState {
    fn record(&self, req: Request) {
        let mut guard = self
            .stream_requests
            .lock()
            .unwrap_or_else(|err| panic!("mutex poisoned: {err}"));
        guard.push(req);
    }

    fn take_stream_requests(&self) -> Vec<Request> {
        let mut guard = self
            .stream_requests
            .lock()
            .unwrap_or_else(|err| panic!("mutex poisoned: {err}"));
        std::mem::take(&mut *guard)
    }
}

#[derive(Clone)]
struct RecordingTransport {
    state: RecordingState,
}

impl RecordingTransport {
    fn new(state: RecordingState) -> Self {
        Self { state }
    }
}

#[derive(Clone)]
struct StaticStreamTransport {
    state: RecordingState,
    chunks: Vec<&'static str>,
}

impl StaticStreamTransport {
    fn new(state: RecordingState, chunks: Vec<&'static str>) -> Self {
        Self { state, chunks }
    }
}

impl HttpTransport for StaticStreamTransport {
    async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
        Err(TransportError::Build("execute should not run".to_string()))
    }

    async fn stream(&self, req: Request) -> Result<StreamResponse, TransportError> {
        self.state.record(req);

        let chunks = self
            .chunks
            .iter()
            .map(|chunk| Ok(Bytes::from_static(chunk.as_bytes())))
            .collect::<Vec<_>>();
        Ok(StreamResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            bytes: Box::pin(futures::stream::iter(chunks)),
        })
    }
}

impl HttpTransport for RecordingTransport {
    async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
        Err(TransportError::Build("execute should not run".to_string()))
    }

    async fn stream(&self, req: Request) -> Result<StreamResponse, TransportError> {
        self.state.record(req);

        let stream = futures::stream::iter(Vec::<Result<Bytes, TransportError>>::new());
        Ok(StreamResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            bytes: Box::pin(stream),
        })
    }
}

#[derive(Clone, Default)]
struct NoAuth;

impl AuthProvider for NoAuth {
    fn add_auth_headers(&self, _headers: &mut HeaderMap) {}
}

#[derive(Clone)]
struct StaticAuth {
    token: String,
    account_id: String,
}

impl StaticAuth {
    fn new(token: &str, account_id: &str) -> Self {
        Self {
            token: token.to_string(),
            account_id: account_id.to_string(),
        }
    }
}

impl AuthProvider for StaticAuth {
    fn add_auth_headers(&self, headers: &mut HeaderMap) {
        let token = &self.token;
        if let Ok(header) = HeaderValue::from_str(&format!("Bearer {token}")) {
            headers.insert(http::header::AUTHORIZATION, header);
        }
        if let Ok(header) = HeaderValue::from_str(&self.account_id) {
            headers.insert("ChatGPT-Account-ID", header);
        }
    }
}

fn provider(name: &str) -> Provider {
    Provider {
        name: name.to_string(),
        base_url: "https://example.com/v1".to_string(),
        query_params: None,
        headers: HeaderMap::new(),
        retry: codex_api::RetryConfig {
            max_attempts: 1,
            base_delay: Duration::from_millis(1),
            retry_429: false,
            retry_5xx: false,
            retry_transport: true,
        },
        stream_idle_timeout: Duration::from_millis(10),
    }
}

async fn chat_completions_body_for_request(
    request: ResponsesApiRequest,
) -> Result<serde_json::Value> {
    let state = RecordingState::default();
    let transport = RecordingTransport::new(state.clone());
    let client = ChatCompletionsClient::new(transport, provider("openai"), Arc::new(NoAuth));

    let _stream = client
        .stream_request(
            request,
            ResponsesOptions {
                compression: Compression::None,
                ..Default::default()
            },
        )
        .await?;

    let requests = state.take_stream_requests();
    assert_eq!(requests.len(), 1);
    Ok(json_body(&requests[0])?.clone())
}

fn text_message(role: &str, text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: role.to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn function_call(call_id: &str, name: &str, arguments: &str) -> ResponseItem {
    ResponseItem::FunctionCall {
        id: None,
        name: name.to_string(),
        namespace: None,
        arguments: arguments.to_string(),
        call_id: call_id.to_string(),
    }
}

fn function_output(call_id: &str, output: &str) -> ResponseItem {
    ResponseItem::FunctionCallOutput {
        call_id: call_id.to_string(),
        output: FunctionCallOutputPayload::from_text(output.to_string()),
    }
}

fn custom_tool_call(call_id: &str, name: &str, input: &str) -> ResponseItem {
    ResponseItem::CustomToolCall {
        id: None,
        status: None,
        call_id: call_id.to_string(),
        name: name.to_string(),
        input: input.to_string(),
    }
}

fn custom_tool_output(call_id: &str, output: &str) -> ResponseItem {
    ResponseItem::CustomToolCallOutput {
        call_id: call_id.to_string(),
        name: None,
        output: FunctionCallOutputPayload::from_text(output.to_string()),
    }
}

#[derive(Clone)]
struct FlakyTransport {
    state: Arc<Mutex<i64>>,
}

impl Default for FlakyTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl FlakyTransport {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(0)),
        }
    }

    fn attempts(&self) -> i64 {
        *self
            .state
            .lock()
            .unwrap_or_else(|err| panic!("mutex poisoned: {err}"))
    }
}

#[derive(Clone)]
struct FailsOnceAuth {
    attempts: Arc<Mutex<i64>>,
    error: Arc<AuthError>,
}

impl FailsOnceAuth {
    fn transient() -> Self {
        Self {
            attempts: Arc::new(Mutex::new(0)),
            error: Arc::new(AuthError::Transient(
                "sts temporarily unavailable".to_string(),
            )),
        }
    }

    fn build() -> Self {
        Self {
            attempts: Arc::new(Mutex::new(0)),
            error: Arc::new(AuthError::Build("invalid auth configuration".to_string())),
        }
    }

    fn attempts(&self) -> i64 {
        *self
            .attempts
            .lock()
            .unwrap_or_else(|err| panic!("mutex poisoned: {err}"))
    }

    async fn apply_auth(&self, request: Request) -> Result<Request, AuthError> {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|err| panic!("mutex poisoned: {err}"));
        *attempts += 1;

        if *attempts == 1 {
            return match self.error.as_ref() {
                AuthError::Build(message) => Err(AuthError::Build(message.clone())),
                AuthError::Transient(message) => Err(AuthError::Transient(message.clone())),
            };
        }

        Ok(request)
    }
}

impl AuthProvider for FailsOnceAuth {
    fn add_auth_headers(&self, _headers: &mut HeaderMap) {}

    fn apply_auth(&self, request: Request) -> codex_api::AuthProviderFuture<'_> {
        Box::pin(FailsOnceAuth::apply_auth(self, request))
    }
}

impl HttpTransport for FlakyTransport {
    async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
        Err(TransportError::Build("execute should not run".to_string()))
    }

    async fn stream(&self, _req: Request) -> Result<StreamResponse, TransportError> {
        let mut attempts = self
            .state
            .lock()
            .unwrap_or_else(|err| panic!("mutex poisoned: {err}"));
        *attempts += 1;

        if *attempts == 1 {
            return Err(TransportError::Network("first attempt fails".to_string()));
        }

        let stream = futures::stream::iter(vec![Ok(Bytes::from(
            r#"event: message
data: {"id":"resp-1","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}]}

"#,
        ))]);

        Ok(StreamResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            bytes: Box::pin(stream),
        })
    }
}

#[tokio::test]
async fn responses_client_uses_responses_path() -> Result<()> {
    let state = RecordingState::default();
    let transport = RecordingTransport::new(state.clone());
    let client = ResponsesClient::new(transport, provider("openai"), Arc::new(NoAuth));

    let body = serde_json::json!({ "echo": true });
    let _stream = client
        .stream(
            body,
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
        )
        .await?;

    let requests = state.take_stream_requests();
    assert_path_ends_with(&requests, "/responses");
    Ok(())
}

#[tokio::test]
async fn chat_completions_client_uses_chat_path() -> Result<()> {
    let state = RecordingState::default();
    let transport = RecordingTransport::new(state.clone());
    let client = ChatCompletionsClient::new(transport, provider("openai"), Arc::new(NoAuth));

    let _stream = client
        .stream_request(
            responses_request(),
            ResponsesOptions {
                compression: Compression::None,
                ..Default::default()
            },
        )
        .await?;

    let requests = state.take_stream_requests();
    assert_path_ends_with(&requests, "/chat/completions");
    Ok(())
}

#[tokio::test]
async fn chat_completions_client_converts_responses_request() -> Result<()> {
    let mut request = responses_request();
    request.input = vec![
        text_message("developer", "be concise"),
        text_message("user", "run pwd"),
        function_call("call-1", "exec_command", "{\"cmd\":\"pwd\"}"),
        function_output("call-1", "/tmp"),
    ];
    request.tools = vec![serde_json::json!({
        "type": "function",
        "name": "exec_command",
        "description": "Runs a command",
        "parameters": {"type": "object"}
    })];

    let body = chat_completions_body_for_request(request).await?;
    assert_eq!(
        body,
        serde_json::json!({
            "model": "gpt-test",
            "messages": [
                {"role": "system", "content": "Say hi"},
                {"role": "system", "content": "be concise"},
                {"role": "user", "content": "run pwd"},
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "exec_command",
                            "arguments": "{\"cmd\":\"pwd\"}"
                        }
                    }]
                },
                {"role": "tool", "content": "/tmp", "tool_call_id": "call-1"}
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "exec_command",
                    "description": "Runs a command",
                    "parameters": {"type": "object"}
                }
            }],
            "tool_choice": "auto",
            "stream": true
        })
    );
    Ok(())
}

#[tokio::test]
async fn chat_completions_client_batches_parallel_tool_calls() -> Result<()> {
    let mut request = responses_request();
    request.input = vec![
        text_message("user", "run both"),
        function_call("call-1", "first_tool", "{\"value\":1}"),
        function_call("call-2", "second_tool", "{\"value\":2}"),
        function_output("call-1", "first output"),
        function_output("call-2", "second output"),
    ];

    let body = chat_completions_body_for_request(request).await?;
    assert_eq!(
        body["messages"],
        serde_json::json!([
            {"role": "system", "content": "Say hi"},
            {"role": "user", "content": "run both"},
            {
                "role": "assistant",
                "tool_calls": [
                    {
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "first_tool",
                            "arguments": "{\"value\":1}"
                        }
                    },
                    {
                        "id": "call-2",
                        "type": "function",
                        "function": {
                            "name": "second_tool",
                            "arguments": "{\"value\":2}"
                        }
                    }
                ]
            },
            {"role": "tool", "content": "first output", "tool_call_id": "call-1"},
            {"role": "tool", "content": "second output", "tool_call_id": "call-2"}
        ])
    );
    Ok(())
}

#[tokio::test]
async fn chat_completions_client_drops_incomplete_tool_call_batch() -> Result<()> {
    let mut request = responses_request();
    request.input = vec![
        text_message("user", "run both"),
        function_call("call-1", "first_tool", "{}"),
        function_call("call-2", "second_tool", "{}"),
        function_output("call-1", "first output"),
        text_message("user", "continue"),
    ];

    let body = chat_completions_body_for_request(request).await?;
    assert_eq!(
        body["messages"],
        serde_json::json!([
            {"role": "system", "content": "Say hi"},
            {"role": "user", "content": "run both"},
            {"role": "user", "content": "continue"}
        ])
    );
    Ok(())
}

#[tokio::test]
async fn chat_completions_client_drops_orphan_tool_output() -> Result<()> {
    let mut request = responses_request();
    request.input = vec![
        text_message("user", "run pwd"),
        function_output("call-1", "/tmp"),
    ];

    let body = chat_completions_body_for_request(request).await?;
    assert_eq!(
        body["messages"],
        serde_json::json!([
            {"role": "system", "content": "Say hi"},
            {"role": "user", "content": "run pwd"}
        ])
    );
    Ok(())
}

#[tokio::test]
async fn chat_completions_client_drops_tool_call_interrupted_by_message() -> Result<()> {
    let mut request = responses_request();
    request.input = vec![
        text_message("user", "run pwd"),
        function_call("call-1", "exec_command", "{}"),
        text_message("user", "never mind"),
        function_output("call-1", "/tmp"),
    ];

    let body = chat_completions_body_for_request(request).await?;
    assert_eq!(
        body["messages"],
        serde_json::json!([
            {"role": "system", "content": "Say hi"},
            {"role": "user", "content": "run pwd"},
            {"role": "user", "content": "never mind"}
        ])
    );
    Ok(())
}

#[tokio::test]
async fn chat_completions_client_converts_custom_tool_call_batch() -> Result<()> {
    let mut request = responses_request();
    request.input = vec![
        text_message("user", "use custom tool"),
        custom_tool_call("call-1", "custom_tool", "{\"value\":1}"),
        custom_tool_output("call-1", "custom output"),
    ];

    let body = chat_completions_body_for_request(request).await?;
    assert_eq!(
        body["messages"],
        serde_json::json!([
            {"role": "system", "content": "Say hi"},
            {"role": "user", "content": "use custom tool"},
            {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "custom_tool",
                        "arguments": "{\"value\":1}"
                    }
                }]
            },
            {"role": "tool", "content": "custom output", "tool_call_id": "call-1"}
        ])
    );
    Ok(())
}

#[tokio::test]
async fn chat_completions_client_drops_local_shell_output_without_chat_tool_call() -> Result<()> {
    let mut request = responses_request();
    request.input = vec![
        text_message("user", "run pwd"),
        ResponseItem::LocalShellCall {
            id: None,
            call_id: Some("call-1".to_string()),
            status: LocalShellStatus::Completed,
            action: LocalShellAction::Exec(LocalShellExecAction {
                command: vec!["pwd".to_string()],
                timeout_ms: None,
                working_directory: None,
                env: None,
                user: None,
            }),
        },
        function_output("call-1", "/tmp"),
    ];

    let body = chat_completions_body_for_request(request).await?;
    assert_eq!(
        body["messages"],
        serde_json::json!([
            {"role": "system", "content": "Say hi"},
            {"role": "user", "content": "run pwd"}
        ])
    );
    Ok(())
}

#[tokio::test]
async fn chat_completions_stream_wraps_text_deltas_in_message_item() -> Result<()> {
    let state = RecordingState::default();
    let transport = StaticStreamTransport::new(
        state,
        vec![
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"\\n\\n\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"he\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"llo\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        ],
    );
    let client = ChatCompletionsClient::new(transport, provider("openai"), Arc::new(NoAuth));

    let mut stream = client
        .stream_request(
            responses_request(),
            ResponsesOptions {
                compression: Compression::None,
                ..Default::default()
            },
        )
        .await?;

    let mut events = Vec::new();
    while let Some(event) = stream.rx_event.recv().await {
        events.push(event?);
    }

    assert_eq!(events.len(), 6);
    assert!(matches!(events[0], ResponseEvent::Created));
    assert!(matches!(
        &events[1],
        ResponseEvent::OutputItemAdded(ResponseItem::Message {
            id,
            role,
            content,
            ..
        }) if id.as_deref() == Some("chatcmpl-1-message")
            && role == "assistant"
            && content.is_empty()
    ));
    assert!(matches!(
        &events[2],
        ResponseEvent::OutputTextDelta(delta) if delta == "he"
    ));
    assert!(matches!(
        &events[3],
        ResponseEvent::OutputTextDelta(delta) if delta == "llo"
    ));
    assert!(matches!(
        &events[4],
        ResponseEvent::OutputItemDone(ResponseItem::Message { id, role, content, .. })
            if id.as_deref() == Some("chatcmpl-1-message")
                && role == "assistant"
                && content == &vec![ContentItem::OutputText {
                    text: "hello".to_string(),
                }]
    ));
    assert!(matches!(
        &events[5],
        ResponseEvent::Completed { response_id, .. } if response_id == "chatcmpl-1"
    ));
    Ok(())
}

#[tokio::test]
async fn streaming_client_adds_auth_headers() -> Result<()> {
    let state = RecordingState::default();
    let transport = RecordingTransport::new(state.clone());
    let auth = Arc::new(StaticAuth::new("secret-token", "acct-1"));
    let client = ResponsesClient::new(transport, provider("openai"), auth);

    let body = serde_json::json!({ "model": "gpt-test" });
    let _stream = client
        .stream(
            body,
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
        )
        .await?;

    let requests = state.take_stream_requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];

    let auth_header = req.headers.get(http::header::AUTHORIZATION);
    assert!(auth_header.is_some(), "missing auth header");
    assert_eq!(
        auth_header.unwrap().to_str().ok(),
        Some("Bearer secret-token")
    );

    let account_header = req.headers.get("ChatGPT-Account-ID");
    assert!(account_header.is_some(), "missing account header");
    assert_eq!(account_header.unwrap().to_str().ok(), Some("acct-1"));

    let accept_header = req.headers.get(http::header::ACCEPT);
    assert!(accept_header.is_some(), "missing Accept header");
    assert_eq!(
        accept_header.unwrap().to_str().ok(),
        Some("text/event-stream")
    );
    Ok(())
}

#[tokio::test]
async fn streaming_client_retries_on_transport_error() -> Result<()> {
    let transport = FlakyTransport::new();

    let mut provider = provider("openai");
    provider.retry.max_attempts = 2;

    let request = responses_request();
    let client = ResponsesClient::new(transport.clone(), provider, Arc::new(NoAuth));

    let _stream = client
        .stream_request(
            request,
            ResponsesOptions {
                compression: Compression::None,
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(transport.attempts(), 2);
    Ok(())
}

#[tokio::test]
async fn streaming_client_retries_on_transient_auth_error() -> Result<()> {
    let state = RecordingState::default();
    let transport = RecordingTransport::new(state.clone());
    let auth = FailsOnceAuth::transient();

    let mut provider = provider("openai");
    provider.retry.max_attempts = 2;

    let client = ResponsesClient::new(transport, provider, Arc::new(auth.clone()));
    let body = serde_json::json!({ "model": "gpt-test" });
    let _stream = client
        .stream(
            body,
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
        )
        .await?;

    assert_eq!(auth.attempts(), 2);
    assert_eq!(state.take_stream_requests().len(), 1);
    Ok(())
}

#[tokio::test]
async fn streaming_client_does_not_retry_auth_build_error() -> Result<()> {
    let state = RecordingState::default();
    let transport = RecordingTransport::new(state.clone());
    let auth = FailsOnceAuth::build();

    let mut provider = provider("openai");
    provider.retry.max_attempts = 2;

    let client = ResponsesClient::new(transport, provider, Arc::new(auth.clone()));
    let body = serde_json::json!({ "model": "gpt-test" });
    let result = client
        .stream(
            body,
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
        )
        .await;
    let err = match result {
        Ok(_) => panic!("auth build errors should fail without retry"),
        Err(err) => err,
    };

    assert!(matches!(
        err,
        ApiError::Transport(TransportError::Build(message))
            if message == "invalid auth configuration"
    ));
    assert_eq!(auth.attempts(), 1);
    assert_eq!(state.take_stream_requests().len(), 0);
    Ok(())
}

#[tokio::test]
async fn azure_default_store_attaches_ids_and_headers() -> Result<()> {
    let state = RecordingState::default();
    let transport = RecordingTransport::new(state.clone());
    let client = ResponsesClient::new(transport, provider("azure"), Arc::new(NoAuth));

    let request = ResponsesApiRequest {
        model: "gpt-test".into(),
        instructions: "Say hi".into(),
        input: vec![ResponseItem::Message {
            id: Some("msg_1".into()),
            role: "user".into(),
            content: vec![ContentItem::InputText { text: "hi".into() }],
            phase: None,
        }],
        tools: Vec::new(),
        tool_choice: "auto".into(),
        parallel_tool_calls: false,
        reasoning: None,
        store: true,
        stream: true,
        include: Vec::new(),
        service_tier: None,
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
    };

    let mut extra_headers = HeaderMap::new();
    extra_headers.insert("x-test-header", HeaderValue::from_static("present"));
    let _stream = client
        .stream_request(
            request,
            ResponsesOptions {
                session_id: Some("sess_123".into()),
                thread_id: Some("thread_123".into()),
                session_source: Some(SessionSource::SubAgent(SubAgentSource::Review)),
                extra_headers,
                compression: Compression::None,
                turn_state: None,
            },
        )
        .await?;

    let requests = state.take_stream_requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];

    assert_eq!(
        req.headers.get("session-id").and_then(|v| v.to_str().ok()),
        Some("sess_123")
    );
    assert_eq!(
        req.headers.get("thread-id").and_then(|v| v.to_str().ok()),
        Some("thread_123")
    );
    assert_eq!(
        req.headers
            .get("x-client-request-id")
            .and_then(|v| v.to_str().ok()),
        Some("thread_123")
    );
    assert_eq!(
        req.headers
            .get("x-openai-subagent")
            .and_then(|v| v.to_str().ok()),
        Some("review")
    );
    assert_eq!(
        req.headers
            .get("x-test-header")
            .and_then(|v| v.to_str().ok()),
        Some("present")
    );

    let input_id = req
        .body
        .as_ref()
        .and_then(RequestBody::json)
        .and_then(|body| body.get("input"))
        .and_then(|input| input.get(0))
        .and_then(|item| item.get("id"))
        .and_then(|id| id.as_str());
    assert_eq!(input_id, Some("msg_1"));

    Ok(())
}
