use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use codex_api::AuthProvider;
use codex_api::Compression;
use codex_api::Provider;
use codex_api::ResponseEvent;
use codex_api::ResponsesClient;
use codex_client::HttpTransport;
use codex_client::Request;
use codex_client::Response;
use codex_client::StreamResponse;
use codex_client::TransportError;
use codex_protocol::models::ResponseItem;
use futures::StreamExt;
use http::HeaderMap;
use http::StatusCode;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[derive(Clone)]
struct FixtureSseTransport {
    body: String,
}

impl FixtureSseTransport {
    fn new(body: String) -> Self {
        Self { body }
    }
}

#[async_trait]
impl HttpTransport for FixtureSseTransport {
    async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
        Err(TransportError::Build("execute should not run".to_string()))
    }

    async fn stream(&self, _req: Request) -> Result<StreamResponse, TransportError> {
        let stream = futures::stream::iter(vec![Ok::<Bytes, TransportError>(Bytes::from(
            self.body.clone(),
        ))]);
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
        stream_idle_timeout: Duration::from_millis(50),
    }
}

fn build_responses_body(events: Vec<Value>) -> String {
    let mut body = String::new();
    for e in events {
        let kind = e
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("fixture event missing type in SSE fixture: {e}"));
        if e.as_object().map(|o| o.len() == 1).unwrap_or(false) {
            body.push_str(&format!("event: {kind}\n\n"));
        } else {
            body.push_str(&format!("event: {kind}\ndata: {e}\n\n"));
        }
    }
    body
}

#[tokio::test]
async fn responses_stream_parses_items_and_completed_end_to_end() -> Result<()> {
    let item1 = serde_json::json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "Hello"}]
        }
    });

    let item2 = serde_json::json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "World"}]
        }
    });

    let completed = serde_json::json!({
        "type": "response.completed",
        "response": { "id": "resp1" }
    });

    let body = build_responses_body(vec![item1, item2, completed]);
    let transport = FixtureSseTransport::new(body);
    let client = ResponsesClient::new(transport, provider("openai"), Arc::new(NoAuth));

    let mut stream = client
        .stream(
            serde_json::json!({"echo": true}),
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
        )
        .await?;

    let mut events = Vec::new();
    while let Some(ev) = stream.next().await {
        events.push(ev?);
    }

    let events: Vec<ResponseEvent> = events
        .into_iter()
        .filter(|ev| !matches!(ev, ResponseEvent::RateLimits(_)))
        .collect();

    assert_eq!(events.len(), 3);

    match &events[0] {
        ResponseEvent::OutputItemDone(ResponseItem::Message { role, .. }) => {
            assert_eq!(role, "assistant");
        }
        other => panic!("unexpected first event: {other:?}"),
    }

    match &events[1] {
        ResponseEvent::OutputItemDone(ResponseItem::Message { role, .. }) => {
            assert_eq!(role, "assistant");
        }
        other => panic!("unexpected second event: {other:?}"),
    }

    match &events[2] {
        ResponseEvent::Completed {
            response_id,
            token_usage,
            end_turn,
        } => {
            assert_eq!(response_id, "resp1");
            assert!(token_usage.is_none());
            assert!(end_turn.is_none());
        }
        other => panic!("unexpected third event: {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn anthropic_stream_parses_text_and_completed_end_to_end() -> Result<()> {
    let message_start = serde_json::json!({
        "type": "message_start",
        "message": {
            "id": "msg1",
            "model": "MiniMax-M2",
            "usage": { "input_tokens": 10 }
        }
    });

    let block_start = serde_json::json!({
        "type": "content_block_start",
        "index": 0,
        "content_block": {
            "type": "text",
            "text": ""
        }
    });

    let delta1 = serde_json::json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": {
            "type": "text_delta",
            "text": "Hello"
        }
    });

    let delta2 = serde_json::json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": {
            "type": "text_delta",
            "text": " World"
        }
    });

    let block_stop = serde_json::json!({
        "type": "content_block_stop",
        "index": 0
    });

    let message_delta = serde_json::json!({
        "type": "message_delta",
        "delta": { "stop_reason": "end_turn" },
        "usage": { "output_tokens": 5 }
    });

    let message_stop = serde_json::json!({
        "type": "message_stop"
    });

    let body = build_responses_body(vec![
        message_start,
        block_start,
        delta1,
        delta2,
        block_stop,
        message_delta,
        message_stop,
    ]);

    let transport = FixtureSseTransport::new(body);
    let client =
        codex_api::AnthropicMessagesClient::new(transport, provider("minimax"), Arc::new(NoAuth));

    let mut stream = client
        .stream(
            serde_json::json!({"echo": true}),
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
        )
        .await?;

    let mut events = Vec::new();
    while let Some(ev) = stream.next().await {
        events.push(ev?);
    }

    let events: Vec<ResponseEvent> = events
        .into_iter()
        .filter(|ev| !matches!(ev, ResponseEvent::RateLimits(_)))
        .collect();

    assert_eq!(events.len(), 6);

    assert!(matches!(events[0], ResponseEvent::Created));
    if let ResponseEvent::ServerModel(m) = &events[1] {
        assert_eq!(m, "MiniMax-M2");
    } else {
        panic!("expected ServerModel");
    }
    if let ResponseEvent::OutputTextDelta(d) = &events[2] {
        assert_eq!(d, "Hello");
    } else {
        panic!("expected OutputTextDelta");
    }
    if let ResponseEvent::OutputTextDelta(d) = &events[3] {
        assert_eq!(d, " World");
    } else {
        panic!("expected OutputTextDelta");
    }
    if let ResponseEvent::OutputItemDone(ResponseItem::Message { role, content, .. }) = &events[4] {
        assert_eq!(role, "assistant");
        assert_eq!(content.len(), 1);
        if let codex_protocol::models::ContentItem::OutputText { text } = &content[0] {
            assert_eq!(text, "Hello World");
        } else {
            panic!("expected OutputText content item");
        }
    } else {
        panic!("expected OutputItemDone");
    }
    if let ResponseEvent::Completed {
        response_id,
        token_usage,
        end_turn,
    } = &events[5]
    {
        assert_eq!(response_id, "msg1");
        assert!(end_turn.unwrap_or(false));
        let usage = token_usage.as_ref().unwrap();
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.total_tokens, 15);
    } else {
        panic!("expected Completed");
    }

    Ok(())
}

