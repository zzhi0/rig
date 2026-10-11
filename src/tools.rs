use std::path::PathBuf;

use serde::Deserialize;
use serde_json::json;

use crate::runtime::{ToolCall, ToolDefinition, Tools};

pub struct ReadFiles {
    paths: Vec<String>,
}

impl ReadFiles {
    pub fn new(paths: Vec<PathBuf>) -> Result<Self, String> {
        let paths = paths
            .into_iter()
            .map(|path| {
                let path = path
                    .canonicalize()
                    .map_err(|error| format!("cannot select {}: {error}", path.display()))?;
                if !path.is_file() {
                    return Err(format!("not a file: {}", path.display()));
                }
                path.into_os_string()
                    .into_string()
                    .map_err(|path| format!("file path is not UTF-8: {}", path.to_string_lossy()))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { paths })
    }
}

impl Tools for ReadFiles {
    fn definitions(&self) -> Vec<ToolDefinition> {
        if self.paths.is_empty() {
            return Vec::new();
        }
        vec![ToolDefinition {
            name: "read_file".into(),
            description: "Read the UTF-8 contents of a file selected by the user.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "enum": self.paths }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }]
    }

    async fn execute(&self, call: &ToolCall) -> Result<String, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Arguments {
            path: String,
        }

        let arguments: Arguments = serde_json::from_str(&call.arguments)
            .map_err(|error| format!("invalid read_file arguments: {error}"))?;
        if !self.paths.contains(&arguments.path) {
            return Err(format!("file was not selected: {}", arguments.path));
        }
        tokio::fs::read_to_string(&arguments.path)
            .await
            .map_err(|error| format!("cannot read {}: {error}", arguments.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(path: &std::path::Path) -> ToolCall {
        ToolCall {
            id: "call-1".into(),
            name: "read_file".into(),
            arguments: json!({ "path": path }).to_string(),
        }
    }

    #[tokio::test]
    async fn reads_selected_file_at_call_time() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("selected.txt");
        std::fs::write(&path, "before").unwrap();
        let tools = ReadFiles::new(vec![path.clone()]).unwrap();
        let path = path.canonicalize().unwrap();
        assert_eq!(
            tools.definitions()[0].parameters["properties"]["path"]["enum"],
            json!([path])
        );

        std::fs::write(&path, "updated 中文").unwrap();
        assert_eq!(tools.execute(&call(&path)).await.unwrap(), "updated 中文");
    }

    #[tokio::test]
    async fn rejects_unselected_file_and_malformed_arguments() {
        let directory = tempfile::tempdir().unwrap();
        let selected = directory.path().join("selected.txt");
        let unselected = directory.path().join("unselected.txt");
        std::fs::write(&selected, "selected").unwrap();
        std::fs::write(&unselected, "private").unwrap();
        let tools = ReadFiles::new(vec![selected]).unwrap();
        assert!(
            tools
                .execute(&call(&unselected.canonicalize().unwrap()))
                .await
                .unwrap_err()
                .contains("not selected")
        );

        let mut invalid = call(&unselected);
        invalid.arguments = "{\"path\":42}".into();
        assert!(
            tools
                .execute(&invalid)
                .await
                .unwrap_err()
                .contains("invalid read_file arguments")
        );
    }

    #[tokio::test]
    async fn reports_unreadable_content_and_removed_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("selected.txt");
        std::fs::write(&path, [0xff]).unwrap();
        let tools = ReadFiles::new(vec![path.clone()]).unwrap();
        let call = call(&path.canonicalize().unwrap());
        assert!(
            tools
                .execute(&call)
                .await
                .unwrap_err()
                .contains("cannot read")
        );

        std::fs::remove_file(&path).unwrap();
        assert!(
            tools
                .execute(&call)
                .await
                .unwrap_err()
                .contains("cannot read")
        );
    }

    #[test]
    fn empty_selection_publishes_no_tools_and_directory_is_rejected() {
        assert!(ReadFiles::new(Vec::new()).unwrap().definitions().is_empty());
        let directory = tempfile::tempdir().unwrap();
        assert!(ReadFiles::new(vec![directory.path().into()]).is_err());
    }
}
