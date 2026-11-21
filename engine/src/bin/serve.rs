//! The agent API for the native engine — the M1 port of the v0 web
//! prototype's `/__composition` contract (see AGENTS.md).
//!
//!   GET  /project           current document (JSON)
//!   POST /project           replace document (validated; saved to disk)
//!   POST /render            {"out": "path.mp4", "profile": "mp4|webm"}
//!   POST /script            TypeScript body: `export function edit(p: Project): Project`
//!                           (requires the "scripting" cargo feature)
//!   GET  /status            engine info
//!
//! Every route but `/render` is shared with the app's file-backed server
//! (`dualcut_engine::api::serve_api`); this bin holds the project in a
//! `MemoryStore` and adds `/render` on top.
//!
//! Usage: serve <project.json> [port]     (default port 7357)

use anyhow::{Context, Result};
use dualcut_engine::api::{serve_api, MemoryStore, ProjectStore};
use dualcut_engine::{document::Project, init};
use std::path::PathBuf;
use tiny_http::{Header, Method, Response, Server};

fn json_response(status: u16, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

fn main() -> Result<()> {
    init()?;

    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().unwrap_or_else(|| "project.json".into()));
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(7357);

    let project = if path.exists() {
        Project::from_json(&std::fs::read_to_string(&path)?)
            .with_context(|| format!("loading {}", path.display()))?
    } else {
        anyhow::bail!("project file not found: {}", path.display());
    };
    let base_dir = path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
    let store = MemoryStore::new(project, path.clone());

    let server =
        Server::http(("127.0.0.1", port)).map_err(|e| anyhow::anyhow!("binding server: {e}"))?;
    println!("dualcut engine API on http://127.0.0.1:{port} (project: {})", path.display());

    serve_api(&server, &store, |store, method, url, body| {
        if (method, url) != (&Method::Post, "/render") {
            return None;
        }
        let (out, profile) = match render_request(body) {
            Ok(req) => req,
            Err(e) => {
                return Some(json_response(400, format!(r#"{{"error":{:?}}}"#, e.to_string())))
            }
        };
        let rendered = store
            .load()
            .and_then(|project| render_with_profile(&project, &base_dir, &out, &profile));
        Some(match rendered {
            Ok(warnings) => json_response(
                200,
                serde_json::json!({ "ok": true, "out": out, "warnings": warnings }).to_string(),
            ),
            Err(e) => json_response(500, format!(r#"{{"error":{:?}}}"#, e.to_string())),
        })
    });
    Ok(())
}

/// `(out, profile)` from a `/render` body. An empty body takes the
/// defaults (`out/render.mp4`, profile from `out`); a body that is not
/// JSON is an error rather than a silent render to the default path.
fn render_request(body: &str) -> Result<(String, String)> {
    let req: serde_json::Value = if body.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(body).context("render body must be JSON")?
    };
    let out: String = req["out"].as_str().unwrap_or("out/render.mp4").into();
    let profile = req["profile"].as_str().unwrap_or(&out).to_string();
    Ok((out, profile))
}

fn render_with_profile(
    project: &Project,
    base_dir: &std::path::Path,
    out: &str,
    profile: &str,
) -> Result<Vec<String>> {
    dualcut_engine::render_project(&project.to_json(), base_dir, out, profile)
}

#[cfg(test)]
mod tests {
    use super::render_request;

    #[test]
    fn render_request_defaults_on_an_empty_body() {
        let (out, profile) = render_request("").unwrap();
        assert_eq!(out, "out/render.mp4");
        assert_eq!(profile, "out/render.mp4");
        let (out, profile) = render_request(r#"{"out":"a.webm","profile":"webm"}"#).unwrap();
        assert_eq!((out.as_str(), profile.as_str()), ("a.webm", "webm"));
    }

    #[test]
    fn render_request_rejects_a_body_that_is_not_json() {
        assert!(render_request("{not json").is_err());
    }
}
