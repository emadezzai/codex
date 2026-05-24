# Contract: MiniMax Provider Configuration (CLI / config.toml)

This is the user-facing configuration contract — the command/config surface a
user touches to select and configure MiniMax. It is the CLI-tool analogue of an
API contract.

## Built-in provider entry

A `minimax` provider is available out-of-the-box (FR-001, FR-003, US3). No config
is required to see it; the following describes overridable fields.

```toml
# Optional: override only what you need. The built-in defaults are shown.
[model_providers.minimax]
# name      = "MiniMax"                              # display name (built-in)
base_url    = "https://api.minimax.io/anthropic"     # override for region, e.g.
                                                     # "https://api.minimaxi.com/..." (mainland)
# env_key   = "MINIMAX_API_KEY"                      # credential env var (built-in)
# wire_api  = "anthropic"                            # built-in; not user-set normally
```

Contract rules:
- `model_providers.minimax` MAY override `base_url` (FR-003, region switch).
- Other non-default fields follow the standard provider config rules; the
  built-in definition supplies sensible defaults.
- `wire_api = "anthropic"` is a newly accepted value; `config.schema.json` MUST
  be regenerated (`just write-config-schema`) to include it.

## Selecting MiniMax and a model

```toml
model_provider = "minimax"
model          = "MiniMax-M2"            # normal tier
# model        = "MiniMax-M2-highspeed"  # high-speed tier (FR-007)
```

- Any MiniMax model id is accepted, including ids not in the bundled catalog
  (FR-010, SC-006). Uncataloged ids derive tier from the `-highspeed` marker.
- The selected model persists across sessions until changed (FR-008).

## Credential

```bash
export MINIMAX_API_KEY="sk-..."   # supplied via the standard env-var/login flow (FR-002)
```

Error contract:
- **Missing key** → clear, actionable error naming `MINIMAX_API_KEY` and how to
  set it (FR-009, SC-005).
- **Invalid/expired key** → distinct authentication error (FR-009).

## Model listing contract

When listing/selecting MiniMax models (FR-006, SC-003):
- Each model displays a tier label: `normal` or `high-speed`.
- Legacy models are clearly marked but remain selectable (FR-011).
- No model is left uncategorized.

## Unsupported input contract (FR-012)

If a turn would send an image or document to MiniMax, the turn is **blocked
before sending** with a message naming the unsupported input type. Content is
never silently dropped, partially sent, or failed opaquely.
