# Quickstart: Using MiniMax with Codex

This guide validates the feature end-to-end from a user's perspective. It maps
to the spec's user stories and success criteria.

## Prerequisites

- A MiniMax account and API key.
- A build of Codex including the MiniMax provider integration.

## 1. Set your credential (FR-002, FR-009)

```bash
export MINIMAX_API_KEY="your-minimax-api-key"
```

## 2. Select MiniMax as your provider and model (US1, FR-001, FR-007)

In `~/.codex/config.toml`:

```toml
model_provider = "minimax"
model          = "MiniMax-M2"
```

Or pick MiniMax + a model from the model picker in the TUI.

## 3. Run a session (US1, SC-001)

Start Codex and send a prompt. Expected:
- A coherent streamed response from MiniMax (FR-003, FR-004).
- Tool calls execute and results feed back to the model (US1 scenario 2).

## 4. Switch tiers (US2, SC-004)

In the model list, each MiniMax model shows a tier label (`normal` /
`high-speed`) (FR-006, SC-003). Switch from a normal model to its high-speed
counterpart in a single selection:

```toml
model = "MiniMax-M2-highspeed"
```

Expected: requests route to the high-speed identifier (US2 scenario 2). The
selection persists into new sessions (FR-008, US2 scenario 3).

## 5. Override the region endpoint (FR-003)

For the mainland-China endpoint:

```toml
[model_providers.minimax]
base_url = "https://api.minimaxi.com/anthropic"
```

## Validation checklist (maps to Success Criteria)

- [ ] **SC-001**: Configure MiniMax and complete a turn including a tool call, no source edits.
- [ ] **SC-002**: All bundled M-series models are selectable on first use (US3, FR-005).
- [ ] **SC-003**: Every selectable model shows a tier label; none uncategorized.
- [ ] **SC-004**: Switch normal → high-speed in one selection.
- [ ] **SC-005**: With no/invalid `MINIMAX_API_KEY`, the error names the credential and the fix.
- [ ] **SC-006**: A new MiniMax model id not in the bundled catalog works without an update (FR-010).

## Edge cases to verify

- **Unknown model id**: typing an uncataloged id forwards it as-is; tier derived
  from the `-highspeed` marker (FR-010).
- **Legacy model**: marked but still selectable (FR-011).
- **Image/document input**: the turn is blocked before sending with a message
  naming the unsupported input type (FR-012) — not silently dropped.
- **Missing vs invalid key**: distinct error messages (FR-009).
- **Rate limit / 5xx**: surfaced as recoverable, consistent with other providers (FR-013).
