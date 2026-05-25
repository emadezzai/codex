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
use regex_lite::Regex;
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

struct ExtractedXmlToolCall {
    name: String,
    arguments: String,
}

/// Some providers (e.g. MiniMax) emit tool calls as XML text inside regular
/// `text` content blocks instead of using Anthropic-native `tool_use` blocks.
/// This function extracts any `<tool_call>` blocks and parses their tool name
/// and parameter attributes.
fn extract_xml_tool_calls(text: &str) -> (String, Vec<ExtractedXmlToolCall>) {
    let mut xml_tool_calls = Vec::new();
    let tool_call_re = Regex::new(r"(?s)<tool_call>(.*?)</tool_call>").unwrap();
    let name_re = Regex::new(r#"^\s*(?:"([^"]+)"|'([^']+)'|([a-zA-Z_][a-zA-Z0-9_-]*))"#).unwrap();
    let param_re =
        Regex::new(r#"([a-zA-Z_][a-zA-Z0-9_-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap();

    let mut remaining_text = String::new();
    let mut last_idx = 0;

    for caps in tool_call_re.captures_iter(text) {
        let full_match = caps.get(0).unwrap();
        let start = full_match.start();
        let end = full_match.end();

        remaining_text.push_str(&text[last_idx..start]);
        last_idx = end;

        let inner = &caps[1];
        if let Some(name_caps) = name_re.captures(inner) {
            let name = name_caps
                .get(1)
                .or_else(|| name_caps.get(2))
                .or_else(|| name_caps.get(3))
                .map(|m| m.as_str())
                .unwrap_or_default()
                .to_string();

            if !name.is_empty() {
                let mut args_map = serde_json::Map::new();
                for param_caps in param_re.captures_iter(inner) {
                    let key = param_caps
                        .get(1)
                        .map(|m| m.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let val = param_caps
                        .get(2)
                        .or_else(|| param_caps.get(3))
                        .or_else(|| param_caps.get(4))
                        .map(|m| m.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if !key.is_empty() {
                        args_map.insert(key, serde_json::Value::String(val));
                    }
                }

                let arguments = serde_json::to_string(&serde_json::Value::Object(args_map))
                    .unwrap_or_else(|_| "{}".to_string());

                xml_tool_calls.push(ExtractedXmlToolCall { name, arguments });
            }
        }
    }

    remaining_text.push_str(&text[last_idx..]);
    (remaining_text, xml_tool_calls)
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
                    let reas_id = format!("reas-{}-{index}", chrono::Utc::now().timestamp_millis());
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
                            if let ContentBlockState::Thinking {
                                thinking: accum, ..
                            } = state
                            {
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
                            let (remaining_text, xml_tool_calls) = extract_xml_tool_calls(&text);

                            if xml_tool_calls.is_empty() {
                                let item = ResponseItem::Message {
                                    id: Some(id),
                                    role: "assistant".to_string(),
                                    content: vec![ContentItem::OutputText { text }],
                                    phase: None,
                                };
                                let _ =
                                    tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
                            } else {
                                debug!(
                                    "extracted {} XML tool call(s) from text block",
                                    xml_tool_calls.len()
                                );
                                let text_content = if remaining_text.is_empty() {
                                    vec![]
                                } else {
                                    vec![ContentItem::OutputText {
                                        text: remaining_text,
                                    }]
                                };
                                let item = ResponseItem::Message {
                                    id: Some(id),
                                    role: "assistant".to_string(),
                                    content: text_content,
                                    phase: None,
                                };
                                let _ =
                                    tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;

                                for tc in xml_tool_calls {
                                    let tc_id = format!(
                                        "xml-{}",
                                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
                                    );
                                    let tc_item = ResponseItem::FunctionCall {
                                        id: Some(tc_id.clone()),
                                        name: tc.name,
                                        namespace: None,
                                        arguments: tc.arguments,
                                        call_id: tc_id,
                                    };
                                    let _ = tx_event
                                        .send(Ok(ResponseEvent::OutputItemAdded(tc_item.clone())))
                                        .await;
                                    let _ = tx_event
                                        .send(Ok(ResponseEvent::OutputItemDone(tc_item)))
                                        .await;
                                }
                            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_extract_xml_tool_calls_no_calls() {
        let text = "Hello world, no tool calls here.";
        let (remaining, calls) = extract_xml_tool_calls(text);
        assert_eq!(remaining, "Hello world, no tool calls here.");
        assert!(calls.is_empty());
    }

    #[test]
    fn test_extract_xml_tool_calls_single_quoted_name() {
        let text = "Here is a call: <tool_call>\n\"open_file\" path=\"/path/to/file.txt\"\n</tool_call>\nHope that helps.";
        let (remaining, calls) = extract_xml_tool_calls(text);
        assert_eq!(remaining.trim(), "Here is a call: \nHope that helps.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "open_file");
        assert_eq!(calls[0].arguments, "{\"path\":\"/path/to/file.txt\"}");
    }

    #[test]
    fn test_extract_xml_tool_calls_unquoted_name_and_multiple_params() {
        let text = "Running... <tool_call>\nrun_command command=\"cargo build\" Cwd=\"/workspace\"\n</tool_call>\nDone.";
        let (remaining, calls) = extract_xml_tool_calls(text);
        assert_eq!(remaining.trim(), "Running... \nDone.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "run_command");
        let parsed_args: serde_json::Value = serde_json::from_str(&calls[0].arguments).unwrap();
        assert_eq!(parsed_args["command"], "cargo build");
        assert_eq!(parsed_args["Cwd"], "/workspace");
    }

    #[test]
    fn test_extract_xml_tool_calls_multiple_calls() {
        let text = "First: <tool_call>\"t1\" p=\"v1\"</tool_call> and second: <tool_call>\"t2\" p=\"v2\"</tool_call>.";
        let (remaining, calls) = extract_xml_tool_calls(text);
        assert_eq!(remaining, "First:  and second: .");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "t1");
        assert_eq!(calls[0].arguments, "{\"p\":\"v1\"}");
        assert_eq!(calls[1].name, "t2");
        assert_eq!(calls[1].arguments, "{\"p\":\"v2\"}");
    }
}
