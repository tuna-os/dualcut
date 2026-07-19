//! File-backed agent API. The project file on disk is the bus between the
//! HTTP surface and whatever holds the file open (the editor app watches
//! mtime, so agent writes appear live). Serving and editing share no
//! memory — every request reads/writes the file.

use crate::document::Project;
use anyhow::{Context, Result};
use std::path::PathBuf;
use tiny_http::{Header, Method, Response, Server};

fn json_response(status: u16, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

/// Serve the agent API for `path` on 127.0.0.1:`port`, blocking forever.
/// Spawn on a thread to run alongside a UI.
pub fn serve_file_api(path: PathBuf, port: u16) -> Result<()> {
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| anyhow::anyhow!("binding 127.0.0.1:{port}: {e}"))?;
    println!("agent API on http://127.0.0.1:{port} (project: {})", path.display());

    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().clone();
        let mut body = String::new();
        let _ = request.as_reader().read_to_string(&mut body);

        let load = || -> Result<Project> {
            Project::from_json(&std::fs::read_to_string(&path)?)
                .with_context(|| format!("loading {}", path.display()))
        };

        let response = match (&method, url.as_str()) {
            (Method::Get, "/project") => match load() {
                Ok(p) => json_response(200, p.to_json()),
                Err(e) => json_response(500, format!(r#"{{"error":{:?}}}"#, e.to_string())),
            },
            (Method::Post, "/project") => match Project::from_json(&body) {
                Ok(project) => match std::fs::write(&path, project.to_json()) {
                    Ok(()) => json_response(200, r#"{"ok":true}"#.into()),
                    Err(e) => json_response(500, format!(r#"{{"error":{:?}}}"#, e.to_string())),
                },
                Err(e) => json_response(400, format!(r#"{{"error":{:?}}}"#, e.to_string())),
            },
            #[cfg(feature = "scripting")]
            (Method::Post, "/script") => match load()
                .and_then(|p| crate::scripting::run_script(&body, &p))
            {
                Ok(edited) => match std::fs::write(&path, edited.to_json()) {
                    Ok(()) => json_response(200, r#"{"ok":true}"#.into()),
                    Err(e) => json_response(500, format!(r#"{{"error":{:?}}}"#, e.to_string())),
                },
                Err(e) => json_response(400, format!(r#"{{"error":{:?}}}"#, e.to_string())),
            },
            // Editing ops with nontrivial math, so agents don't have to
            // reimplement offset/animation splitting. Body: {"op": ...}.
            (Method::Post, "/op") => match load().and_then(|mut p| {
                let req: serde_json::Value = serde_json::from_str(&body)?;
                let op = req["op"].as_str().unwrap_or_default().to_string();
                let id = req["id"].as_str().unwrap_or_default().to_string();
                let result = match op.as_str() {
                    "split" => {
                        let at = req["at"].as_f64().context("'at' (seconds) required")?;
                        let new_id = crate::document::split_clip(&mut p, &id, at)?;
                        serde_json::json!({"ok": true, "new_id": new_id})
                    }
                    "ripple_delete" => {
                        crate::document::ripple_delete(&mut p, &id)?;
                        serde_json::json!({"ok": true})
                    }
                    "detach_audio" => {
                        let new_id = crate::document::detach_audio(&mut p, &id)
                            .context("clip is not a video clip")?;
                        serde_json::json!({"ok": true, "new_id": new_id})
                    }
                    "move_to_lane" => {
                        let lane = req["lane"].as_u64().context("'lane' required")? as usize;
                        let at = req["at"].as_f64().context("'at' (seconds) required")?;
                        crate::document::move_clip_to_lane(&mut p, &id, lane, at)?;
                        serde_json::json!({"ok": true})
                    }
                    other => anyhow::bail!(
                        "unknown op {other:?} (split, ripple_delete, detach_audio, move_to_lane)"
                    ),
                };
                p.validate()?;
                std::fs::write(&path, p.to_json())?;
                Ok(result)
            }) {
                Ok(v) => json_response(200, v.to_string()),
                Err(e) => json_response(400, format!(r#"{{"error":{:?}}}"#, e.to_string())),
            },
            (Method::Get, "/status") => match load() {
                Ok(p) => json_response(
                    200,
                    serde_json::json!({
                        "engine": "dualcut",
                        "version": env!("CARGO_PKG_VERSION"),
                        "project": p.meta.title,
                        "duration": p.duration(),
                        "scenes": p.scenes.len(),
                    })
                    .to_string(),
                ),
                Err(e) => json_response(500, format!(r#"{{"error":{:?}}}"#, e.to_string())),
            },
            _ => json_response(404, r#"{"error":"not found"}"#.into()),
        };
        let _ = request.respond(response);
    }
    Ok(())
}
