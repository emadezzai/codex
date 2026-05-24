# Feature Specification: MiniMax Provider Integration

**Feature Branch**: `001-minimax-provider`  
**Created**: 2026-05-24  
**Status**: Draft  
**Input**: User description: "Integrate MiniMax as a new provider. Configure the integration to include all available MiniMax models, categorized into 'normal' and 'high-speed' tiers."

## Clarifications

### Session 2026-05-24

- Q: Which MiniMax service endpoint/region should the integration target? → A: Default to the international endpoint (`api.minimax.io`), with the base URL overridable via configuration.
- Q: How does the user supply the MiniMax API credential? → A: Via Codex's existing login/auth flow using the MiniMax API key, reusing the same credential mechanism (including standard env-var support) as other providers — no MiniMax-specific credential path.
- Q: What is the scope of the bundled model catalog? → A: Only the models MiniMax serves over its Anthropic-compatible endpoint (the M-series, e.g. `MiniMax-M2`, plus their high-speed variants); legacy `abab`/Text chat models reachable only via other interfaces are excluded.
- Q: How is a model's tier determined? → A: Store an explicit `tier` field on each catalog model; for uncataloged (forward-compatible) ids, fall back to detecting the `-highspeed` marker (absent → normal).
- Q: How should the system handle unsupported (image/document) inputs? → A: Block the turn and show a clear message naming the unsupported input type (do not silently drop or send partial content).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Use MiniMax as a model provider (Priority: P1)

A user who has a MiniMax account and API key wants to run their coding sessions against MiniMax models instead of the default provider. They configure MiniMax as their provider, supply their API key, pick a MiniMax model, and begin working with full conversational and tool-use capabilities.

**Why this priority**: This is the core value of the feature. Without the ability to actually select MiniMax and run a session against it, nothing else matters. It delivers a complete, usable provider on its own.

**Independent Test**: Configure MiniMax with a valid API key, select any MiniMax model, send a prompt, and confirm a coherent response is returned with tool calls functioning.

**Acceptance Scenarios**:

1. **Given** a user with a valid MiniMax API key configured, **When** they select MiniMax as the active provider and send a message, **Then** the system routes the request to MiniMax and returns the model's response.
2. **Given** MiniMax is the active provider, **When** the model issues a tool call during a session, **Then** the tool call is executed and the result is fed back to the model as in any other provider.
3. **Given** no MiniMax API key is configured, **When** the user selects MiniMax and sends a message, **Then** the system reports a clear, actionable error explaining that the API key is missing and how to set it.

---

### User Story 2 - Choose between normal and high-speed model tiers (Priority: P2)

A user wants to trade off response quality against latency. For a given MiniMax model family they can choose the "normal" tier (higher quality, slower) or the "high-speed" tier (faster, lower latency). The available models are presented grouped by these two tiers so the choice is obvious.

**Why this priority**: Tier selection is the distinguishing requirement the user called out explicitly. It builds directly on top of P1 (a working provider) and adds meaningful user-facing value, but the provider is usable without it.

**Independent Test**: With MiniMax configured, list the available models and confirm each model is labeled as either "normal" or "high-speed" tier; select a high-speed variant and confirm requests are routed to that variant.

**Acceptance Scenarios**:

1. **Given** MiniMax is configured, **When** the user views the list of available MiniMax models, **Then** each model is presented with its tier classification ("normal" or "high-speed").
2. **Given** a model family with both tiers (e.g. a standard and a high-speed variant), **When** the user selects the high-speed variant, **Then** requests are sent to the high-speed model identifier.
3. **Given** the user has selected a specific tier/model, **When** they start a new session, **Then** that selection persists as their chosen MiniMax model.

---

### User Story 3 - Discover and configure MiniMax without external lookup (Priority: P3)

A user new to MiniMax wants to set it up without leaving the tool to research model names or endpoints. The full catalog of supported MiniMax models ships with the integration, and the configuration guidance explains which credential is required.

**Why this priority**: A convenience and onboarding improvement. The provider works without it (a user could type a model id manually), but bundling the catalog and guidance reduces setup friction.

**Acceptance Scenarios**:

1. **Given** a fresh installation, **When** the user inspects the available providers, **Then** MiniMax appears as a selectable built-in provider with its full set of models already known.
2. **Given** the user selects MiniMax for the first time, **When** the credential is not yet set, **Then** the system surfaces instructions naming the exact credential required.

---

### Edge Cases

