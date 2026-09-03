# Observability Assessment & Stack Guidelines

## Operational Posture

`dualcut` is a desktop video editing application and Rust rendering engine (`org.tunaos.dualcut`).

### Telemetry Exporter Configuration
- **Backend Status:** No external metric collection or log forwarding exporter is configured.
- **Client-Side Logging:** `dualcut` relies on standard Rust logging primitives (`tracing` / `log`) and GTK/GES stderr console output.
- **Local Diagnostics:** Execution and render logs are written to stdout/stderr or local application logs when invoked via CLI/terminal or Flatpak sandbox (`~/.var/app/org.tunaos.dualcut/`).

## Stack Guidelines & Privacy Boundary

1. **Zero External Exporters:** Do not introduce external telemetry collectors, HTTP log forwarders, or analytics SDKs.
2. **Local Diagnostics Only:** All diagnostics must remain on the user's local system.
3. **Structured Stderr/Stdout:** Future logging enhancements must target stderr/stdout or local file sinks without network side-effects.
