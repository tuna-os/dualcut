//! JSON and filesystem boundary for project documents.
//!
//! The current format has no explicit version field or migration step. Future
//! format detection and migration belong here, before schema validation, rather
//! than in editor history or rendering code.

use super::schema::Project;
use anyhow::Result;
use std::path::Path;

/// Stateless persistence service; it owns no selection, history, or viewport.
pub struct DocumentRepository;

impl DocumentRepository {
    pub fn decode(json: &str) -> Result<Project> {
        let project: Project = serde_json::from_str(json)?;
        project.validate()?;
        Ok(project)
    }

    pub fn encode(project: &Project) -> String {
        serde_json::to_string_pretty(project).expect("project serializes")
    }

    pub fn load(path: &Path) -> Result<Project> {
        Self::decode(&std::fs::read_to_string(path)?)
    }

    pub fn save(path: &Path, project: &Project) -> Result<()> {
        std::fs::write(path, Self::encode(project))?;
        Ok(())
    }
}

// Compatibility methods keep every existing engine and script caller working.
impl Project {
    pub fn from_json(json: &str) -> Result<Self> {
        DocumentRepository::decode(json)
    }

    pub fn to_json(&self) -> String {
        DocumentRepository::encode(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_without_an_editor_session() {
        let project = DocumentRepository::decode(include_str!("../../examples/demo-project.json"))
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.json");
        DocumentRepository::save(&path, &project).unwrap();
        let loaded = DocumentRepository::load(&path).unwrap();
        assert_eq!(project.to_json(), loaded.to_json());
    }

    #[test]
    fn rejects_invalid_documents_and_reports_io_errors() {
        assert!(DocumentRepository::decode("not json").is_err());
        let mut json: serde_json::Value = serde_json::from_str(include_str!(
            "../../examples/demo-project.json"
        )).unwrap();
        json["meta"]["width"] = 0.into();
        assert!(DocumentRepository::decode(&json.to_string()).is_err());
        let dir = tempfile::tempdir().unwrap();
        assert!(DocumentRepository::load(&dir.path().join("missing.json")).is_err());
        let project = Project::from_json(include_str!("../../examples/demo-project.json")).unwrap();
        assert!(DocumentRepository::save(dir.path(), &project).is_err());
    }
}