#[tokio::test]
async fn anthropic_stream_parses_tool_use_end_to_end() -> Result<()> {
    let message_start = serde_json::json!({
        "type": "message_start",
        "message": {
            "id": "msg2",
            "model": "MiniMax-M2",
            "usage": { "input_tokens": 15 }
        }
    });

    let block_start = serde_json::json!({
        "type": "content_block_start",
        "index": 0,
        "content_block": {
            "type": "tool_use",
            "id": "call_123",
            "name": "get_weather",
            "input": {}
        }
    });

    let delta1 = serde_json::json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": {
            "type": "input_json_delta",
            "partial_json": "{\"location\":"
        }
    });

    let delta2 = serde_json::json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": {
            "type": "input_json_delta",
            "partial_json": "\"Paris\"}"
        }
    });

    let block_stop = serde_json::json!({
        "type": "content_block_stop",
        "index": 0
    });

    let message_delta = serde_json::json!({
        "type": "message_delta",
        "delta": { "stop_reason": "tool_use" },
        "usage": { "output_tokens": 8 }
    });

    let message_stop = serde_json::json!({
        "type": "message_stop"
    });

    let body = build_responses_body(vec![
        message_start,
        block_start,
        delta1,
        delta2,
        block_stop,
        message_delta,
        message_stop,
    ]);

    let transport = FixtureSseTransport::new(body);
    let client =
        codex_api::AnthropicMessagesClient::new(transport, provider("minimax"), Arc::new(NoAuth));

    let mut stream = client
        .stream(
            serde_json::json!({"echo": true}),
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
        )
        .await?;

    let mut events = Vec::new();
    while let Some(ev) = stream.next().await {
        events.push(ev?);
    }

    let events: Vec<ResponseEvent> = events
        .into_iter()
        .filter(|ev| !matches!(ev, ResponseEvent::RateLimits(_)))
        .collect();

    assert_eq!(events.len(), 7);

    assert!(matches!(events[0], ResponseEvent::Created));
    assert!(matches!(events[1], ResponseEvent::ServerModel(_)));
    if let ResponseEvent::OutputItemAdded(ResponseItem::FunctionCall { call_id, name, .. }) =
        &events[2]
    {
        assert_eq!(call_id, "call_123");
        assert_eq!(name, "get_weather");
    } else {
        panic!("expected OutputItemAdded FunctionCall");
    }
    if let ResponseEvent::ToolCallInputDelta { item_id, delta, .. } = &events[3] {
        assert_eq!(item_id, "call_123");
        assert_eq!(delta, "{\"location\":");
    } else {
        panic!("expected ToolCallInputDelta");
    }
    if let ResponseEvent::ToolCallInputDelta { item_id, delta, .. } = &events[4] {
        assert_eq!(item_id, "call_123");
        assert_eq!(delta, "\"Paris\"}");
    } else {
        panic!("expected ToolCallInputDelta");
    }
    if let ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
        call_id,
        name,
        arguments,
        ..
    }) = &events[5]
    {
        assert_eq!(call_id, "call_123");
        assert_eq!(name, "get_weather");
        assert_eq!(arguments, "{\"location\":\"Paris\"}");
    } else {
        panic!("expected OutputItemDone FunctionCall");
    }
    if let ResponseEvent::Completed { response_id, .. } = &events[6] {
        assert_eq!(response_id, "msg2");
    } else {
        panic!("expected Completed");
    }

    Ok(())
}

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

