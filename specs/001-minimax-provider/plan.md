# Implementation Plan: MiniMax Provider Integration

**Branch**: `001-minimax-provider` | **Date**: 2026-05-24 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/001-minimax-provider/spec.md`

## Summary

Add MiniMax as a built-in, selectable model provider that speaks MiniMax's
Anthropic-compatible Messages interface (`/v1/messages`), defaulting to the
international endpoint `https://api.minimax.io` with an overridable base URL.
The integration bundles the M-series catalog (e.g. `MiniMax-M2` plus
`-highspeed` variants), classifies each model into a `normal` or `high-speed`
tier (explicit field, with a `-highspeed`-marker fallback for uncataloged ids),
reuses Codex's existing credential/auth flow (API-key env var), and blocks
image/document inputs with a clear message before sending.

**Technical reality that shapes the approach**: Codex's wire layer
(`codex-api`) currently speaks *only* the OpenAI Responses API
(`WireApi::Responses`; the `chat` variant was removed — see
[model-provider-info/src/lib.rs:45-79](../../codex-rs/model-provider-info/src/lib.rs#L45-L79)).
The internal turn model is `ResponseItem` serialized to the Responses
`input`/SSE shape. MiniMax does **not** expose a Responses-API endpoint; it
exposes an Anthropic Messages API. Therefore the core engineering work is a new
**Anthropic-Messages wire adapter** plus a new `WireApi::Anthropic` variant —
not just a config entry. This is the dominant risk and is resolved in
[research.md](./research.md).

## Technical Context

**Language/Version**: Rust (workspace `codex-rs`, edition per workspace toolchain `rust-toolchain.toml`)
**Primary Dependencies**: `reqwest` (HTTP/SSE), `serde`/`serde_json`, `tokio`, `eventsource-stream`/existing SSE plumbing in `codex-api`, `schemars` (config schema), `async-trait`
**Storage**: Bundled model catalog (`codex-rs/models-manager/models.json` or a MiniMax-specific catalog file, compiled via `include_str!`); on-disk model cache (`models_cache.json`); credential via auth/env var (existing `auth.json` + env-var flow)
**Testing**: `cargo test`/`just test -p <crate>`; SSE/wire tests in `codex-api` (mock server like existing `tests/sse_end_to_end.rs`); `insta` snapshots in `tui` for any user-visible model-list/tier UI; `core_test_support::responses` helpers do not cover Anthropic wire, so new mock helpers are needed
**Target Platform**: Cross-platform CLI/TUI (macOS, Linux, Windows) — same as Codex
**Project Type**: Single Rust workspace (multi-crate); CLI + TUI + core
**Performance Goals**: Parity with existing providers; streaming first-token latency dominated by upstream, no added buffering of full responses
**Constraints**: Must not bloat `codex-core` (Principle II); new wire functionality belongs in `codex-api`/`model-provider`/new crate, not `core`; modules < 500 LOC target; no `#[async_trait]` in *new* traits where RPITIT is feasible (existing `ModelProvider` trait already uses `async_trait`)
**Scale/Scope**: ~6–12 bundled MiniMax models; one new wire protocol; one new built-in provider; tier metadata + input-modality gating

### Resolved unknowns (see research.md)

- **Wire protocol gap** → Add `WireApi::Anthropic` + an Anthropic Messages request builder & SSE translator in `codex-api`, translating to/from internal `ResponseItem` stream events.
- **Credential mechanism** → Reuse `ModelProviderInfo.env_key` (e.g. `MINIMAX_API_KEY`) and the existing login/auth flow; MiniMax authenticates with `Authorization: Bearer <key>` (Anthropic-style `x-api-key` confirmed in research).
- **Tier model** → Explicit `tier` field on bundled catalog entries; `-highspeed` suffix detection as forward-compat fallback.
- **Unsupported inputs** → Gate on `input_modalities` (text/tool/reasoning only) and block the turn with a typed error before request construction.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Assessment | Action |
|-----------|------------|--------|
| I. Small, Focused Modules | New wire adapter must be split into request-builder + SSE-translator + types modules, each < 500 LOC; do not extend central TUI files with provider-specific helpers | Place adapter under `codex-api/src/endpoint/anthropic/` and `requests/anthropic.rs`, mirroring the `responses` layout |
| II. Crate Decoupling & Bloat Resistance | No new generic helpers in `codex-core`. Wire logic lives in `codex-api`; provider definition in `codex-model-provider-info` / `model-provider`; catalog in `codex-models-manager` | Confirmed: changes confined to API/provider/manager crates + config/tui surface |
| III. Idiomatic Rust / Clippy | Inline format args, exhaustive matches (notably the `WireApi` match arms — adding `Anthropic` forces all `match wire_api` sites to be updated, no wildcard), enums over bool params, doc comments on the new translator trait | Audit every `match` on `WireApi`; run `just clippy` + `just fix` |
| IV. TUI Styling | Model/tier listing must use `Stylize` helpers, no hardcoded white, `textwrap`/`wrapping.rs` for any new wrapped text | Apply if tier labels are added to model picker |
| V. Test Discipline | `insta` snapshots for any user-visible model-list/tier/error UI; `pretty_assertions::assert_eq`; no env mutation in tests; new Anthropic mock-server helper analogous to `core_test_support::responses` | Add wire tests + snapshot tests |
| API Standards (app-server v2) | If model/provider listing is exposed over app-server v2, follow `Params`/`Response`/`Notification` + camelCase + `#[ts(...)]` conventions; run `just write-app-server-schema` | Only if v2 surface changes |
| Workflow | `just fmt`, `just clippy`, `just fix -p`, `just test -p`; `just write-config-schema` (ConfigToml touched for provider/base-url override); update `BUILD.bazel` `compile_data` if a new `include_str!` catalog file is added; `just bazel-lock-update`/`check` if deps change | Mandatory before finalizing |

**Gate result**: PASS (no unjustified violations). The new `WireApi::Anthropic`
variant and Anthropic wire adapter are necessary, not optional — MiniMax cannot
be reached over the existing Responses API. No entries required in Complexity
Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/001-minimax-provider/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output (provider config + wire contract)
│   ├── minimax-provider-config.md
│   └── anthropic-wire-contract.md
├── checklists/
│   └── requirements.md  # (already present)
└── tasks.md             # Phase 2 output (/speckit.tasks — NOT created here)
```

### Source Code (repository root: `codex-rs/`)

```text
codex-rs/
├── model-provider-info/src/
│   └── lib.rs                      # ADD WireApi::Anthropic; ADD built-in `minimax`
│                                   #   provider (create_minimax_provider); base-url
│                                   #   override + MINIMAX_API_KEY env_key
├── codex-api/src/
│   ├── requests/
│   │   ├── mod.rs                  # route by wire protocol
│   │   └── anthropic.rs            # NEW: build /v1/messages payload from ResponseItem[]
│   ├── sse/
│   │   ├── mod.rs
│   │   └── anthropic.rs            # NEW: translate Anthropic SSE → internal stream events
│   └── endpoint/
│       └── anthropic/              # NEW: messages endpoint (mirror endpoint/responses.rs)
│           ├── mod.rs
│           └── messages.rs
├── model-provider/src/
│   └── provider.rs                 # wire dispatch / capabilities for the MiniMax provider
├── models-manager/
│   ├── models.json                 # ADD MiniMax M-series entries (tier, legacy, modalities)
│   │                               #   OR new minimax_models.json (+ BUILD.bazel compile_data)
│   └── src/model_info.rs           # tier field plumbing + `-highspeed` fallback derivation
├── core/src/
│   └── client.rs                   # honor WireApi::Anthropic; pre-send modality gate (error)
├── config/ (+ config.md, config.schema.json)
│                                   # base-url override docs; run just write-config-schema
└── tui/src/                        # model picker: surface tier + legacy labels (snapshots)
```

**Structure Decision**: Single Rust workspace (existing). Work is distributed
across `model-provider-info` (provider definition + `WireApi`), `codex-api`
(the new Anthropic wire adapter — the bulk of the effort), `models-manager`
(catalog + tier), `core` (wire dispatch + input-modality gate), and `tui`
(tier/legacy display). No new top-level project; a dedicated wire-adapter
module tree is added inside `codex-api` to satisfy Principle I (focused
modules) and Principle II (keep it out of `core`).

## Complexity Tracking

> No constitution violations requiring justification. The new wire protocol is
> the minimal way to reach MiniMax and is implemented in the API crate rather
> than `core`, consistent with the constitution. Table intentionally empty.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| (none)    | —          | —                                   |
