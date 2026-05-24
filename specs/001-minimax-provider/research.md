# Phase 0 Research: MiniMax Provider Integration

This document resolves the unknowns surfaced while filling Technical Context.
The decisive finding is that MiniMax cannot be reached over Codex's existing
wire layer, so the central research item is the wire-protocol strategy.

## R1. Wire-protocol gap (CRITICAL)

**Question**: Codex's `codex-api` speaks only the OpenAI Responses API
(`WireApi::Responses`; the `chat` variant was deliberately removed —
`model-provider-info/src/lib.rs:45-79`). MiniMax exposes an *Anthropic-compatible
Messages* interface, not a Responses endpoint. How do we connect them?

**Options considered**:

1. **Add `WireApi::Anthropic` + an in-process Anthropic Messages adapter in
   `codex-api`.** Translate the internal `ResponseItem` turn model to an
   Anthropic `/v1/messages` request, and translate Anthropic's streaming SSE
   (`message_start`, `content_block_delta`, `tool_use`, `message_delta`, etc.)
   back into Codex's internal stream events.
2. **Re-introduce a Chat-Completions wire and use MiniMax's OpenAI-compatible
   endpoint.** Rejected: the `chat` wire was explicitly removed and resurrecting
   it contradicts the project direction (CHAT_WIRE_API_REMOVED_ERROR); also the
   spec mandates the Anthropic-compatible interface.
3. **Local translation proxy** (extend `responses-api-proxy`) that accepts
   Responses requests and forwards Anthropic. Rejected as the primary path:
   adds a process/transport hop, complicates auth/credential handling and
   error surfacing, and pushes wire logic outside the typed Rust pipeline. May
   remain a fallback experiment but not the shipped design.

**Decision**: Option 1 — a first-class `WireApi::Anthropic` variant with an
Anthropic Messages adapter implemented in `codex-api`
(`requests/anthropic.rs`, `sse/anthropic.rs`, `endpoint/anthropic/`), mirroring
the existing `responses` module layout.

**Rationale**: Matches the spec's mandated interface, keeps the wire logic typed
and in-process, reuses existing retry/timeout/header plumbing on
`codex_api::Provider`, and isolates the new code from `codex-core` per
Principle II. Adding the enum variant also forces (via exhaustive `match`,
Principle III) every wire-dispatch site to consciously handle Anthropic.

**Alternatives considered**: see options 2 and 3 above.

**Key translation concerns to handle in design**:
- System prompt: Responses embeds instructions in `input`; Anthropic uses a
  top-level `system` field.
- Tools: map Codex tool/function definitions to Anthropic `tools` + `tool_choice`;
  map `tool_use`/`tool_result` blocks to internal `FunctionCall`/tool output items.
- Reasoning: map Anthropic "thinking"/reasoning content to internal reasoning items.
- Streaming: Anthropic SSE event sequence differs from Responses; the translator
  must assemble content blocks and emit Codex's incremental events.
- No support for Responses-only features (remote compaction, websockets,
  image generation) — `ProviderCapabilities` for MiniMax must disable these.

## R2. Endpoint, region, and base-URL override

**Decision**: Default `base_url = https://api.minimax.io/anthropic` (international),
overridable via `model_providers.minimax.base_url` in `config.toml` (e.g.
`https://api.minimaxi.com/...` for mainland China). The exact path suffix
(`/anthropic` vs `/v1/messages`) is finalized against MiniMax docs during
implementation; the adapter appends the `messages` path via
`Provider::url_for_path`.

**Rationale**: Matches clarification (international default, overridable) and the
existing `base_url: Option<String>` config mechanism. No new config primitive
needed.

**Alternatives**: Hardcoding a single region (rejected — clarified overridable);
separate region enum (rejected — base_url override already covers it).

## R3. Credential / auth flow

**Decision**: Reuse `ModelProviderInfo.env_key` with `MINIMAX_API_KEY` and the
existing login/auth flow; no MiniMax-specific credential path. The Anthropic
wire authenticates via the standard Anthropic header (`x-api-key`, or
`Authorization: Bearer` if MiniMax accepts it — confirmed in implementation);
the adapter injects the resolved key into request headers.

**Rationale**: Matches clarification (reuse existing mechanism + standard env
var). `api_key()` already returns a clear `EnvVar` error when missing
(supports FR-009 "missing key"); an *invalid* key surfaces as an upstream 401,
which the adapter maps to a distinct authentication error (FR-009 "auth failed").

**Alternatives**: Bespoke credential store (rejected — clarification).

## R4. Model catalog scope and tier classification

**Decision**: Bundle only the M-series models served over the Anthropic-compatible
endpoint (e.g. `MiniMax-M2` and its `-highspeed` variant). Each bundled entry
carries an explicit `tier` field (`normal` | `high-speed`). For uncataloged
(forward-compatible) ids, derive tier by detecting the `-highspeed` marker
(absent ⇒ normal). Legacy models keep a lifecycle/visibility marker so they
remain selectable but distinguishable (FR-011).

**Rationale**: Matches clarifications on catalog scope and tier determination.
The bundled catalog mechanism (`models.json` via `include_str!`,
`bundled_models_response()`) already exists; we extend it (or add a sibling
catalog file) with MiniMax entries and a tier attribute.

**Open implementation detail**: whether `tier` reuses/extends the existing
`additional_speed_tiers`/`service_tiers` fields on `ModelInfo` or is a new
explicit attribute. Decided in data-model.md → prefer a dedicated, explicit
`tier` to satisfy FR-006 unambiguously, with `-highspeed` fallback in code.

**Alternatives**: Tier inferred purely from naming (rejected — clarification
requires an explicit stored field, naming is fallback only).

## R5. Unsupported input handling (images / documents)

**Decision**: Mark MiniMax models' `input_modalities` as text-only (plus
tool/reasoning content). Before constructing the request, gate the turn: if it
contains an image or document input, abort with a typed error naming the
unsupported input type. Do not silently drop or send partial content.

**Rationale**: Matches clarification (block + clear message). `ModelInfo`
already has `input_modalities`; the gate lives at request-build/dispatch time
in `core/src/client.rs` (or the adapter boundary) so it is provider-agnostic
and testable.

**Alternatives**: Silent drop / partial send (explicitly rejected by spec);
relying on upstream rejection (rejected — opaque, late, wastes a round-trip).

## R6. Error surfacing (rate limits, 5xx, auth)

**Decision**: Map Anthropic-style error responses to Codex's existing
recoverable/non-recoverable error categories: 429/5xx → recoverable (retryable
per `RetryConfig`, consistent with other providers); 401/403 → distinct
authentication error; missing env key → existing `EnvVar` error.

**Rationale**: FR-013 requires parity with existing providers; the
`RetryConfig` on `codex_api::Provider` already governs retry behavior. The
adapter only needs to classify Anthropic error bodies into these buckets.

**Alternatives**: New MiniMax-specific error taxonomy (rejected — parity
required).

## Summary of resolutions

| Item | Decision |
|------|----------|
| Wire protocol | New `WireApi::Anthropic` + Anthropic Messages adapter in `codex-api` |
| Endpoint | Default `api.minimax.io`, base_url overridable via config |
| Credential | Reuse `env_key` (`MINIMAX_API_KEY`) + existing auth flow |
| Catalog | Bundle M-series; explicit `tier` field; `-highspeed` fallback |
| Unsupported inputs | text-only modalities; pre-send block with typed error |
| Errors | Map to existing recoverable/auth/missing-key categories |

All NEEDS CLARIFICATION items are resolved. No blocking unknowns remain for
Phase 1 design.
