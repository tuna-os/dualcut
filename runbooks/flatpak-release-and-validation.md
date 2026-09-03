# Flatpak Release & Validation Runbook

## Overview
This runbook describes the procedure for validating, packaging, and testing Flatpak releases for `org.tunaos.dualcut`.

## Pre-Release Validation
1. Verify Flatpak manifest syntax (`engine/build-aux/org.tunaos.dualcut.json`).
2. Run local build and validation:
   ```bash
   flatpak-builder --force-clean build-dir engine/build-aux/org.tunaos.dualcut.json
   ```
3. Validate AppStream metainfo XML (`engine/build-aux/org.tunaos.dualcut.metainfo.xml`):
   ```bash
   appstream-util validate engine/build-aux/org.tunaos.dualcut.metainfo.xml
   ```

## Smoke Testing
- Launch app in sandbox:
  ```bash
  flatpak-builder --run build-dir engine/build-aux/org.tunaos.dualcut.json dualcut
  ```
- Verify video rendering pipeline, audio playback, and export functionality.

## Rollback Procedure
If a published Flatpak release experiences critical regressions:
1. Revert the commit updated in `org.tunaos.dualcut.json`.
2. Re-trigger build workflow to update Flatpak repository / Flathub build.
