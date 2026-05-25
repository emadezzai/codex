//! Outbound OpenAI Chat Completions request builder and translator.

use crate::common::ResponsesApiRequest;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ResponseItem;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

pub const CHAT_COMPLETIONS_DEFAULT_MAX_TOKENS: i64 = 40_960;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionsRequest {
    pub model: String,
    pub max_tokens: i64,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    pub stream: bool,
    pub stream_options: ChatStreamOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Option<ChatContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatContent {
    Text(String),
    Parts(Vec<ChatContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChatContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ChatImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolCall {
    pub id: String,
    pub r#type: String,
    pub function: ChatToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatToolCallFunction {
    pub name: String,
    pub arguments: String,
}

pub fn translate_request(request: &ResponsesApiRequest) -> ChatCompletionsRequest {
    let mut messages: Vec<ChatMessage> = Vec::new();

    // Collect system content from instructions and system messages
    let mut system_parts = Vec::new();
    if !request.instructions.is_empty() {
        system_parts.push(request.instructions.clone());
    }
    for item in &request.input {
        if let ResponseItem::Message { role, content, .. } = item
            && role == "system"
        {
            for block in content {
                if let ContentItem::InputText { text } | ContentItem::OutputText { text } = block {
                    system_parts.push(text.clone());
                }
            }
        }
    }
    if !system_parts.is_empty() {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some(ChatContent::Text(system_parts.join("\n\n"))),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    for item in &request.input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                if role == "system" {
                    continue;
                }
                let role = if role == "user" { "user" } else { "assistant" };
                let parts: Vec<ChatContentPart> = content
                    .iter()
                    .filter_map(|c| match c {
                        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                            Some(ChatContentPart::Text { text: text.clone() })
                        }
                        ContentItem::InputImage { image_url, .. } => {
                            Some(ChatContentPart::ImageUrl {
                                image_url: ChatImageUrl {
                                    url: image_url.clone(),
                                },
                            })
                        }
                    })
                    .collect();

                let content = if parts.is_empty() {
                    None
                } else if parts.len() == 1 {
                    if let ChatContentPart::Text { text } = &parts[0] {
                        Some(ChatContent::Text(text.clone()))
                    } else {
                        Some(ChatContent::Parts(parts))
                    }
                } else {
                    Some(ChatContent::Parts(parts))
                };

                append_or_merge_chat_message(&mut messages, role, content, None, None);
            }
            ResponseItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                let tool_call = ChatToolCall {
                    id: call_id.clone(),
                    r#type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: name.clone(),
                        arguments: arguments.clone(),
                    },
                };
                append_or_merge_chat_message(
                    &mut messages,
                    "assistant",
                    None,
                    Some(vec![tool_call]),
                    None,
                );
            }
            ResponseItem::FunctionCallOutput { call_id, output }
            | ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                let text = match &output.body {
                    FunctionCallOutputBody::Text(t) => t.clone(),
                    FunctionCallOutputBody::ContentItems(items) => items
                        .iter()
                        .filter_map(|item| {
                            if let FunctionCallOutputContentItem::InputText { text } = item {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(ChatContent::Text(text)),
                    tool_calls: None,
                    tool_call_id: Some(call_id.clone()),
                });
            }
            _ => {}
        }
    }

    let tool_choice = translate_tool_choice(&request.tool_choice);
    let tools = translate_tools_for_chat_completions(&request.tools);

    ChatCompletionsRequest {
        model: request.model.clone(),
        max_tokens: CHAT_COMPLETIONS_DEFAULT_MAX_TOKENS,
        messages,
        tools,
        tool_choice,
        temperature: None,
        stream: request.stream,
        stream_options: ChatStreamOptions {
            include_usage: true,
        },
    }
}

fn append_or_merge_chat_message(
    messages: &mut Vec<ChatMessage>,
    role: &str,
    content: Option<ChatContent>,
    tool_calls: Option<Vec<ChatToolCall>>,
    tool_call_id: Option<String>,
) {
    // Only merge consecutive assistant text messages (not tool calls)
    if tool_calls.is_none()
        && tool_call_id.is_none()
        && let Some(last) = messages.last_mut()
        && last.role == role
        && last.tool_calls.is_none()
        && let (Some(ChatContent::Text(existing)), Some(ChatContent::Text(new))) =
            (&mut last.content, &content)
    {
        existing.push_str(new);
        return;
    }
    messages.push(ChatMessage {
        role: role.to_string(),
        content,
        tool_calls,
        tool_call_id,
    });
}

fn translate_tool_choice(openai_choice: &str) -> Option<Value> {
    match openai_choice {
        "auto" => Some(Value::String("auto".to_string())),
        "required" => Some(Value::String("required".to_string())),
        "none" => Some(Value::String("none".to_string())),
        other => serde_json::from_str(other).ok(),
    }
}

fn translate_tools_for_chat_completions(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let type_hint = tool.get("type")?.as_str()?;
            if type_hint != "function" {
                return Some(tool.clone());
            }
            let name = tool.get("name")?.as_str()?.to_string();
            let description = tool
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            let parameters = tool.get("parameters").cloned().unwrap_or(Value::Null);
            Some(serde_json::json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": parameters
                }
            }))
        })
        .collect()
}
