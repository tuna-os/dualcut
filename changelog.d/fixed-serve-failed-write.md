The headless `serve` binary now answers a failed project write (read-only file,
full disk, removed directory) with `500 {"error":…}` and keeps serving, instead
of exiting. Its `POST /render` returns `400` for a body that is not JSON instead
of silently rendering with defaults, and the app's `POST /script` answers `501`
(not `404`) when built without the `scripting` feature, matching `serve`.
