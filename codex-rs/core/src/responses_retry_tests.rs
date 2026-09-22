use super::ResponsesStreamRequest;
use super::log_retry;
use super::should_fallback_to_chat;
use crate::session::tests::make_session_and_context;
use codex_protocol::error::CodexErr;
use codex_protocol::error::UnexpectedResponseError;
use http::StatusCode;
use std::time::Duration;
use tracing_test::internal::MockWriter;

#[tokio::test]
async fn sampling_retry_logs_stream_error_context() {
    let (_session, turn_context) = make_session_and_context().await;
    let buffer: &'static std::sync::Mutex<Vec<u8>> =
        Box::leak(Box::new(std::sync::Mutex::new(Vec::new())));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(MockWriter::new(buffer))
        .finish();
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);

    log_retry(
        ResponsesStreamRequest::Sampling,
        &turn_context,
        &CodexErr::Stream("websocket closed by server before response.completed".to_string()),
        /*retries*/ 2,
        /*max_retries*/ 5,
        Duration::from_secs(1),
    );

    let logs = String::from_utf8(
        buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone(),
    )
    .expect("retry log should be valid utf-8");
    assert!(logs.contains("stream disconnected - retrying sampling request"));
    assert!(logs.contains(&format!("turn_id={}", turn_context.sub_id)));
    assert!(logs.contains("retries=2"));
    assert!(logs.contains("max_retries=5"));
    assert!(logs.contains(
        "sampling_error=stream disconnected before completion: websocket closed by server before response.completed"
    ));
}

#[test]
fn stream_closed_before_completed_does_not_trigger_chat_fallback() {
    let err = CodexErr::Stream("stream closed before response.completed".into());
    assert!(!should_fallback_to_chat(&err));
}

#[test]
fn unrelated_stream_error_does_not_trigger_chat_fallback() {
    let err = CodexErr::Stream("some other stream error".into());
    assert!(!should_fallback_to_chat(&err));
}

#[test]
fn gateway_400005_body_triggers_chat_fallback() {
    let body = r#"{"error":{"message":"model 'deepseek-v4-pro' does not allow Responses API access via Chat Completions conversion","type":"gateway_error","param":"","code":"400005"}}"#;
    assert!(should_fallback_to_chat(&CodexErr::InvalidRequest(
        body.into()
    )));
}

#[test]
fn code_400005_alone_triggers_chat_fallback() {
    let body = r#"{"error":{"code":"400005","message":"unrelated message"}}"#;
    assert!(should_fallback_to_chat(&CodexErr::InvalidRequest(
        body.into()
    )));
}

#[test]
fn generic_invalid_request_does_not_trigger_chat_fallback() {
    assert!(!should_fallback_to_chat(&CodexErr::InvalidRequest(
        "invalid model name".into()
    )));
}

#[test]
fn unexpected_status_with_responses_rejection_triggers_chat_fallback() {
    let body = r#"{"error":{"message":"does not allow Responses API access"}}"#;
    let err = CodexErr::UnexpectedStatus(UnexpectedResponseError {
        status: StatusCode::FORBIDDEN,
        body: body.into(),
        user_message: None,
        url: None,
        cf_ray: None,
        request_id: None,
        identity_authorization_error: None,
        identity_error_code: None,
    });
    assert!(should_fallback_to_chat(&err));
}

#[test]
fn unexpected_status_other_does_not_trigger_chat_fallback() {
    let err = CodexErr::UnexpectedStatus(UnexpectedResponseError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        body: "internal server error".into(),
        user_message: None,
        url: None,
        cf_ray: None,
        request_id: None,
        identity_authorization_error: None,
        identity_error_code: None,
    });
    assert!(!should_fallback_to_chat(&err));
}

#[test]
fn non_stream_non_status_errors_do_not_trigger_chat_fallback() {
    assert!(!should_fallback_to_chat(&CodexErr::Timeout));
    assert!(!should_fallback_to_chat(&CodexErr::TurnAborted));
}
