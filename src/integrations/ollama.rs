use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaData {
    pub status: String,
    pub models: Vec<OllamaModel>,
    pub running_model: Option<RunningModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    pub size: u64,
    pub modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunningModel {
    pub name: String,
    pub vram_usage: u64,
    pub gpu_usage: f32,
}

pub struct OllamaClient {}

impl OllamaClient {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    pub async fn collect_data(&self) -> Result<OllamaData> {
        Ok(OllamaData {
            status: "Running".to_string(),
            models: vec![],
            running_model: None,
        })
    }
}
