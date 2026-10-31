# Separate document data from document behavior

Status: accepted for the structural extraction in #203.

The document model is shared by the renderer, editor, HTTP API and scripts.
Its public paths and persisted JSON must remain compatible during decomposition.

`document.rs` is the compatibility façade. `document/schema.rs` owns the Serde
structs, enums and wire defaults; animation and effect declarations are in
schema submodules. It imports no validation, persistence or editing code.
`validate.rs` owns project and clip invariants. `persistence.rs` owns JSON
loading and saving, including validation after loading. `timing.rs` owns pure
timeline calculations and rate expansion; `color.rs` owns color conversion.
`traversal.rs` remains the shared scene/overlay traversal layer.

`commands/` groups the existing editing functions by responsibility: split,
trim/silence removal, detached audio, templates and lane movement. Operations
use schema types and traversal directly. Their existing signatures and failure
behavior remain intact. Rate expansion can be tested with a single Clip; other
operations accept in-memory Projects without parsing JSON. The regression tests
live in `document/tests.rs`, outside the implementation modules.

This extraction deliberately preserves algorithms, error text, Serde defaults,
and the unversioned model-v2 wire format. `Project::from_json` and `to_json`
remain the persistence entry points. A future version dispatcher and migrations
belong in `persistence.rs`, before validation. Introducing a version field now
would change the canonical JSON/TypeScript schemas and the agent bundle without
an actual old-to-new transformation to implement.

Remaining parts of #203 are a command protocol (including typed command errors
and transaction/rollback semantics), actual version migrations when the wire
format changes, and decomposition of the longer split/silence algorithms into
smaller operations. They should have their own behavioral tests and compatibility
contracts rather than being mixed into this mechanical extraction. The current
operation bodies are kept together to make the move auditable; a 50-line budget
alone is not sufficient reason to change their control flow.

Run `scripts/test-document.sh` to compile and test the actual document module
in a temporary minimal crate (Rust plus its Serde/anyhow dependencies, no native
media libraries). This supplements the full-engine CI commands in CONTRIBUTING;
keep its three dependency requirements aligned with `engine/Cargo.toml`.
