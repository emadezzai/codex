//! Inbound OpenAI Chat Completions SSE event translator.

use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::rate_limits::parse_all_rate_limits;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
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
    let upstream_request_id = stream_response
        .headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        for snapshot in rate_limit_snapshots {
            let _ = tx_event.send(Ok(ResponseEvent::RateLimits(snapshot))).await;
        }
        process_sse(stream_response.bytes, tx_event, idle_timeout, telemetry).await;
    });

    ResponseStream {
        rx_event,
        upstream_request_id,
    }
}

#[derive(Debug, Deserialize)]
struct ChatChunk {
    id: Option<String>,
    model: Option<String>,
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
    role: Option<String>,
    content: Option<String>,
    tool_calls: Option<Vec<ChatToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ChatToolCallDelta {
    index: usize,
    id: Option<String>,
    function: Option<ChatFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct ChatFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    total_tokens: Option<i64>,
}

struct ToolCallState {
    id: String,
    name: String,
    arguments: String,
}

pub async fn process_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = stream.eventsource();
    let mut response_id = String::new();
    let mut msg_item_id = String::new();
    let mut input_tokens: i64 = 0;
    let mut output_tokens: i64 = 0;
    let mut tool_states: HashMap<usize, ToolCallState> = HashMap::new();
    let mut started = false;

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
                // Stream ended without [DONE]
                if started {
                    let usage = TokenUsage {
                        input_tokens,
                        cached_input_tokens: 0,
                        output_tokens,
                        reasoning_output_tokens: 0,
                        total_tokens: input_tokens + output_tokens,
                    };
                    let _ = tx_event
                        .send(Ok(ResponseEvent::Completed {
                            response_id: response_id.clone(),
                            token_usage: Some(usage),
                            end_turn: Some(true),
                        }))
                        .await;
                } else {
                    let _ = tx_event
                        .send(Err(ApiError::Stream(
                            "stream closed before any data".into(),
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

        if sse.data == "[DONE]" {
            // Finalize any open tool calls
            for (_, state) in tool_states.drain() {
                let item = ResponseItem::FunctionCall {
                    id: Some(state.id.clone()),
                    name: state.name,
                    namespace: None,
                    arguments: state.arguments,
                    call_id: state.id,
                };
                let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
            }
            // Finalize open text message if any
            if !msg_item_id.is_empty() {
                let item = ResponseItem::Message {
                    id: Some(msg_item_id.clone()),
                    role: "assistant".to_string(),
                    content: vec![],
                    phase: None,
                };
                let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
            }
            let usage = TokenUsage {
                input_tokens,
                cached_input_tokens: 0,
                output_tokens,
                reasoning_output_tokens: 0,
                total_tokens: input_tokens + output_tokens,
            };
            let _ = tx_event
                .send(Ok(ResponseEvent::Completed {
                    response_id: response_id.clone(),
                    token_usage: Some(usage),
                    end_turn: Some(true),
                }))
                .await;
            return;
        }

        let chunk: ChatChunk = match serde_json::from_str(&sse.data) {
            Ok(c) => c,
            Err(e) => {
                debug!(
                    "Failed to parse Chat Completions SSE chunk: {e}, data: {}",
                    &sse.data
                );
                continue;
            }
        };

        // Capture usage if present (may arrive on last chunk)
        if let Some(usage) = &chunk.usage {
            input_tokens = usage.prompt_tokens.unwrap_or(input_tokens);
            output_tokens = usage.completion_tokens.unwrap_or(output_tokens);
        }

        if let Some(id) = &chunk.id
            && response_id.is_empty()
        {
            response_id = id.clone();
            let _ = tx_event.send(Ok(ResponseEvent::Created)).await;
            started = true;
        }
        if let Some(model) = &chunk.model
            && !model.is_empty()
            && started
            && response_id.len() == chunk.id.as_deref().unwrap_or("").len()
        {
            // Only emit once, on first chunk
            let _ = tx_event
                .send(Ok(ResponseEvent::ServerModel(model.clone())))
                .await;
        }

        for choice in &chunk.choices {
            let delta = &choice.delta;

            // Text content delta
            if let Some(text) = &delta.content
                && !text.is_empty()
            {
                if msg_item_id.is_empty() {
                    msg_item_id = format!("msg-{}", chrono::Utc::now().timestamp_millis());
                    let item = ResponseItem::Message {
                        id: Some(msg_item_id.clone()),
                        role: "assistant".to_string(),
                        content: vec![],
                        phase: None,
                    };
                    let _ = tx_event
                        .send(Ok(ResponseEvent::OutputItemAdded(item)))
                        .await;
                }
                let _ = tx_event
                    .send(Ok(ResponseEvent::OutputTextDelta(text.clone())))
                    .await;
            }

            // Tool call deltas
            if let Some(tool_deltas) = &delta.tool_calls {
                for td in tool_deltas {
                    let state = tool_states
                        .entry(td.index)
                        .or_insert_with(|| ToolCallState {
                            id: String::new(),
                            name: String::new(),
                            arguments: String::new(),
                        });

                    if let Some(id) = &td.id
                        && state.id.is_empty()
                    {
                        state.id = id.clone();
                    }

                    if let Some(func) = &td.function {
                        if let Some(name) = &func.name
                            && state.name.is_empty()
                        {
                            state.name = name.clone();
                            // Emit OutputItemAdded when we first know the tool name
                            let item = ResponseItem::FunctionCall {
                                id: Some(state.id.clone()),
                                name: state.name.clone(),
                                namespace: None,
                                arguments: String::new(),
                                call_id: state.id.clone(),
                            };
                            let _ = tx_event
                                .send(Ok(ResponseEvent::OutputItemAdded(item)))
                                .await;
                        }
                        if let Some(args) = &func.arguments {
                            state.arguments.push_str(args);
                            let _ = tx_event
                                .send(Ok(ResponseEvent::ToolCallInputDelta {
                                    item_id: state.id.clone(),
                                    call_id: Some(state.id.clone()),
                                    delta: args.clone(),
                                }))
                                .await;
                        }
                    }
                }
            }

            // Finalize completed tool calls when finish_reason == "tool_calls"
            if choice.finish_reason.as_deref() == Some("tool_calls") {
                for (_, state) in tool_states.drain() {
                    // Finalize text message first if open
                    if !msg_item_id.is_empty() {
                        let item = ResponseItem::Message {
                            id: Some(msg_item_id.clone()),
                            role: "assistant".to_string(),
                            content: vec![ContentItem::OutputText {
                                text: String::new(),
                            }],
                            phase: None,
                        };
                        let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                        msg_item_id.clear();
                    }
                    let item = ResponseItem::FunctionCall {
                        id: Some(state.id.clone()),
                        name: state.name,
                        namespace: None,
                        arguments: state.arguments,
                        call_id: state.id,
                    };
                    let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                }
            }

            // Finalize text message on stop
            if choice.finish_reason.as_deref() == Some("stop") && !msg_item_id.is_empty() {
                let item = ResponseItem::Message {
                    id: Some(msg_item_id.clone()),
                    role: "assistant".to_string(),
                    content: vec![],
                    phase: None,
                };
                let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                msg_item_id.clear();
            }
        }
    }
}
