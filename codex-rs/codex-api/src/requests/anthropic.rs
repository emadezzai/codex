//! Outbound Anthropic Messages request builder and translator.

use crate::common::ResponsesApiRequest;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ResponseItem;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// The Anthropic Messages API requires `max_tokens`. MiniMax's M-series models
/// support large completions, so default to a generous cap when the caller does
/// not specify one.
pub const ANTHROPIC_DEFAULT_MAX_TOKENS: i64 = 40_960;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessageRequest {
    pub model: String,
    pub max_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AnthropicTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<AnthropicToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: AnthropicImageSource },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicImageSource {
    pub r#type: String, // base64
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicToolChoice {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "tool")]
    Tool { name: String },
}

pub fn translate_request(request: &ResponsesApiRequest) -> AnthropicMessageRequest {
    let mut system_parts = Vec::new();
    if !request.instructions.is_empty() {
        system_parts.push(request.instructions.clone());
    }

    for item in &request.input {
        if let ResponseItem::Message { role, content, .. } = item
            && role == "system"
        {
            for block in content {
                match block {
                    ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                        system_parts.push(text.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    let mut messages: Vec<AnthropicMessage> = Vec::new();

    for item in &request.input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                if role == "system" {
                    continue;
                }
                let role = if role == "user" { "user" } else { "assistant" };
                let blocks = content
                    .iter()
                    .filter_map(|c| match c {
                        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                            Some(AnthropicContentBlock::Text { text: text.clone() })
                        }
                        ContentItem::InputImage {
                            image_url,
                            detail: _,
                        } => parse_image_url(image_url)
                            .map(|source| AnthropicContentBlock::Image { source }),
                    })
                    .collect::<Vec<_>>();

                append_or_merge_message(&mut messages, role.to_string(), blocks);
            }
            ResponseItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                let block = AnthropicContentBlock::ToolUse {
                    id: call_id.clone(),
                    name: name.clone(),
                    input: serde_json::from_str(arguments).unwrap_or(Value::Null),
                };
                append_or_merge_message(&mut messages, "assistant".to_string(), vec![block]);
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                let content_val = match &output.body {
                    FunctionCallOutputBody::Text(text) => Value::String(text.clone()),
                    FunctionCallOutputBody::ContentItems(items) => {
                        let blocks = items
                            .iter()
                            .filter_map(|item| match item {
                                FunctionCallOutputContentItem::InputText { text } => {
                                    Some(serde_json::json!({
                                        "type": "text",
                                        "text": text
                                    }))
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>();
                        Value::Array(blocks)
                    }
                };
                let block = AnthropicContentBlock::ToolResult {
                    tool_use_id: call_id.clone(),
                    content: content_val,
                    is_error: output.success.map(|s| !s),
                };
                append_or_merge_message(&mut messages, "user".to_string(), vec![block]);
            }
            ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                let content_val = match &output.body {
                    FunctionCallOutputBody::Text(text) => Value::String(text.clone()),
                    FunctionCallOutputBody::ContentItems(items) => {
                        let blocks = items
                            .iter()
                            .filter_map(|item| match item {
                                FunctionCallOutputContentItem::InputText { text } => {
                                    Some(serde_json::json!({
                                        "type": "text",
                                        "text": text
                                    }))
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>();
                        Value::Array(blocks)
                    }
                };
                let block = AnthropicContentBlock::ToolResult {
                    tool_use_id: call_id.clone(),
                    content: content_val,
                    is_error: output.success.map(|s| !s),
                };
                append_or_merge_message(&mut messages, "user".to_string(), vec![block]);
            }
            _ => {}
        }
    }

    let tools = request
        .tools
        .iter()
        .filter_map(translate_tool)
        .collect::<Vec<_>>();

    let tool_choice = translate_tool_choice(&request.tool_choice);

    AnthropicMessageRequest {
        model: request.model.clone(),
        max_tokens: ANTHROPIC_DEFAULT_MAX_TOKENS,
        system,
        messages,
        tools,
        tool_choice,
        temperature: None,
        stream: request.stream,
    }
}

fn append_or_merge_message(
    messages: &mut Vec<AnthropicMessage>,
    role: String,
    mut blocks: Vec<AnthropicContentBlock>,
) {
    if blocks.is_empty() {
        return;
    }
    if let Some(last) = messages.last_mut()
        && last.role == role
    {
        last.content.append(&mut blocks);
        return;
    }
    messages.push(AnthropicMessage {
        role,
        content: blocks,
    });
}

fn parse_image_url(url: &str) -> Option<AnthropicImageSource> {
    if let Some(rest) = url.strip_prefix("data:") {
        let parts: Vec<&str> = rest.splitn(2, ";base64,").collect();
        if parts.len() == 2 {
            return Some(AnthropicImageSource {
                r#type: "base64".to_string(),
                media_type: parts[0].to_string(),
                data: parts[1].to_string(),
            });
        }
    }
    None
}

fn translate_tool(openai_tool: &Value) -> Option<AnthropicTool> {
    let function = openai_tool.get("function")?;
    let name = function.get("name")?.as_str()?.to_string();
    let description = function
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string();
    let input_schema = function.get("parameters").cloned().unwrap_or_else(|| {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    });
    Some(AnthropicTool {
        name,
        description,
        input_schema,
    })
}

fn translate_tool_choice(openai_choice: &str) -> Option<AnthropicToolChoice> {
    if openai_choice == "auto" {
        Some(AnthropicToolChoice::Auto)
    } else if openai_choice == "required" {
        Some(AnthropicToolChoice::Any)
    } else if openai_choice == "none" {
        None
    } else if let Ok(val) = serde_json::from_str::<Value>(openai_choice) {
        val.get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            .map(|name| AnthropicToolChoice::Tool {
                name: name.to_string(),
            })
    } else {
        None
    }
}
