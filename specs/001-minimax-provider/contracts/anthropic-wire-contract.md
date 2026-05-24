# Contract: Anthropic Messages Wire Adapter (internal)

Defines the translation contract between Codex's internal turn model
(`ResponseItem` + Responses-style streaming) and MiniMax's Anthropic-compatible
Messages API. Implemented in `codex-api` under `requests/anthropic.rs`,
`sse/anthropic.rs`, `endpoint/anthropic/`. This is an internal contract; it has
no JSON Schema exposed to users.

## Endpoint

```
POST {base_url}/messages
Headers:
  x-api-key: {MINIMAX_API_KEY}            # (or Authorization: Bearer, per MiniMax)
  anthropic-version: <version>            # if required by MiniMax
  content-type: application/json
Body: Anthropic Messages request (below)
Stream: text/event-stream (SSE)
```

## Request translation (outbound)

| Internal (`ResponseItem` / turn) | Anthropic Messages field |
|----------------------------------|--------------------------|
| System instructions | top-level `system` (string/blocks) |
| User/assistant messages | `messages[]` with role + content blocks |
| Tool/function definitions | `tools[]` (name, description, input_schema) |
| Tool choice setting | `tool_choice` |
| Function call item | assistant `tool_use` content block |
| Function call output | user `tool_result` content block |
| Temperature | `temperature` |
| Streaming flag | `stream: true` |
| Model id | `model` |

Rules:
- MUST NOT include image/document content (gated upstream, FR-012).
- Reasoning request settings map to Anthropic thinking config where applicable (FR-004).

## Response translation (inbound, SSE)

| Anthropic SSE event | Internal stream event |
|---------------------|-----------------------|
| `message_start` | turn/response start |
| `content_block_start` (text) | begin text block |
| `content_block_delta` (text_delta) | text delta |
| `content_block_start/delta` (thinking) | reasoning delta |
| `content_block_start` (tool_use) + input deltas | function-call item assembly |
| `content_block_stop` | end of block |
| `message_delta` (usage, stop_reason) | usage + completion metadata |
| `message_stop` | turn complete |
| `error` | error classification (below) |

Rules:
- The translator MUST assemble streamed `tool_use` input JSON before emitting a
  complete internal function-call item.
- Token usage MUST be propagated where present (`message_delta.usage`).

## Error classification contract

| Upstream | Internal category | Behavior |
|----------|-------------------|----------|
| 429 (rate limit) | recoverable | retry per `RetryConfig` (FR-013) |
| 5xx | recoverable | retry per `RetryConfig` (FR-013) |
| 401 / 403 | authentication | distinct auth error (FR-009) |
| missing `MINIMAX_API_KEY` | missing-key (pre-flight) | actionable error (FR-009, SC-005) |
| malformed SSE | transport error | surfaced consistent with other providers |

## Capability contract

For the MiniMax provider, `ProviderCapabilities` MUST disable Responses-only
features not supported over the Anthropic wire (image generation; websockets;
remote compaction). Multi-turn, system prompt, streaming, temperature, tools,
tool choice, and reasoning content are supported (FR-004).

## Test contract

- A mock Anthropic SSE server (analogous to `core_test_support::responses`)
  drives end-to-end wire tests: text streaming, a tool-use round trip
  (US1 scenario 2), auth failure, and rate-limit retry.
- Exhaustive `match WireApi` coverage verified by compilation (no wildcard arm).