- **Unknown / future model id**: The user specifies a MiniMax model name that is not in the bundled catalog. The system should still attempt the request (forward the id as-is) rather than blocking, since MiniMax adds and retires models over time.
- **Legacy models**: Some MiniMax models are marked legacy but still available. They should remain selectable and clearly distinguishable from current models.
- **Unsupported input types**: MiniMax's interface does not accept image or document inputs. When a session would send such input, the system must block the turn and present a clear message naming the unsupported input type, rather than silently dropping it, sending a partial request, or failing opaquely.
- **Authentication failure**: An invalid or expired API key must produce a clear authentication error, distinct from a "missing key" error.
- **Tier without a counterpart**: A model that exists in only one tier (e.g. a base model with no high-speed variant) must still be selectable and categorized sensibly.
- **Rate limiting / upstream errors**: Provider-side throttling or 5xx responses must be surfaced as recoverable errors consistent with how other providers' errors are reported.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST offer MiniMax as a selectable provider alongside existing providers.
- **FR-002**: System MUST allow the user to supply a MiniMax API credential through Codex's existing login/auth flow (the same credential mechanism, including standard environment-variable support, used by other providers) and use it to authenticate requests to MiniMax.
- **FR-003**: System MUST route conversation requests for a MiniMax-selected session to the MiniMax service using its Anthropic-compatible message interface. The default base URL MUST target the international endpoint (`api.minimax.io`) and MUST be overridable via configuration (e.g. for the mainland-China endpoint `api.minimaxi.com`).
- **FR-004**: System MUST support the core conversational capabilities MiniMax exposes: multi-turn messages, system prompts, streaming responses, temperature control, tool definitions, tool choice, and reasoning content.
- **FR-005**: System MUST include the catalog of MiniMax models served over the Anthropic-compatible endpoint (the M-series, e.g. `MiniMax-M2`, plus their high-speed variants) so users can select any of them without manual entry. Legacy `abab`/Text chat models reachable only via other (non-Anthropic) interfaces are out of scope for the bundled catalog.
- **FR-006**: System MUST classify each MiniMax model into one of two tiers — "normal" or "high-speed" — and expose that classification to the user when listing or selecting models. Tier MUST be stored as an explicit field on each bundled catalog model; for uncataloged (forward-compatible) model ids, the system MUST derive the tier by detecting the `-highspeed` marker (its absence denoting the normal tier).
- **FR-007**: System MUST allow the user to select any specific MiniMax model (and thereby its tier) as the active model for a session.
- **FR-008**: System MUST persist the user's selected MiniMax model across sessions until they change it.
- **FR-009**: System MUST present a clear, actionable error when the MiniMax credential is missing, and a distinct error when authentication fails.
- **FR-010**: System MUST allow a user to use a MiniMax model identifier that is not part of the bundled catalog (forward-compatibility for newly released models).
- **FR-011**: System MUST clearly indicate when a selected MiniMax model is a legacy model.
- **FR-012**: System MUST handle MiniMax-unsupported inputs (images, documents) by blocking the turn before sending and showing a clear message that names the unsupported input type, rather than silently dropping the content, sending a partial request, or failing opaquely.
- **FR-013**: System MUST surface MiniMax upstream errors (rate limits, server errors) in a manner consistent with existing providers.

### Key Entities *(include if feature involves data)*

- **Provider (MiniMax)**: A selectable model provider. Attributes: display name, service endpoint, required credential reference, the message-interface style it speaks (Anthropic-compatible).
- **Model**: A MiniMax model offering. Attributes: model identifier, model family, tier ("normal" or "high-speed"), lifecycle status (current or legacy), context window, supported input/output capabilities.
- **Tier**: A categorization of models. Two values: "normal" (quality-optimized) and "high-speed" (latency-optimized). Used to group and label models for selection.
- **Credential**: The user-supplied API key used to authenticate to MiniMax.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user can configure MiniMax and complete a working conversational turn (including at least one tool call) without editing any source code.
- **SC-002**: 100% of the MiniMax models served over the Anthropic-compatible endpoint are selectable in the integration on first use.
- **SC-003**: Every selectable MiniMax model displays a tier label ("normal" or "high-speed") with no model left uncategorized.
- **SC-004**: A user can switch from a normal-tier model to its high-speed counterpart in a single selection action, with no further configuration.
- **SC-005**: When a credential is missing or invalid, 100% of such attempts produce an error message that names the required credential and the corrective action.
- **SC-006**: A newly released MiniMax model id (not in the bundled catalog) can be used successfully without a software update.

## Assumptions

- MiniMax exposes an Anthropic-compatible message interface authenticated by a single API key; the integration uses this interface rather than a bespoke protocol. The default endpoint is the international host (`api.minimax.io`), overridable via configuration.
- The "normal" vs "high-speed" tiering follows MiniMax's own naming convention, where high-speed variants carry an explicit marker (e.g. a "-highspeed" suffix) and the absence of that marker denotes the normal tier. Bundled models carry an explicit tier field; this marker convention is used only as the fallback for uncataloged ids.
- The bundled model catalog reflects MiniMax's currently published models at implementation time; the integration tolerates the catalog drifting out of date by allowing arbitrary model ids (FR-010).
- Image and document inputs are out of scope because the MiniMax interface does not accept them; text, tool calls, and reasoning content are in scope.
- Credential storage, session persistence, and provider-selection mechanisms reuse the existing patterns already present for other providers rather than introducing new ones.
- Pricing, billing, and quota management are handled by MiniMax and are out of scope for this integration.
