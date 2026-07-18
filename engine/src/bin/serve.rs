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
//! Usage: serve <project.json> [port]     (default port 7357)

use anyhow::{Context, Result};
use dualcut_engine::{document::Project, encoding_profile, init, mapping, run_to_eos};
use ges::prelude::*;
use gstreamer_editing_services as ges;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use tiny_http::{Header, Method, Response, Server};

struct State {
    project: Project,
    path: PathBuf,
}

fn json_response(status: u32, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(status as u16)
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
    let state = Mutex::new(State { project, path: path.clone() });

    let server =
        Server::http(("127.0.0.1", port)).map_err(|e| anyhow::anyhow!("binding server: {e}"))?;
    println!("dualcut engine API on http://127.0.0.1:{port} (project: {})", path.display());

    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().clone();
        let mut body = String::new();
        let _ = request.as_reader().read_to_string(&mut body);

        let response = match (&method, url.as_str()) {
            (Method::Get, "/project") => {
                let state = state.lock().unwrap();
                json_response(200, state.project.to_json())
            }
            (Method::Post, "/project") => match Project::from_json(&body) {
                Ok(project) => {
                    let mut state = state.lock().unwrap();
                    std::fs::write(&state.path, project.to_json())
                        .context("saving project file")?;
                    state.project = project;
                    json_response(200, r#"{"ok":true}"#.into())
                }
                Err(e) => json_response(400, format!(r#"{{"error":{:?}}}"#, e.to_string())),
            },
            (Method::Post, "/render") => {
                let req: serde_json::Value =
                    serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                let out: String = req["out"].as_str().unwrap_or("out/render.mp4").into();
                let profile = req["profile"].as_str().unwrap_or(&out).to_string();
                let _ = &profile;
                let project = state.lock().unwrap().project.clone();
                match render_with_profile(&project, &base_dir, &out, &profile) {
                    Ok(warnings) => json_response(
                        200,
                        serde_json::json!({ "ok": true, "out": out, "warnings": warnings })
                            .to_string(),
                    ),
                    Err(e) => json_response(500, format!(r#"{{"error":{:?}}}"#, e.to_string())),
                }
            }
            (Method::Post, "/script") => {
                #[cfg(feature = "scripting")]
                {
                    let project = state.lock().unwrap().project.clone();
                    match run_script(&body, &project) {
                        Ok(project) => {
                            let mut state = state.lock().unwrap();
                            std::fs::write(&state.path, project.to_json())
                                .context("saving project file")?;
                            state.project = project;
                            json_response(200, r#"{"ok":true}"#.into())
                        }
                        Err(e) => json_response(400, format!(r#"{{"error":{:?}}}"#, e.to_string())),
                    }
                }
                #[cfg(not(feature = "scripting"))]
                {
                    json_response(
                        501,
                        r#"{"error":"engine built without the scripting feature"}"#.into(),
                    )
                }
            }
            (Method::Get, "/status") => {
                let state = state.lock().unwrap();
                json_response(
                    200,
                    serde_json::json!({
                        "engine": "dualcut",
                        "version": env!("CARGO_PKG_VERSION"),
                        "project": state.project.meta.title,
                        "duration": state.project.duration(),
                        "scenes": state.project.scenes.len(),
                    })
                    .to_string(),
                )
            }
            _ => json_response(404, r#"{"error":"not found"}"#.into()),
        };
        let _ = request.respond(response);
    }
    Ok(())
}


fn render_with_profile(
    project: &Project,
    base_dir: &std::path::Path,
    out: &str,
    profile: &str,
) -> Result<Vec<String>> {
    let compiled = mapping::compile(project, base_dir)?;
    let pipeline = ges::Pipeline::new();
    pipeline.set_timeline(&compiled.timeline).context("attaching timeline")?;
    if let Some(parent) = std::path::Path::new(out).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let out_abs = std::path::absolute(out)?;
    pipeline.set_render_settings(&format!("file://{}", out_abs.display()), &encoding_profile(profile)?)?;
    pipeline.set_mode(ges::PipelineFlags::RENDER)?;
    run_to_eos(&pipeline)?;
    Ok(compiled.warnings)
}

/// Run a TypeScript module against the current document. The module must
/// export `edit(project: Project): Project`; the returned document is
/// validated before being accepted.
#[cfg(feature = "scripting")]
fn run_script(source: &str, project: &Project) -> Result<Project> {
    use rustyscript::{json_args, Module, Runtime, RuntimeOptions};
    let mut runtime = Runtime::new(RuntimeOptions::default())?;
    let module = Module::new("agent-script.ts", source);
    let handle = runtime.load_module(&module)?;
    let value: serde_json::Value = runtime.call_function(
        Some(&handle),
        "edit",
        json_args!(serde_json::to_value(project)?),
    )?;
    let edited: Project = serde_json::from_value(value).context("script returned invalid document")?;
    edited.validate()?;
    Ok(edited)
}
