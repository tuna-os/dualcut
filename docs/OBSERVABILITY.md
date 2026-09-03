# Observability Assessment & Stack Guidelines

This document provides a comprehensive audit of the telemetry, logging, metrics, and profiling capabilities within `dualcut`, alongside operational guidelines and recommendations for future instrumentation.

---

## 1. Executive Summary

`dualcut` is a Rust-based video editing engine and GTK4/Libadwaita application. Currently, no external telemetry exporter or remote backend (such as Prometheus, OpenTelemetry Collector, or OTLP endpoints) is configured. In accordance with the telemetry policy for unconfigured backends, this project operates in audit-only mode; data flows remain strictly local.

This document serves as the authoritative reference for existing logging/diagnostic mechanisms in `dualcut` and outlines guidelines for future bounded instrumentation.

---

## 2. Existing Instrumentation Subsystems

### 2.1 Logging & Diagnostics
- **Standard Diagnostics**: Currently, `dualcut` relies on standard Rust error handling (`Result<T, E>`) and stdlib printing/error formatting.
- **GTK / GStreamer Logging**: When running the GUI preview application (`preview`), GStreamer and GTK system logs (`GST_DEBUG`, `G_MESSAGES_DEBUG`) provide low-level media decoding and UI pipeline diagnostics.
- **Process Integration**: Diagnostics for subprocess execution (such as whisper.cpp integration in `captions.rs` and render tasks in `export.rs`) report status through GUI progress callbacks and error dialogs.

### 2.2 Metrics & Profiling
- **Performance Benchmarks**: Profiling and timing checks are done locally during development and export pipeline execution.
- **Unbounded Data Guardrails**: No unbounded metric labels, high-cardinality attributes, or dynamic metric allocations exist.

---

## 3. Operational Guidelines & Policy Constraints

When expanding telemetry or diagnostics in `dualcut`, all changes must adhere to the following telemetry policy rules:

1. **Local-Only Boundary**:
   - Do not send telemetry or log data off-box without an explicitly configured operator backend.
   - Do not hardcode remote collector endpoints, API keys, credentials, or secrets in code or configuration files.

2. **Bounded Attribute & Label Cardinality**:
   - Metric labels and span attributes must use static, bounded string sets (e.g., render status codes, static component names).
   - Dynamic user parameters (such as project filenames, audio track paths, or transcription text) must never be used as metric labels.

3. **Structured Diagnostics**:
   - If structured logging is added (e.g., via `tracing` or `log` crates), standard log levels (`ERROR`, `WARN`, `INFO`, `DEBUG`) must be applied consistently.
   - Subprocess failures (e.g., export pipeline crashes or whisper process errors) should produce structured error logs with standardized error codes.

---

## 4. Future OpenTelemetry & Metrics Roadmap

Should an operator backend be configured in the future, the recommended path for telemetry instrumentation in `dualcut` includes:

- **OpenTelemetry SDK Integration**: Introduce `tracing` and `tracing-opentelemetry` to instrument key pipeline functions (`document::Project` loading, `export` rendering, `silence` detection, and `karaoke` timing generation).
- **Export & Render Metrics**: Expose internal render queue metrics (e.g., `dualcut_render_duration_seconds`, `dualcut_export_jobs_total`, `dualcut_silence_detect_seconds`) using bounded metric dimensions.
- **Opt-in Exporter Configuration**: Provide environment-variable driven exporter configuration (`OTEL_EXPORTER_OTLP_ENDPOINT`) ensuring zero data leakage by default.
