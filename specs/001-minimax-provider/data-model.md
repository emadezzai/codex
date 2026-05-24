# Phase 1 Data Model: MiniMax Provider Integration

Entities are expressed in terms of existing Codex types where they already
exist, with the new fields/variants this feature introduces called out.

## 1. WireApi (extended enum)

Existing enum in `codex-rs/model-provider-info/src/lib.rs`.

- **Current**: `Responses` (only variant; `chat` removed).
- **New variant**: `Anthropic` — selects the Anthropic Messages wire adapter.

Rules:
- `Deserialize` accepts `"anthropic"`; `Display`/`Serialize` emit `"anthropic"`.
- Every `match` on `WireApi` MUST be updated (exhaustive, no wildcard — Principle III).
- Default remains `Responses`.

## 2. Provider (MiniMax) — `ModelProviderInfo`

Existing struct `ModelProviderInfo`. The MiniMax built-in is constructed by a
new `create_minimax_provider(base_url: Option<String>)` and registered in
`built_in_model_providers` under id `minimax`.

| Field | Value for MiniMax |
|-------|-------------------|
| `name` | `"MiniMax"` |
| `base_url` | `Some("https://api.minimax.io/anthropic")` (override path finalized in impl) |
| `env_key` | `Some("MINIMAX_API_KEY")` |
| `env_key_instructions` | Message naming `MINIMAX_API_KEY` and how to set it (FR-009) |
| `wire_api` | `WireApi::Anthropic` |
| `requires_openai_auth` | `false` |
| `supports_websockets` | `false` |
| `http_headers` | Anthropic version header if required by MiniMax |

Validation: existing `validate()` applies. The provider is overridable via
`merge_configured_model_providers` for `base_url` (international ↔ mainland).

Constants to add: `MINIMAX_PROVIDER_NAME`, `MINIMAX_PROVIDER_ID = "minimax"`,
`MINIMAX_DEFAULT_BASE_URL`.

## 3. Model (MiniMax catalog entry) — `ModelInfo` + tier

Bundled in `models-manager/models.json` (or sibling `minimax_models.json`).
Reuses `ModelInfo`; adds tier semantics.

| Attribute | Source / Value |
|-----------|----------------|
| `slug` / model id | e.g. `MiniMax-M2`, `MiniMax-M2-highspeed` |
| `display_name` | Human label |
| **`tier`** (NEW) | Explicit `normal` \| `high-speed` (see entity 4) |
| lifecycle / `visibility` | `list` for current; legacy models marked so they stay selectable but distinguishable (FR-011) |
| `context_window` / `max_context_window` | Per MiniMax model |
| `input_modalities` | **text only** (no `image`/document) — drives FR-012 gate |
| `supported_in_api` | `true` (allows unknown/forward-compat ids to still run, FR-010) |
| tool/streaming/reasoning support | Enabled (FR-004) |

Validation rules:
- Every bundled MiniMax model MUST have an explicit `tier` (SC-003: none uncategorized).
- `input_modalities` MUST NOT include `image`/document for MiniMax models.

## 4. Tier (classification)

New explicit attribute on MiniMax catalog models; two values.

```
Tier = Normal | HighSpeed
```

- **Stored**: explicit `tier` field on each bundled catalog entry (authoritative).
- **Derived (fallback)**: for an id not in the catalog, `tier = HighSpeed` iff the
  id contains the `-highspeed` marker, else `Normal` (FR-006, FR-010).
- **Exposure**: surfaced in model listing/selection UI as a label
  ("normal"/"high-speed") (FR-006, SC-003).

Decision (from research R4): introduce a dedicated, explicit `tier` field rather
than overloading `service_tiers`/`additional_speed_tiers`, to satisfy FR-006
unambiguously. Code provides the `-highspeed` fallback for uncataloged ids.

## 5. Credential

No new type. Reuses `ModelProviderInfo.env_key` resolution (`api_key()`),
returning the existing `EnvVar` error when missing. Invalid keys surface as an
upstream 401 mapped to a distinct authentication error (entity 6).

## 6. Wire request/response (Anthropic adapter, internal)

Not persisted; transient translation types in `codex-api`.

- **Request**: built from internal `ResponseItem[]` →
  `{ model, system, messages[], tools[], tool_choice, temperature, stream }`
  (Anthropic Messages shape).
- **Streaming response**: Anthropic SSE events
  (`message_start`, `content_block_start/delta/stop`, `tool_use`,
  `message_delta`, `message_stop`, `error`) → internal incremental stream events
  (text deltas, tool calls, reasoning, completion, usage).
- **Errors**: classified into `recoverable` (429/5xx), `authentication`
  (401/403), `missing_key` (pre-flight) — see research R6 / FR-009, FR-013.

## State transitions

- **Model selection persistence** (FR-008): selected MiniMax model id persists
  across sessions via existing model-selection config — no new storage.
- **Turn gating** (FR-012): `Build → (modality check) → Send`. If an
  unsupported input is present, transition to `Blocked` with a typed error
  naming the input type, before any network call.

## Relationships

```
Provider(minimax) 1───* Model(catalog entry)
Model ───1 Tier (explicit field, or derived for uncataloged ids)
Provider ───1 WireApi::Anthropic ───uses─→ Anthropic Messages adapter
Provider ───1 Credential (MINIMAX_API_KEY via env_key)
```
