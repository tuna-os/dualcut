# ADR 0003: Distribute via GitHub Releases only (no Flathub)

Date: 2026-07-18 · Status: accepted

## Context
The deliverable is a Flatpak. Flathub's AI policy (as of July 2026) rules
out submission for this project.

## Decision
Dualcut is distributed primarily via the canonical TunaOS Flatpak repository
(`https://tunaos.org/flatpak/tuna-os.flatpakrepo`).

In addition, every `v*` tag builds `dualcut.flatpak` in CI and attaches it to a
GitHub Release as a direct single-file download fallback. `scripts/release.sh`
is the only sanctioned way to cut a release (bumps Cargo + appstream metainfo,
tags, pushes).

## Consequences
- Primary deployment and updates are managed via the TunaOS Flatpak repo
- Direct `.flatpak` release assets on GitHub Releases remain available for manual/offline installation
- No Flathub review pipeline; sandbox holes are our own judgement
  (network for the agent API, home for project files).
- Revisit if Flathub policy changes.
