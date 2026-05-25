//! Inbound Anthropic SSE event translator.

use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::rate_limits::parse_all_rate_limits;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

pub fn spawn_response_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    _turn_state: Option<Arc<OnceLock<String>>>,
) -> ResponseStream {
    let rate_limit_snapshots = parse_all_rate_limits(&stream_response.headers);
    let models_etag = stream_response
        .headers
        .get("X-Models-Etag")
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string);
    let upstream_request_id = stream_response
        .headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        for snapshot in rate_limit_snapshots {
            let _ = tx_event.send(Ok(ResponseEvent::RateLimits(snapshot))).await;
        }
        if let Some(etag) = models_etag {
            let _ = tx_event.send(Ok(ResponseEvent::ModelsEtag(etag))).await;
        }
        process_sse(stream_response.bytes, tx_event, idle_timeout, telemetry).await;
    });

    ResponseStream {
        rx_event,
        upstream_request_id,
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicSseEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: AnthropicMessageStartInfo },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: i64,
        content_block: AnthropicContentBlockStartInfo,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: i64,
        delta: AnthropicContentBlockDeltaInfo,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: i64 },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: AnthropicMessageDeltaInfo,
        usage: Option<AnthropicMessageDeltaUsage>,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "error")]
    Error { error: AnthropicSseErrorInfo },
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageStartInfo {
    id: String,
    model: String,
    usage: AnthropicMessageStartUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageStartUsage {
    input_tokens: i64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlockStartInfo {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    // `tool_use` blocks open with an empty `input`; the argument JSON arrives via
    // subsequent `input_json_delta` events, so we don't read it here.
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)]
enum AnthropicContentBlockDeltaInfo {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    // Extended-thinking blocks emit a `signature_delta` carrying a cryptographic
    // signature for the reasoning. Codex has no use for it, so accept and ignore.
    #[serde(rename = "signature_delta")]
    SignatureDelta {
        #[allow(dead_code)]
        signature: String,
    },
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDeltaInfo {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDeltaUsage {
    output_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct AnthropicSseErrorInfo {
    message: String,
}

enum ContentBlockState {
    Text {
        id: String,
        text: String,
    },
    Thinking {
        id: String,
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        partial_json: String,
    },
}

pub async fn process_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = stream.eventsource();
    let mut active_blocks: HashMap<i64, ContentBlockState> = HashMap::new();
    let mut message_id = String::new();
    let mut input_tokens = 0;
    let mut output_tokens = 0;
    let mut _stop_reason = None;

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }

        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(e))) => {
                debug!("SSE Error: {e:#}");
                let _ = tx_event.send(Err(ApiError::Stream(e.to_string()))).await;
                return;
            }
            Ok(None) => {
                if !message_id.is_empty() {
                    let usage = TokenUsage {
                        input_tokens,
                        cached_input_tokens: 0,
                        output_tokens,
                        reasoning_output_tokens: 0,
                        total_tokens: input_tokens + output_tokens,
                    };
                    let _ = tx_event
                        .send(Ok(ResponseEvent::Completed {
                            response_id: message_id,
                            token_usage: Some(usage),
                            end_turn: Some(true),
                        }))
                        .await;
                } else {
                    let _ = tx_event
                        .send(Err(ApiError::Stream(
                            "stream closed before response.completed".into(),
                        )))
                        .await;
                }
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };

        trace!("SSE event: {}", &sse.data);

        if sse.event == "ping" {
            continue;
        }

        let event: AnthropicSseEvent = match serde_json::from_str(&sse.data) {
            Ok(event) => event,
            Err(e) => {
                debug!("Failed to parse SSE event: {e}, data: {}", &sse.data);
                continue;
            }
        };

        match event {
            AnthropicSseEvent::MessageStart { message } => {
                message_id = message.id.clone();
                input_tokens = message.usage.input_tokens;
                let _ = tx_event.send(Ok(ResponseEvent::Created)).await;
                let _ = tx_event
                    .send(Ok(ResponseEvent::ServerModel(message.model)))
                    .await;
            }
            AnthropicSseEvent::ContentBlockStart {
                index,
                content_block,
            } => match content_block {
                AnthropicContentBlockStartInfo::Text { text } => {
                    let msg_id = format!("msg-{}-{index}", chrono::Utc::now().timestamp_millis());
                    active_blocks.insert(
                        index,
                        ContentBlockState::Text {
                            id: msg_id.clone(),
                            text,
                        },
                    );
                    // Open an assistant message item so streamed text deltas have an
                    // active item to attach to.
                    let item = ResponseItem::Message {
                        id: Some(msg_id),
                        role: "assistant".to_string(),
                        content: vec![],
                        phase: None,
                    };
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemAdded(item)))
                        .await;
                }
                AnthropicContentBlockStartInfo::Thinking { thinking } => {
                    let reas_id =
                        format!("reas-{}-{index}", chrono::Utc::now().timestamp_millis());
                    active_blocks.insert(
                        index,
                        ContentBlockState::Thinking {
                            id: reas_id.clone(),
                            thinking,
                        },
                    );
                    // Open a reasoning item so downstream consumers have an active
                    // item to attach the streamed thinking deltas to.
                    let item = ResponseItem::Reasoning {
                        id: reas_id,
                        summary: vec![],
                        content: Some(vec![]),
                        encrypted_content: None,
                    };
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemAdded(item)))
                        .await;
                }
                AnthropicContentBlockStartInfo::ToolUse { id, name } => {
                    active_blocks.insert(
                        index,
                        ContentBlockState::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            partial_json: String::new(),
                        },
                    );
                    let item = ResponseItem::FunctionCall {
                        id: Some(id.clone()),
                        name,
                        namespace: None,
                        arguments: String::new(),
                        call_id: id,
                    };
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemAdded(item)))
                        .await;
                }
            },
            AnthropicSseEvent::ContentBlockDelta { index, delta } => {
                if let Some(state) = active_blocks.get_mut(&index) {
                    match delta {
                        AnthropicContentBlockDeltaInfo::TextDelta { text } => {
                            if let ContentBlockState::Text { text: accum, .. } = state {
                                accum.push_str(&text);
                            }
                            let _ = tx_event
                                .send(Ok(ResponseEvent::OutputTextDelta(text)))
                                .await;
                        }
                        AnthropicContentBlockDeltaInfo::ThinkingDelta { thinking } => {
                            if let ContentBlockState::Thinking { thinking: accum, .. } = state {
                                accum.push_str(&thinking);
                            }
                            let _ = tx_event
                                .send(Ok(ResponseEvent::ReasoningContentDelta {
                                    delta: thinking,
                                    content_index: index,
                                }))
                                .await;
                        }
                        AnthropicContentBlockDeltaInfo::SignatureDelta { .. } => {}
                        AnthropicContentBlockDeltaInfo::InputJsonDelta { partial_json } => {
                            if let ContentBlockState::ToolUse {
                                id,
                                partial_json: accum,
                                ..
                            } = state
                            {
                                accum.push_str(&partial_json);
                                let _ = tx_event
                                    .send(Ok(ResponseEvent::ToolCallInputDelta {
                                        item_id: id.clone(),
                                        call_id: Some(id.clone()),
                                        delta: partial_json,
                                    }))
                                    .await;
                            }
                        }
                    }
                }
            }
            AnthropicSseEvent::ContentBlockStop { index } => {
                if let Some(state) = active_blocks.remove(&index) {
                    match state {
                        ContentBlockState::Text { id, text } => {
                            let item = ResponseItem::Message {
                                id: Some(id),
                                role: "assistant".to_string(),
                                content: vec![ContentItem::OutputText { text }],
                                phase: None,
                            };
                            let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                        }
                        ContentBlockState::Thinking { id, thinking } => {
                            let item = ResponseItem::Reasoning {
                                id,
                                summary: vec![],
                                content: Some(vec![ReasoningItemContent::ReasoningText {
                                    text: thinking,
                                }]),
                                encrypted_content: None,
                            };
                            let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                        }
                        ContentBlockState::ToolUse {
                            id,
                            name,
                            partial_json,
                        } => {
                            let item = ResponseItem::FunctionCall {
                                id: Some(id.clone()),
                                name,
                                namespace: None,
                                arguments: partial_json,
                                call_id: id,
                            };
                            let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                        }
                    }
                }
            }
            AnthropicSseEvent::MessageDelta { delta, usage } => {
                _stop_reason = delta.stop_reason;
                if let Some(u) = usage {
                    output_tokens = u.output_tokens;
                }
            }
            AnthropicSseEvent::MessageStop => {
                let usage = TokenUsage {
                    input_tokens,
                    cached_input_tokens: 0,
                    output_tokens,
                    reasoning_output_tokens: 0,
                    total_tokens: input_tokens + output_tokens,
                };
                let _ = tx_event
                    .send(Ok(ResponseEvent::Completed {
                        response_id: message_id.clone(),
                        token_usage: Some(usage),
                        end_turn: Some(true),
                    }))
                    .await;
                return;
            }
            AnthropicSseEvent::Error { error } => {
                let api_err = ApiError::Stream(error.message);
                let _ = tx_event.send(Err(api_err)).await;
                return;
            }
        }
    }
}
