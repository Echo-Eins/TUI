use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

pub fn parse_json_array<T: DeserializeOwned>(output: &str) -> Result<Vec<T>> {
    let trimmed = output.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(Vec::new());
    }

    let value: serde_json::Value =
        serde_json::from_str(trimmed).context("Failed to parse JSON output")?;

    match value {
        serde_json::Value::Null => Ok(Vec::new()),
        serde_json::Value::Array(items) => items
            .into_iter()
            .map(|item| {
                serde_json::from_value(item).context("Failed to deserialize JSON array item")
            })
            .collect(),
        _ => {
            let item =
                serde_json::from_value(value).context("Failed to deserialize JSON object")?;
            Ok(vec![item])
        }
    }
}