struct CallCountingTransport {
    status: StatusCode,
    calls: Arc<AtomicUsize>,
}

impl CallCountingTransport {
    fn new(status: StatusCode) -> Self {
        Self {
            status,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl HttpTransport for CallCountingTransport {
    async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(TransportError::Http {
            status: self.status,
            url: Some("https://example.com/v1/messages".to_string()),
            headers: None,
            body: Some("Error payload".to_string()),
        })
    }

    async fn stream(&self, _req: Request) -> Result<StreamResponse, TransportError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(TransportError::Http {
            status: self.status,
            url: Some("https://example.com/v1/messages".to_string()),
            headers: None,
            body: Some("Error payload".to_string()),
        })
    }
}

#[tokio::test]
async fn anthropic_error_classification_unauthorized_aborts_immediately() -> Result<()> {
    let transport = CallCountingTransport::new(StatusCode::UNAUTHORIZED);
    let calls = transport.calls.clone();

    let mut provider_conf = provider("minimax");
    provider_conf.retry = codex_api::RetryConfig {
        max_attempts: 3,
        base_delay: Duration::from_millis(1),
        retry_429: true,
        retry_5xx: true,
        retry_transport: true,
    };

    let client =
        codex_api::AnthropicMessagesClient::new(transport, provider_conf, Arc::new(NoAuth));

    let stream_result = client
        .stream(
            serde_json::json!({"echo": true}),
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
        )
        .await;

    assert!(stream_result.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1); // unauthorized must not retry

    Ok(())
}

#[tokio::test]
async fn anthropic_error_classification_too_many_requests_retries() -> Result<()> {
    let transport = CallCountingTransport::new(StatusCode::TOO_MANY_REQUESTS);
    let calls = transport.calls.clone();

    let mut provider_conf = provider("minimax");
    provider_conf.retry = codex_api::RetryConfig {
        max_attempts: 3,
        base_delay: Duration::from_millis(1),
        retry_429: true,
        retry_5xx: true,
        retry_transport: true,
    };

    let client =
        codex_api::AnthropicMessagesClient::new(transport, provider_conf, Arc::new(NoAuth));

    let stream_result = client
        .stream(
            serde_json::json!({"echo": true}),
            HeaderMap::new(),
            Compression::None,
            /*turn_state*/ None,
        )
        .await;

    assert!(stream_result.is_err());
    // max_attempts is the retry count, so the initial call plus 3 retries = 4 calls.
    assert_eq!(calls.load(Ordering::SeqCst), 4); // too many requests should retry

    Ok(())
}
