# ADR 0003: Distribute via GitHub Releases and the TunaOS Flatpak remote

Date: 2026-07-18 · Amended: 2026-08-30 · Status: accepted

## Context
The deliverable is a Flatpak. Flathub's AI policy (as of July 2026) rules out
submission for this project. The original decision used GitHub Releases as the
only distribution channel. TunaOS subsequently added its own Flatpak remote
and an organization-level reusable publication workflow.

## Decision
Every `v*` tag publishes the same application version through two channels:

1. CI builds architecture-specific bundles and attaches them to a GitHub
   Release.
2. `.github/workflows/publish-flatpak.yml` invokes the TunaOS reusable
   workflow to publish to the TunaOS Flatpak remote, the primary install path
   in the README.

`scripts/release.sh` is the sanctioned way to cut a release (it updates Cargo
and AppStream metadata, then tags and pushes). The tag, GitHub Release,
AppStream metadata, and TunaOS remote must identify the same user-visible
version.

## Consequences
- GitHub bundles provide direct, architecture-specific artifacts; the TunaOS
  remote provides normal Flatpak installation and updates.
- Publication or verification failure in either channel leaves the release
  incomplete and must be resolved before it is advertised as shipped.
- No Flathub review pipeline; sandbox permissions are a TunaOS project
  decision (network for the agent API, home for project files).
- Revisit if Flathub policy changes.
