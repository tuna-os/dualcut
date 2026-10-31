//! JSON persistence boundary for the document model v2.
//! Keep legacy unversioned projects readable here; future wire migrations
//! belong at this boundary before domain validation, not in schema types.

use super::schema::Project;
use anyhow::Result;

impl Project {
    pub fn from_json(json: &str) -> Result<Self> {
        let project: Project = serde_json::from_str(json)?;
        project.validate()?;
        Ok(project)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("project serializes")
    }

}
