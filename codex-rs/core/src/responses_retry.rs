//! Shared retry and transport fallback decisions for Responses requests.

use std::time::Duration;

use crate::client::ModelClientSession;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::util::backoff;
use codex_protocol::error::CodexErr;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::WarningEvent;
use tracing::warn;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ResponsesStreamRequest {
    Sampling,
    RemoteCompactionV2,
}

/// Handles a retryable stream error and returns `Ok(())` when the caller should
/// retry the request loop.
pub(crate) async fn handle_retryable_response_stream_error(
    retries: &mut u64,
    max_retries: u64,
    err: CodexErr,
    client_session: &mut ModelClientSession,
    sess: &Session,
    turn_context: &TurnContext,
    request: ResponsesStreamRequest,
) -> Result<(), CodexErr> {
    if *retries >= max_retries
        && client_session.try_switch_fallback_transport(
            &turn_context.session_telemetry,
            &turn_context.model_info,
        )
    {
        sess.send_event(
            turn_context,
            EventMsg::Warning(WarningEvent {
                message: format!("Falling back from WebSockets to HTTPS transport. {err:#}"),
            }),
        )
        .await;
        *retries = 0;
        return Ok(());
    }

    if *retries < max_retries {
        *retries += 1;
        let retry_count = *retries;
        let delay = match &err {
            CodexErr::Stream(_, requested_delay) => {
                requested_delay.unwrap_or_else(|| backoff(retry_count))
            }
            _ => backoff(retry_count),
        };
        log_retry(request, turn_context, &err, retry_count, max_retries, delay);

        // In release builds, hide the first websocket retry notification to reduce noisy
        // transient reconnect messages. In debug builds, keep full visibility for diagnosis.
        let report_error = retry_count > 1
            || cfg!(debug_assertions)
            || !sess.services.model_client.responses_websocket_enabled();
        if report_error {
            // Surface retry information to any UI/front-end so the user understands what is
            // happening instead of staring at a seemingly frozen screen.
            sess.notify_stream_error(
                turn_context,
                format!("Reconnecting... {retry_count}/{max_retries}"),
                err,
            )
            .await;
        }
        tokio::time::sleep(delay).await;
        return Ok(());
    }

    Err(err)
}

fn log_retry(
    request: ResponsesStreamRequest,
    turn_context: &TurnContext,
    err: &CodexErr,
    retries: u64,
    max_retries: u64,
    delay: Duration,
) {
    match request {
        ResponsesStreamRequest::Sampling => {
            warn!(
                "stream disconnected - retrying sampling request ({retries}/{max_retries} in {delay:?})...",
            );
        }
        ResponsesStreamRequest::RemoteCompactionV2 => {
            warn!(
                turn_id = %turn_context.sub_id,
                retries,
                max_retries,
                compact_error = %err,
                "remote compaction v2 stream failed; retrying request after delay"
            );
        }
    }
}

/// Returns `true` when the error indicates the Responses API is unusable for this
/// model/request and the caller should immediately retry with the Chat Completions API.
///
/// Detected conditions:
/// - The gateway rejected Responses API access for the model (HTTP 400/4xx with
///   `code` `400005` or a message mentioning "does not allow Responses API").
pub(crate) fn should_fallback_to_chat(err: &CodexErr) -> bool {
    match err {
        CodexErr::InvalidRequest(body) => {
            body.contains("does not allow Responses API") || body.contains("\"code\":\"400005\"")
        }
        CodexErr::UnexpectedStatus(unexpected) => {
            unexpected.body.contains("does not allow Responses API")
                || unexpected.body.contains("\"code\":\"400005\"")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::error::UnexpectedResponseError;
    use http::StatusCode;

    #[test]
    fn stream_closed_before_completed_does_not_trigger_fallback() {
        let err = CodexErr::Stream("stream closed before response.completed".into(), None);
        assert!(!should_fallback_to_chat(&err));
    }

    #[test]
    fn unrelated_stream_error_does_not_trigger_fallback() {
        let err = CodexErr::Stream("some other stream error".into(), None);
        assert!(!should_fallback_to_chat(&err));
    }

    #[test]
    fn gateway_400005_body_triggers_fallback() {
        let body = r#"{"error":{"message":"model 'deepseek-v4-pro' does not allow Responses API access via Chat Completions conversion","type":"gateway_error","param":"","code":"400005"}}"#;
        assert!(should_fallback_to_chat(&CodexErr::InvalidRequest(
            body.into()
        )));
    }

    #[test]
    fn code_400005_alone_triggers_fallback() {
        let body = r#"{"error":{"code":"400005","message":"unrelated message"}}"#;
        assert!(should_fallback_to_chat(&CodexErr::InvalidRequest(
            body.into()
        )));
    }

    #[test]
    fn generic_invalid_request_does_not_trigger_fallback() {
        assert!(!should_fallback_to_chat(&CodexErr::InvalidRequest(
            "invalid model name".into()
        )));
    }

    #[test]
    fn unexpected_status_with_responses_rejection_triggers_fallback() {
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
    fn unexpected_status_other_does_not_trigger_fallback() {
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
    fn non_stream_non_status_errors_do_not_trigger_fallback() {
        assert!(!should_fallback_to_chat(&CodexErr::Timeout));
        assert!(!should_fallback_to_chat(&CodexErr::TurnAborted));
    }
}
