# Observability Assessment & Stack Guidelines

## Current Telemetry & Subsystem Architecture

`dualcut` is a native Rust-based non-linear video editor engine (`dualcut-engine`) built on top of GStreamer (`gstreamer`, `gstreamer-editing-services`, `gstreamer-app`, `gstreamer-pbutils`), GTK4 (`gtk4`), Libadwaita (`libadwaita`), and Vello (`vello`).

### Operational Characteristics & Data Handling

1. **HTTP Local Host File API (`api.rs`)**:
   - `dualcut-engine` exposes a lightweight, local HTTP API server (`tiny_http`) bound exclusively to `127.0.0.1`.
   - Used for agent scripting, project inspect/edit ops (`/project`, `/script`, `/op`), and engine status checks (`/status`).
   - No external data transmission or remote network calls are performed by the API server.

2. **Logging & Diagnostic Tracing**:
   - The engine relies on standard GStreamer debug logging (`GST_DEBUG` environment variable) for low-level media pipeline, encoder, and demuxer diagnostics.
   - Rust standard library formatting and `anyhow` context chains propagate operational errors across pipeline creation, media file canonicalization, and project file loading.

3. **Backend Status & Export Guidelines**:
   - **No telemetry backend is currently configured or authorized** for external data export.
   - Standard telemetry policy prohibits introducing external metric exporters, OpenTelemetry SDK collectors, or telemetry endpoints when no backend has been explicitly configured by operators.
   - Any future telemetry integration must maintain local-only scoping, strict attribute cardinality controls, and user privacy protections.

## Recommendations for Future Tracing & Metrics Instrumentation

Should an operational backend be designated in the future, the following bounded instrumentation architecture is recommended:

1. **OpenTelemetry Rust Tracing (`tracing-opentelemetry`)**:
   - Instrument long-running tasks such as project rendering (`render_project_with_progress`), silence detection (`detect_silence_in_uri`), and script execution with `tracing::span`.
   - Maintain bounded span attributes (e.g., project ID hashes, clip export format, pipeline state transitions) and prohibit raw user content or file paths in exported attributes.

2. **Metrics Collection**:
   - Expose operational performance metrics (export duration, frames rendered, pipeline init latency) strictly via local, pull-based metrics endpoints (e.g., `/metrics` on localhost) or standard event listeners.
