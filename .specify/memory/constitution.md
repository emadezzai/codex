<!--
Sync Impact Report:
- Version change: none (first template instantiation) → 1.0.0
- List of modified principles:
  - [PRINCIPLE_1_NAME] → I. Small, Focused Modules
  - [PRINCIPLE_2_NAME] → II. Crate Decoupling & Bloat Resistance
  - [PRINCIPLE_3_NAME] → III. Idiomatic Rust, Formatting & Clippy Standards
  - [PRINCIPLE_4_NAME] → IV. TUI Styling & Wrapping Conventions
  - [PRINCIPLE_5_NAME] → V. Test Discipline & Snapshot Validation
- Added sections:
  - Additional Constraints & API Standards (formerly [SECTION_2_NAME])
  - Development Workflow & Build Actions (formerly [SECTION_3_NAME])
- Removed sections: None
- Templates requiring updates:
  - ✅ updated: None (templates did not require custom edits since they refer to the constitution dynamically)
- Follow-up TODOs: None (all placeholders defined)
-->

# Codex CLI Constitution

## Core Principles

### I. Small, Focused Modules
Rust modules MUST target under 500 lines of code, excluding tests. If a file exceeds roughly 800 lines of code, you MUST implement new functionality in a new module rather than extending the existing file, unless there is a strong, documented justification. High-touch central files (e.g. `codex-rs/tui/src/app.rs`, `codex-rs/tui/src/bottom_pane/chat_composer.rs`, `codex-rs/tui/src/bottom_pane/footer.rs`, `codex-rs/tui/src/chatwidget.rs`, and `codex-rs/tui/src/bottom_pane/mod.rs`) MUST be kept focused on orchestration; do not add standalone helper methods to these files unless trivial. When extracting code, related tests and documentation MUST be moved alongside the implementation to keep invariants close to their code.

### II. Crate Decoupling & Bloat Resistance
Do not add generic helper methods, standalone features, or API logic to `codex-core` (`codex-rs/core/`). You MUST verify if another existing crate is appropriate, or introduce a new crate to the Cargo workspace for any new functionality, refactoring existing code as necessary. Crate APIs MUST remain private by default, explicitly exporting public crate APIs.

### III. Idiomatic Rust, Formatting & Clippy Standards
You MUST follow strict formatting and Clippy conventions:
1. Inline variables into `{}` in `format!` arguments whenever possible.
2. Collapse nested `if` statements.
3. Use method references over closures where applicable.
4. Avoid boolean or ambiguous `Option` parameters that force callers to write hard-to-read code; prefer enums, named methods, newtypes, or other self-documenting Rust API shapes.
5. If positional-literals are unavoidable, follow the `argument_comment_lint` convention by using an exact `/*param_name*/` comment before opaque literal arguments (e.g., `/*param_name*/ None`).
6. Match statements MUST be exhaustive and avoid wildcard arms.
7. Newly added traits MUST include documentation comments explaining their role and implementation guidelines.
8. Discourage both `#[async_trait]` and `#[allow(async_fn_in_trait)]` in traits; prefer native RPITIT trait methods returning futures with explicit `Send` bounds.

### IV. TUI Styling & Wrapping Conventions
All changes affecting the Terminal User Interface (TUI) MUST adhere to `codex-rs/tui/styles.md`:
1. Use concise styling helpers from ratatui's `Stylize` trait (e.g., `"text".dim()`) instead of manual `Style` construction.
2. Avoid hardcoded white (`.white()`); use the default foreground.
3. Use `"text".into()` for spans and `vec![...].into()` for lines where type inference is clear; use `Line::from` or `Span::from` to resolve ambiguities.
4. Keep styles compact and check wrapping rules. Always use `textwrap::wrap` for plain strings, and the word-wrap helpers in `tui/src/wrapping.rs` (e.g., `word_wrap_lines`) for ratatui lines, utilizing initial/subsequent indentation options or prefix helpers like `prefix_lines`.

### V. Test Discipline & Snapshot Validation
Any change that affects user-visible UI MUST include corresponding `insta` snapshot coverage in `codex-rs/tui`. Tests MUST use `pretty_assertions::assert_eq` for clear diffs, and perform deep equals comparisons on entire objects rather than verifying fields one by one. Avoid mutating the process environment in tests; prefer passing environment-derived flags or dependencies. Spawn workspace binaries using `codex_utils_cargo_bin::cargo_bin` and resolve fixture/resource paths with `codex_utils_cargo_bin::find_resource!`. For core integration tests, utilize `core_test_support::responses` mocking helpers.

## Additional Constraints & API Standards

All active API development MUST happen in app-server v2 in `app-server-protocol/src/protocol/v2.rs`. Adhere to the following conventions:
1. Follow payload naming consistently: request payloads must end in `Params`, responses in `Response`, and notifications in `Notification`.
2. Expose RPC methods as singular `<resource>/<method>` (e.g., `thread/read`, `app/list`).
3. Serialize fields as camelCase via `#[serde(rename_all = "camelCase")]` and matching `#[ts(rename = "...")]`.
4. Optional fields in `*Params` MUST be annotated with `#[ts(optional = nullable)]`. Optional collections (`Vec`, `HashMap`) MUST use `Option` and MUST NOT use `#[serde(default)]` or `skip_serializing_if` for v2.
5. Discriminated unions MUST specify explicit tagging in both serializers: `#[serde(tag = "type")]` and `#[ts(tag = "type")]`.
6. Timestamps MUST be integer Unix seconds (`i64`) named `*_at`. IDs MUST use plain `String` at the API boundary.
7. For experimental APIs, use `#[experimental("method/or/field")]` and schema generation commands (`just write-app-server-schema`).

## Development Workflow & Build Actions

Ensure the development flow adheres to standard workspace gates:
1. Run `just fmt` automatically after making Rust code changes.
2. Run `just clippy` and `just fix -p <project>` to fix any lint issues in the code before finalizing changes.
3. Run tests using `just test -p <project>`. Once these pass, run `just test` for changes in common, core, or protocol. Avoid `--all-features` for routine local runs.
4. If modifying dependencies in `Cargo.toml`, run `just bazel-lock-update` followed by `just bazel-lock-check` to keep lockfiles synchronized.
5. If modifying `ConfigToml` or nested config types, run `just write-config-schema` to update `config.schema.json`.
6. Update the crate's `BUILD.bazel` (`compile_data`, `build_script_data`, or test data) if compile-time file macros (`include_str!`, `include_bytes!`, etc.) are introduced.

## Governance

1. This constitution establishes the code health, API, styling, and testing standards for all changes within the `codex` workspace.
2. AI assistants and contributors MUST verify compliance with all five core principles before proposing or completing any feature implementation.
3. Deviations or exceptions to these rules must be explicitly documented and justified in the Complexity Tracking table of the feature's `plan.md`.
4. Amendments to this constitution must increment the version according to semantic versioning (Major for rule removal/relaxation, Minor for new rules/expanded guidance, Patch for clarifications) and must update any dependent templates.
5. `AGENTS.md` and TUI styling files serve as runtime development guidance and must be updated if this constitution is amended.

**Version**: 1.0.0 | **Ratified**: 2026-05-24 | **Last Amended**: 2026-05-24
