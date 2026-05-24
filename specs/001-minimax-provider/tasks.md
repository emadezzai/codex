---
description: "Task list for MiniMax Provider Integration"
---

# Tasks: MiniMax Provider Integration

**Input**: Design documents from `/specs/001-minimax-provider/`
**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/)

**Tests**: Included. The constitution (Principle V: Test Discipline) makes wire-protocol and user-visible-UI tests mandatory for this feature, so targeted test tasks are present in each phase.

**Organization**: Tasks are grouped by user story. The Anthropic wire adapter is a hard blocking prerequisite (Phase 2) because MiniMax cannot be reached over Codex's existing Responses-only wire — see [research.md](./research.md) R1.

> **For the implementing model**: Update the **Progress Tracker** table below as you go (set Status to `done`/`blocked`). Each task lists exact files and the symbols to add/modify. Run `just fmt` after Rust edits and `just clippy` before marking a phase done (Constitution Workflow). Do not add provider-specific helpers to `codex-core` (Principle II) — wire logic lives in `codex-api`.

---

## Progress Tracker

| Phase | Tasks | Status | Notes |
|-------|-------|--------|-------|
| 1. Setup | T001–T003 | todo | constants, scaffolding |
| 2. Foundational (Anthropic wire) ⚠️ BLOCKING | T004–T015 | todo | the bulk of the work |
| 3. US1 — Use MiniMax (P1) 🎯 MVP | T016–T024 | todo | provider usable end-to-end |
| 4. US2 — Tier selection (P2) | T025–T032 | todo | normal/high-speed |
| 5. US3 — Discover/onboard (P3) | T033–T038 | todo | full catalog + guidance |
| 6. Polish | T039–T045 | todo | schema, bazel, docs, validation |

**Per-task status** (mark `x` in the checkbox when done):

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Constants and identifiers shared across all later phases.

- [ ] T001 Add MiniMax identity constants to [codex-rs/model-provider-info/src/lib.rs](../../codex-rs/model-provider-info/src/lib.rs): `MINIMAX_PROVIDER_NAME = "MiniMax"`, `pub const MINIMAX_PROVIDER_ID = "minimax"`, `pub const MINIMAX_DEFAULT_BASE_URL = "https://api.minimax.io/anthropic"`, `MINIMAX_API_KEY_ENV = "MINIMAX_API_KEY"` (place alongside the existing `OPENAI_PROVIDER_ID`/`AMAZON_BEDROCK_*` constants near lines 35–47).
- [ ] T002 [P] Confirm the exact MiniMax base-URL path suffix and auth header (`x-api-key` vs `Authorization: Bearer`, `anthropic-version` header) against MiniMax docs; record the confirmed values as a comment in [contracts/anthropic-wire-contract.md](./contracts/anthropic-wire-contract.md). (Research item left open in plan.md.)
- [ ] T003 [P] Create the empty module skeleton for the wire adapter so later tasks compile incrementally: add `requests/anthropic.rs`, `sse/anthropic.rs`, and `endpoint/anthropic/mod.rs` + `endpoint/anthropic/messages.rs` under [codex-rs/codex-api/src/](../../codex-rs/codex-api/src/), each with a `//!` doc comment and `mod` declarations wired into `requests/mod.rs`, `sse/mod.rs`, and `endpoint/mod.rs`. Keep each file < 500 LOC (Principle I).

**Checkpoint**: Constants exist and empty adapter modules compile.

---

## Phase 2: Foundational (Blocking Prerequisites — Anthropic Messages wire)

**Purpose**: The Anthropic Messages wire adapter and `WireApi::Anthropic` variant. **No user story can function without this** (MiniMax has no Responses endpoint).

**⚠️ CRITICAL**: Complete this entire phase before starting Phase 3.

