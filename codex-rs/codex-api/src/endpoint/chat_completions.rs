use crate::auth::SharedAuthProvider;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::common::ResponsesApiRequest;
use crate::endpoint::ResponsesOptions;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::Compression;
use crate::requests::headers::build_session_headers;
use crate::requests::headers::insert_header;
use crate::requests::headers::subagent_header;
use codex_client::ByteStream;
use codex_client::HttpTransport;
use codex_client::RequestCompression;
use codex_client::RequestTelemetry;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;
use tracing::instrument;

pub struct ChatCompletionsClient<T: HttpTransport> {
    session: EndpointSession<T>,
}

#[derive(Debug, Serialize)]
struct ChatCompletionsRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ChatTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Debug, Serialize)]
struct ChatToolCall {
    id: String,
    r#type: String,
    function: ChatToolCallFunction,
}

#[derive(Debug, Serialize)]
struct ChatToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct ChatTool {
    r#type: String,
    function: ChatToolFunction,
}

#[derive(Debug, Serialize)]
struct ChatToolFunction {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: Value,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    id: Option<String>,
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    delta: ChatDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatDelta {
    content: Option<String>,
    tool_calls: Option<Vec<ChatDeltaToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ChatDeltaToolCall {
    index: usize,
    id: Option<String>,
    function: Option<ChatDeltaToolCallFunction>,
}

#[derive(Debug, Deserialize)]
struct ChatDeltaToolCallFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
}

#[derive(Debug, Default)]
struct PendingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl<T: HttpTransport> ChatCompletionsClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
        }
    }

    pub fn with_telemetry(self, request: Option<Arc<dyn RequestTelemetry>>) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
        }
    }

    #[instrument(
        name = "chat_completions.stream_request",
        level = "info",
        skip_all,
        fields(
            transport = "chat_completions_http",
            http.method = "POST",
            api.path = "chat/completions"
        )
    )]
    pub async fn stream_request(
        &self,
        request: ResponsesApiRequest,
        options: ResponsesOptions,
    ) -> Result<ResponseStream, ApiError> {
        let ResponsesOptions {
            session_id,
            thread_id,
            session_source,
            extra_headers,
            compression,
            turn_state: _,
        } = options;

        let request = convert_request(request)?;
        let body = serde_json::to_value(&request).map_err(|e| {
            ApiError::Stream(format!("failed to encode chat completions request: {e}"))
        })?;

        let mut headers = extra_headers;
        if let Some(ref thread_id) = thread_id {
            insert_header(&mut headers, "x-client-request-id", thread_id);
        }
        headers.extend(build_session_headers(session_id, thread_id));
        if let Some(subagent) = subagent_header(&session_source) {
            insert_header(&mut headers, "x-openai-subagent", &subagent);
        }

        self.stream(body, headers, compression).await
    }

    #[instrument(
        name = "chat_completions.stream",
        level = "info",
        skip_all,
        fields(
            transport = "chat_completions_http",
            http.method = "POST",
            api.path = "chat/completions"
        )
    )]
    async fn stream(
        &self,
        body: Value,
        extra_headers: HeaderMap,
        compression: Compression,
    ) -> Result<ResponseStream, ApiError> {
        let request_compression = match compression {
            Compression::None => RequestCompression::None,
            Compression::Zstd => RequestCompression::Zstd,
        };

        let stream_response = self
            .session
            .stream_with(
                Method::POST,
                "chat/completions",
                extra_headers,
                Some(body),
                |req| {
                    req.headers.insert(
                        http::header::ACCEPT,
                        HeaderValue::from_static("text/event-stream"),
                    );
                    req.compression = request_compression;
                },
            )
            .await?;

        Ok(spawn_chat_response_stream(stream_response))
    }
}

fn convert_request(request: ResponsesApiRequest) -> Result<ChatCompletionsRequest, ApiError> {
    let mut messages = Vec::new();
    if !request.instructions.is_empty() {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some(request.instructions),
            tool_call_id: None,
            tool_calls: None,
        });
    }

    for item in request.input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                if let Some(text) = content_items_to_text(&content) {
                    messages.push(ChatMessage {
                        role: chat_message_role(role),
                        content: Some(text),
                        tool_call_id: None,
                        tool_calls: None,
                    });
                }
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                ..
            } => {
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_call_id: None,
                    tool_calls: Some(vec![ChatToolCall {
                        id: call_id,
                        r#type: "function".to_string(),
                        function: ChatToolCallFunction { name, arguments },
                    }]),
                });
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(output_to_text(output)),
                    tool_call_id: Some(call_id),
                    tool_calls: None,
                });
            }
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                messages.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_call_id: None,
                    tool_calls: Some(vec![ChatToolCall {
                        id: call_id,
                        r#type: "function".to_string(),
                        function: ChatToolCallFunction {
                            name,
                            arguments: input,
                        },
                    }]),
                });
            }
            ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(output_to_text(output)),
                    tool_call_id: Some(call_id),
                    tool_calls: None,
                });
            }
            ResponseItem::Reasoning { .. }
            | ResponseItem::LocalShellCall { .. }
            | ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::CompactionTrigger
            | ResponseItem::ContextCompaction { .. }
            | ResponseItem::Other => {}
        }
    }

    let tools = request
        .tools
        .into_iter()
        .filter_map(chat_tool_from_responses_tool)
        .collect::<Vec<_>>();
    let tool_choice = if tools.is_empty() {
        None
    } else {
        Some(request.tool_choice)
    };

    Ok(ChatCompletionsRequest {
        model: request.model,
        messages,
        tools,
        tool_choice,
        stream: true,
    })
}

