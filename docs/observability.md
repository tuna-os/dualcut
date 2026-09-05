# Dualcut Observability & Operational Readiness Assessment

This document details the operational readiness, logging infrastructure, diagnostic signals, and operational guidelines for Dualcut (`org.tunaos.dualcut`).

---

## 1. Executive Summary & Maintenance Posture

- **Component Name**: Dualcut (`org.tunaos.dualcut`)
- **Architecture**: GTK4 / Libadwaita desktop video editor built with PyGObject, GStreamer Editing Services (GES), and Vello. Includes an embedded HTTP API server for agentic automation and script execution.
- **Maintenance Status**: Legacy / Maintenance mode (Active development transitioning to `shrimply`).
- **Distribution**: Flathub and TunaOS OCI Flatpak Registry (`oci+https://tuna-os.github.io/flatpak-index`).

---

## 2. Telemetry & Data Flow Policy

- **Managed Observability Target**: Zero external telemetry backends configured.
- **Data Flow Policy**: No automated remote metrics collection or off-device telemetry transmission is enabled. All application diagnostics and session logs remain strictly on the local machine.
- **Dependencies**: GStreamer plugin pipeline, PyGObject, GLib main loop, local HTTP agent server.

---

## 3. Diagnostic Signal Sources

### 3.1 Session & Systemd Journal Logs
When running as a user service or Flatpak app, session logs flow through standard output (`stdout`/`stderr`) to the systemd journal:

```bash
# Stream live logs for Dualcut
journalctl --user -f -u org.tunaos.dualcut

# Retrieve recent error events
journalctl --user-unit=org.tunaos.dualcut --since "1 hour ago" -p err
```

### 3.2 GStreamer & GLib Diagnostic Logging
GStreamer pipelines emit rich diagnostic channels configured via environment variables:

```bash
# Debug GStreamer Editing Services (GES) and video rendering
GST_DEBUG=2,ges:4,gespipeline:5 flatpak run org.tunaos.dualcut

# Trace GTK4 and GLib debug events
G_MESSAGES_DEBUG=all flatpak run org.tunaos.dualcut
```

### 3.3 HTTP Agent API Diagnostics
Dualcut exposes an HTTP server interface for remote JSON project manipulation and automated timeline scripting:
- Access logs and API errors print directly to standard stderr.
- Health check endpoints verify main loop responsiveness and active project document state.

---

## 4. Operational Health & Readiness Criteria

| Interface / Surface | Health Signal | Verification Method |
| :--- | :--- | :--- |
| **Flatpak Launch** | GTK application construct & window initialization | `flatpak run org.tunaos.dualcut --gapplication-service` |
| **GStreamer Pipeline** | GES element availability & encoder plugins | `gst-inspect-1.0 ges` |
| **HTTP Agent API** | Local HTTP server response | `curl -f http://localhost:8080/healthz` (when agent API is active) |
| **Project Schema** | JSON project structure validation | `python3 -m json.tool project.json` |

---

## 5. Operations Recommendations

1. **Local Diagnostic Triage**: Use `GST_DEBUG` and `G_MESSAGES_DEBUG` flags during timeline playback or render pipeline investigation.
2. **Deprecation Guidance**: Critical security and crash bugfixes are maintained in Dualcut; new feature requests should be evaluated against `shrimply`.
