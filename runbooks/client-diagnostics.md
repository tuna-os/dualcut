# Client & Engine Diagnostic Runbook

## Overview
Standardized diagnostic procedure for troubleshooting crashes, rendering failures, or pipeline errors in `dualcut`.

## Diagnostic Steps

1. **Capture Console Logs:**
   Run `dualcut` from terminal with debug logging enabled:
   ```bash
   RUST_LOG=debug GST_DEBUG=3 dualcut
   ```
   Or for Flatpak execution:
   ```bash
   flatpak run --env=RUST_LOG=debug --env=GST_DEBUG=3 org.tunaos.dualcut
   ```

2. **Inspect GStreamer / GES Pipeline:**
   Verify GStreamer plugins and GES elements are initialized correctly:
   ```bash
   gst-inspect-1.0 ges
   ```

3. **Vello / GPU Acceleration Issues:**
   If rendering fails on Vello GPU bridge (`vellosrc`), test fallback software rendering paths or inspect graphics drivers (`vulkaninfo`, `glxinfo`).

## Escalation
If issues persist due to GST/GES engine bugs or Vello integration panics, gather full trace output and file an issue using `.github/ISSUE_TEMPLATE/incident_report.md`.