fn content_items_to_text(content: &[ContentItem]) -> Option<String> {
    let text = content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                Some(text.as_str())
            }
            ContentItem::InputImage { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() { None } else { Some(text) }
}

fn chat_message_role(role: String) -> String {
    match role.as_str() {
        "developer" => "system".to_string(),
        _ => role,
    }
}

fn output_to_text(output: FunctionCallOutputPayload) -> String {
    output.body.to_text().unwrap_or_default()
}

fn chat_tool_from_responses_tool(tool: Value) -> Option<ChatTool> {
    let tool_type = tool.get("type")?.as_str()?;
    if tool_type != "function" {
        return None;
    }

    let name = tool.get("name")?.as_str()?.to_string();
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);
    let parameters = tool.get("parameters").cloned().unwrap_or_else(|| json!({}));
    Some(ChatTool {
        r#type: "function".to_string(),
        function: ChatToolFunction {
            name,
            description,
            parameters,
        },
    })
}

fn spawn_chat_response_stream(stream_response: StreamResponse) -> ResponseStream {
    let upstream_request_id = stream_response
        .headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        process_chat_sse(stream_response.bytes, tx_event).await;
    });

    ResponseStream {
        rx_event,
        upstream_request_id,
    }
}

async fn process_chat_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
) {
    let _ = tx_event.send(Ok(ResponseEvent::Created)).await;
    let mut stream = stream.eventsource();
    let mut response_id: Option<String> = None;
    let mut pending_tool_calls: Vec<PendingToolCall> = Vec::new();
    let mut usage: Option<TokenUsage> = None;
    let mut assistant_message_started = false;
    let mut assistant_message_text = String::new();

    while let Some(next) = stream.next().await {
        let sse = match next {
            Ok(sse) => sse,
            Err(err) => {
                let _ = tx_event.send(Err(ApiError::Stream(err.to_string()))).await;
                return;
            }
        };
        if sse.data == "[DONE]" {
            break;
        }

        let chunk = match serde_json::from_str::<ChatCompletionChunk>(&sse.data) {
            Ok(chunk) => chunk,
            Err(err) => {
                debug!("failed to parse chat completion chunk: {err}");
                continue;
            }
        };
        if response_id.is_none() {
            response_id = chunk.id;
        }
        if let Some(chunk_usage) = chunk.usage {
            usage = Some(TokenUsage {
                input_tokens: chunk_usage.prompt_tokens,
                cached_input_tokens: 0,
                output_tokens: chunk_usage.completion_tokens,
                reasoning_output_tokens: 0,
                total_tokens: chunk_usage.total_tokens,
            });
        }

        for choice in chunk.choices {
            if let Some(content) = choice.delta.content {
                ensure_assistant_message_started(&tx_event, &mut assistant_message_started).await;
                assistant_message_text.push_str(&content);
                let _ = tx_event
                    .send(Ok(ResponseEvent::OutputTextDelta(content)))
                    .await;
            }
            if let Some(tool_calls) = choice.delta.tool_calls {
                for tool_call in tool_calls {
                    if pending_tool_calls.len() <= tool_call.index {
                        pending_tool_calls
                            .resize_with(tool_call.index + 1, PendingToolCall::default);
                    }
                    let pending = &mut pending_tool_calls[tool_call.index];
                    if let Some(id) = tool_call.id {
                        pending.id = Some(id);
                    }
                    if let Some(function) = tool_call.function {
                        if let Some(name) = function.name {
                            pending.name = Some(name);
                        }
                        if let Some(arguments) = function.arguments {
                            pending.arguments.push_str(&arguments);
                        }
                    }
                }
            }
            if choice.finish_reason.as_deref() == Some("tool_calls") {
                flush_tool_calls(&tx_event, &mut pending_tool_calls).await;
            }
        }
    }

    flush_tool_calls(&tx_event, &mut pending_tool_calls).await;
    if assistant_message_started {
        let item = ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: assistant_message_text,
            }],
            phase: None,
        };
        let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
    }
    let response_id = response_id.unwrap_or_else(|| "chatcmpl-adapter".to_string());
    let _ = tx_event
        .send(Ok(ResponseEvent::Completed {
            response_id,
            token_usage: usage,
            end_turn: None,
        }))
        .await;
}

async fn ensure_assistant_message_started(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    assistant_message_started: &mut bool,
) {
    if *assistant_message_started {
        return;
    }

    *assistant_message_started = true;
    let item = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: Vec::new(),
        phase: None,
    };
    let _ = tx_event
        .send(Ok(ResponseEvent::OutputItemAdded(item)))
        .await;
}

async fn flush_tool_calls(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    pending_tool_calls: &mut Vec<PendingToolCall>,
) {
    for pending in pending_tool_calls.drain(..) {
        let (Some(call_id), Some(name)) = (pending.id, pending.name) else {
            continue;
        };
        let item = ResponseItem::FunctionCall {
            id: None,
            name,
            namespace: None,
            arguments: pending.arguments,
            call_id,
        };
        let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
    }
}
