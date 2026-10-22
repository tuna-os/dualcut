# ADR 0005: Separate document model, persistence, and editor state

## Context

Issue #187 identified coupling in the document module. The engine's `Project`
contains persisted data, validation, and timing rules; selection, viewport,
undo/redo, and pipeline state actually live in the preview editor. Its lifecycle
coordination was already extracted to `preview/document_manager.rs` in #213.

## Decision

Keep `document.rs` as a compatibility facade over focused modules:

- `document/schema.rs` owns the persisted data types, Serde defaults, validation,
  and timing calculations. `Project` remains the existing mutable data model;
  this refactor does not change its JSON representation or introduce a version.
- `document/persistence.rs` owns JSON encoding/decoding and file loading/saving
  through `DocumentRepository`. Decoding validates before returning a project.
  `Project::from_json` and `to_json` delegate here for compatibility. The preview
  lifecycle manager uses the repository for opening and saving files.
- `document/operations.rs` owns stateless editing operations; traversal stays in
  `document/traversal.rs`. These depend on the model, not the preview editor.
- The preview `Editor`, its session state, history, and lifecycle manager retain
  selection, undo/redo, viewport, and pipeline coordination. They consume the
  engine API; the engine never imports GTK or editor state.

The facade preserves existing `document::*` imports. No parallel
`DocumentSchema` or `DocumentEditor` wrappers duplicate `Project` or `Editor`.
Persisted lane lock/hide/mute flags remain in the schema because they are part
of saved projects and affect compilation; transient selection stays in the UI.

## Consequences

Schema and operation tests run without constructing an editor. Persistence has
its own filesystem and invalid-input tests. Existing JSON files, history
snapshots, and external callers retain their behavior. Saving retains the
existing direct-write semantics; atomic writes are outside this refactor.

The format currently has no explicit version or migrations. A future format
change must define compatibility and update the canonical JSON/TypeScript
schemas and bundled copies. Detection and migration should enter through the
repository's decode path before validation, independently of UI session state.