- [ ] T004 Add the `Anthropic` variant to the `WireApi` enum in [codex-rs/model-provider-info/src/lib.rs:50-79](../../codex-rs/model-provider-info/src/lib.rs#L50-L79): update `Display` to emit `"anthropic"`, and `Deserialize` to accept `"anthropic"` (keep the `"chat"` removed-error arm). Add it to the `unknown_variant` allowed list.
- [ ] T005 Fix every exhaustive `match` on `WireApi` across the workspace so it handles `Anthropic` (no wildcard arms — Principle III). Find sites with `grep -rn "WireApi::" codex-rs --include=*.rs`. Primary sites: [codex-rs/core/src/client.rs](../../codex-rs/core/src/client.rs) and [codex-rs/codex-api/src/](../../codex-rs/codex-api/src/) request/endpoint dispatch.
- [ ] T006 [P] Define the Anthropic Messages request types and the outbound translation in [codex-rs/codex-api/src/requests/anthropic.rs](../../codex-rs/codex-api/src/requests/anthropic.rs): build `{model, system, messages[], tools[], tool_choice, temperature, stream}` from internal `&[ResponseItem]`. Map system instructions → top-level `system`; user/assistant messages → `messages[]` content blocks; tool/function definitions → `tools[]`; `FunctionCall` → `tool_use`; function-call output → `tool_result`. Follow the table in [contracts/anthropic-wire-contract.md](./contracts/anthropic-wire-contract.md).
- [ ] T007 [P] Define the Anthropic SSE event types and the inbound translation in [codex-rs/codex-api/src/sse/anthropic.rs](../../codex-rs/codex-api/src/sse/anthropic.rs): parse `message_start`, `content_block_start/delta/stop` (text + thinking + tool_use), `message_delta` (usage, stop_reason), `message_stop`, `error`; emit Codex's internal incremental stream events. Assemble streamed `tool_use` input JSON before emitting a complete internal function-call item. Mirror the structure of [codex-rs/codex-api/src/sse/responses.rs](../../codex-rs/codex-api/src/sse/responses.rs).
- [ ] T008 Wire the messages endpoint in [codex-rs/codex-api/src/endpoint/anthropic/messages.rs](../../codex-rs/codex-api/src/endpoint/anthropic/messages.rs): POST to `Provider::url_for_path("messages")`, attach auth + version headers, send the T006 payload, stream through the T007 translator. Reuse `Provider.retry`/`stream_idle_timeout`. Mirror [codex-rs/codex-api/src/endpoint/responses.rs](../../codex-rs/codex-api/src/endpoint/responses.rs).
- [ ] T009 Implement error classification in the endpoint (T008): 429/5xx → recoverable (retry per `RetryConfig`); 401/403 → distinct authentication error; malformed SSE → transport error. Map to Codex's existing error categories used by the responses endpoint (FR-009, FR-013). See [contracts/anthropic-wire-contract.md](./contracts/anthropic-wire-contract.md) error table.
- [ ] T010 Add wire dispatch by `WireApi` so a `WireApi::Anthropic` provider routes to the T008 endpoint instead of the responses endpoint. Locate the responses dispatch in [codex-rs/codex-api/src/endpoint/mod.rs](../../codex-rs/codex-api/src/endpoint/mod.rs) / [codex-rs/core/src/client.rs](../../codex-rs/core/src/client.rs) and branch on `provider.wire_api`.
- [ ] T011 In [codex-rs/model-provider/src/provider.rs](../../codex-rs/model-provider/src/provider.rs), set `ProviderCapabilities` for an Anthropic-wire provider to disable Responses-only features: `image_generation = false`, websockets off; keep tools/web_search per support. Ensure `supports_remote_compaction()` stays false for MiniMax.
- [ ] T012 [P] Add an Anthropic mock SSE server test helper (analogous to `core_test_support::responses`) for use by integration tests, in [codex-rs/codex-api/tests/](../../codex-rs/codex-api/tests/) or a shared test-support module. It should serve a scripted Anthropic SSE event sequence.
- [ ] T013 [P] Wire test: text streaming round trip through the adapter in [codex-rs/codex-api/tests/](../../codex-rs/codex-api/tests/) — send a prompt, assert assembled text via the mock server (T012). Use `pretty_assertions::assert_eq`.
- [ ] T014 [P] Wire test: a `tool_use` round trip (model emits a tool call → tool_result fed back) in [codex-rs/codex-api/tests/](../../codex-rs/codex-api/tests/) (covers US1 acceptance scenario 2).
- [ ] T015 [P] Wire test: error classification — assert 401 → auth error and 429 → recoverable/retry, in [codex-rs/codex-api/tests/](../../codex-rs/codex-api/tests/).

**Checkpoint**: The Anthropic wire adapter streams text and tool calls against a mock server and classifies errors. `just test -p codex-api` passes.

---

## Phase 3: User Story 1 — Use MiniMax as a model provider (Priority: P1) 🎯 MVP

**Goal**: A user with a `MINIMAX_API_KEY` can select MiniMax, send a prompt, and get a streamed response with working tool calls.

**Independent Test**: Configure MiniMax with a valid key, select a MiniMax model, send a prompt, confirm a coherent response and a functioning tool call (quickstart steps 1–3).

- [ ] T016 [US1] Add `create_minimax_provider(base_url: Option<String>) -> ModelProviderInfo` in [codex-rs/model-provider-info/src/lib.rs](../../codex-rs/model-provider-info/src/lib.rs): `name = MINIMAX_PROVIDER_NAME`, `base_url = base_url.or(MINIMAX_DEFAULT_BASE_URL)`, `env_key = Some("MINIMAX_API_KEY")`, `wire_api = WireApi::Anthropic`, `requires_openai_auth = false`, `supports_websockets = false`. Mirror `create_amazon_bedrock_provider`.
- [ ] T017 [US1] Register MiniMax in `built_in_model_providers` (same file, lines ~409–435) under `MINIMAX_PROVIDER_ID` so it ships built-in (FR-001).
- [ ] T018 [US1] Allow `base_url` override for MiniMax via `merge_configured_model_providers` (same file, lines ~442–473) so users can switch to the mainland endpoint (FR-003). Decide whether MiniMax follows the general "configured providers extend" path (it should — only Bedrock is special-cased), so likely no special-casing is needed; add a test confirming an override of `model_providers.minimax.base_url` wins.
- [ ] T019 [US1] Add `env_key_instructions` to the MiniMax provider (T016) naming `MINIMAX_API_KEY` and how to set it, so the missing-key path (`api_key()` → `EnvVar` error) is actionable (FR-009 missing-key, SC-005).
- [ ] T020 [US1] Implement the pre-send unsupported-input gate (FR-012): before building the Anthropic request, if the turn contains image/document content and the active provider is Anthropic-wire (or the model's `input_modalities` excludes them), abort the turn with a typed error naming the input type. Implement at the request-build boundary in [codex-rs/core/src/client.rs](../../codex-rs/core/src/client.rs) (or the adapter entry) — keep it provider-agnostic via `input_modalities`.
- [ ] T021 [US1] Map the upstream 401 (from T009) to a user-facing authentication error distinct from the missing-key error, surfaced through the normal error display path (FR-009).
- [ ] T022 [P] [US1] Integration test: end-to-end turn against the mock Anthropic server (T012) with provider `minimax` selected — assert a response is returned (US1 scenario 1, SC-001). Place in [codex-rs/core/tests/](../../codex-rs/core/) or `codex-api` tests.
- [ ] T023 [P] [US1] Integration test: missing `MINIMAX_API_KEY` produces an error naming the credential (US1 scenario 3, SC-005). Do **not** mutate process env in the test (Principle V) — inject the provider/env via the existing test harness flags.
- [ ] T024 [P] [US1] Integration test: unsupported image input blocks the turn with a message naming the input type (FR-012 edge case).

**Checkpoint**: MVP complete — MiniMax is selectable and usable end-to-end with tool calls and correct error messages.

---

## Phase 4: User Story 2 — Choose normal vs high-speed tiers (Priority: P2)

**Goal**: Each MiniMax model is labeled `normal` or `high-speed`; selecting a high-speed variant routes to that id; selection persists.

**Independent Test**: List MiniMax models, confirm each shows a tier label; select a high-speed variant and confirm requests use that id (quickstart step 4).

- [ ] T025 [US2] Add an explicit `tier` representation for catalog models. Add a `MiniMaxTier { Normal, HighSpeed }`-style notion and a serde field on the catalog entry. Decide placement: extend `ModelInfo` in `codex-protocol::openai_models` with an optional `tier` field, OR carry it in a MiniMax-specific catalog struct in [codex-rs/models-manager/src/model_info.rs](../../codex-rs/models-manager/src/model_info.rs). Prefer a dedicated explicit field per [data-model.md](./data-model.md) §4. Run `just write-config-schema` if `ConfigToml`/protocol types change.
- [ ] T026 [US2] Implement the `-highspeed` fallback derivation for uncataloged ids: a helper `tier_for_model_id(id) -> Tier` returning `HighSpeed` iff the id contains `-highspeed`, else `Normal` (FR-006, FR-010). Place in [codex-rs/models-manager/src/model_info.rs](../../codex-rs/models-manager/src/model_info.rs).
- [ ] T027 [P] [US2] Add at least one normal + one high-speed MiniMax entry to the bundled catalog [codex-rs/models-manager/models.json](../../codex-rs/models-manager/models.json) (e.g. `MiniMax-M2`, `MiniMax-M2-highspeed`), each with explicit `tier`, text-only `input_modalities`, context window, and tool/streaming support.
- [ ] T028 [US2] Expose the tier label when listing/selecting models. Surface `tier` from the catalog (or T026 fallback) in the model-listing path so it reaches the picker (FR-006, SC-003).
- [ ] T029 [US2] Display tier (and keep it readable) in the TUI model picker in [codex-rs/tui/src/](../../codex-rs/tui/src/): use `Stylize` helpers, no hardcoded white, `textwrap`/`wrapping.rs` for any wrapped text (Principle IV).
- [ ] T030 [US2] Confirm selected MiniMax model persists across sessions via the existing model-selection config (FR-008) — add/verify no regression; no new storage.
- [ ] T031 [P] [US2] Unit test: `tier_for_model_id` returns HighSpeed for `*-highspeed`, Normal otherwise (FR-006/FR-010).
- [ ] T032 [P] [US2] `insta` snapshot test in [codex-rs/tui/](../../codex-rs/tui/) for the model picker showing tier labels (Principle V, SC-003).

**Checkpoint**: Tiers are visible and selectable; switching to a high-speed id routes correctly.

---

## Phase 5: User Story 3 — Discover and configure without external lookup (Priority: P3)

**Goal**: The full supported MiniMax catalog ships built-in with onboarding guidance; legacy models are marked but selectable.

**Independent Test**: On a fresh install, MiniMax appears as a built-in provider with its full model set; selecting it without a key surfaces instructions naming the exact credential.

- [ ] T033 [P] [US3] Complete the bundled MiniMax catalog in [codex-rs/models-manager/models.json](../../codex-rs/models-manager/models.json) — add all M-series models served over the Anthropic-compatible endpoint and their high-speed variants (FR-005, SC-002). Exclude legacy `abab`/Text-only chat models (out of scope).
- [ ] T034 [US3] Mark legacy-but-available models with a lifecycle/visibility indicator so they remain selectable yet clearly distinguishable (FR-011), per [data-model.md](./data-model.md) §3.
- [ ] T035 [US3] Surface the legacy marker in the model picker UI in [codex-rs/tui/src/](../../codex-rs/tui/src/) (Principle IV styling rules).
- [ ] T036 [US3] Verify forward-compat: an uncataloged MiniMax id is accepted and runs (FR-010, SC-006), with tier derived via T026 — add/confirm a test.
- [ ] T037 [P] [US3] Test: a fresh-config run lists MiniMax as a built-in provider with the bundled models present (US3 scenario 1, SC-002).
- [ ] T038 [P] [US3] Test: selecting MiniMax with no credential surfaces instructions naming `MINIMAX_API_KEY` (US3 scenario 2; complements T023).

**Checkpoint**: All user stories independently functional.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Schema, build metadata, docs, and final validation.

- [ ] T039 Run `just write-config-schema` to regenerate `config.schema.json` for the new `wire_api = "anthropic"` value and any provider/tier config additions (Constitution Workflow #5).
- [ ] T040 If a new `include_str!`/`include_bytes!` catalog file was added (instead of editing `models.json`), update the crate's `BUILD.bazel` `compile_data` (Constitution Workflow #6). If only `models.json` was edited, confirm it is already in `compile_data`.
- [ ] T041 If `Cargo.toml` dependencies changed, run `just bazel-lock-update` then `just bazel-lock-check` (Constitution Workflow #4).
- [ ] T042 [P] Document MiniMax in user docs: provider selection, `MINIMAX_API_KEY`, region override, tiers, unsupported inputs. Update [codex-rs/config.md](../../codex-rs/config.md) and any provider docs.
- [ ] T043 Run `just fmt`, then `just clippy` and `just fix -p codex-api -p codex-model-provider-info -p codex-models-manager` to clear lints (Constitution Workflow #1–2).
- [ ] T044 Run `just test -p codex-api -p codex-models-manager -p codex-tui`; then `just test` for core/common/protocol changes (Constitution Workflow #3). Avoid `--all-features`.
- [ ] T045 Execute the [quickstart.md](./quickstart.md) validation checklist (SC-001…SC-006) against a real or mock MiniMax endpoint and record results.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — start immediately.
- **Foundational (Phase 2)**: Depends on Setup. **BLOCKS all user stories** (the wire adapter is mandatory).
- **US1 (Phase 3)**: Depends on Phase 2. MVP.
- **US2 (Phase 4)**: Depends on Phase 2; builds on US1 but independently testable.
- **US3 (Phase 5)**: Depends on Phase 2; extends the catalog from US2 (T027 → T033) but independently testable.
- **Polish (Phase 6)**: Depends on all desired stories.

### Critical path

T001 → T004 → T005 → (T006, T007) → T008 → T009/T010 → Phase 3 (T016–T021) → MVP.

### Within each story

- Provider/types/models before wiring; wiring before UI; tests after the code they cover (this feature is not TDD — wire tests in Phase 2 use a mock server).
- Models before services; services before endpoints.

### Parallel Opportunities

- T002, T003 (Setup) in parallel.
- T006 and T007 in parallel (different files: request builder vs SSE parser).
- T012–T015 (wire tests) in parallel once T008–T010 land.
- T022–T024 (US1 tests) in parallel.
- T031, T032 (US2 tests) in parallel; T027 parallel with T025/T026 logic.
- T037, T038 (US3 tests) in parallel.

---

## Parallel Example: Phase 2 wire adapter

```bash
# After T004/T005 (enum + match sites), build the two translators in parallel:
Task: "Outbound request translation in codex-rs/codex-api/src/requests/anthropic.rs"   # T006
Task: "Inbound SSE translation in codex-rs/codex-api/src/sse/anthropic.rs"             # T007

# After T008–T010, run the wire tests in parallel:
Task: "Text streaming round trip test"   # T013
Task: "tool_use round trip test"          # T014
Task: "Error classification test"         # T015
```

---

## Implementation Strategy

### MVP First (User Story 1)

1. Phase 1: Setup.
2. Phase 2: Foundational wire adapter (the hard part — do not skip tests T012–T015).
3. Phase 3: US1.
4. **STOP & VALIDATE**: send a real prompt to MiniMax with a valid key; confirm a tool call works (SC-001).

### Incremental Delivery

1. Setup + Foundational → wire works against mock.
2. US1 → MiniMax usable (MVP).
3. US2 → tiers visible/selectable.
4. US3 → full catalog + onboarding.
5. Polish → schema/bazel/docs/validation.

---

## Notes

- [P] = different files, no incomplete-task dependency.
- Keep new modules < 500 LOC (Principle I); never add provider helpers to `codex-core` (Principle II).
- Adding `WireApi::Anthropic` deliberately breaks compilation at every `match` site — fix them rather than adding a wildcard arm (Principle III).
- Confirm MiniMax's exact path suffix and auth header (T002) before T008.
- Commit after each task or logical group; update the Progress Tracker table.
