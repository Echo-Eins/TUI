use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabType {
    Cpu,
    Gpu,
    Ram,
    Disk,
    Network,
    Ollama,
    Processes,
    Services,
    DiskAnalyzer,
    Settings,
}

impl TabType {
    pub fn as_str(&self) -> &str {
        match self {
            TabType::Cpu => "CPU",
            TabType::Gpu => "GPU",
            TabType::Ram => "RAM",
            TabType::Disk => "Disk",
            TabType::Network => "Network",
            TabType::Ollama => "Ollama",
            TabType::Processes => "Processes",
            TabType::Services => "Services",
            TabType::DiskAnalyzer => "Disk Analyzer",
            TabType::Settings => "Settings",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "cpu" => Some(TabType::Cpu),
            "gpu" => Some(TabType::Gpu),
            "ram" => Some(TabType::Ram),
            "disk" => Some(TabType::Disk),
            "network" => Some(TabType::Network),
            "ollama" => Some(TabType::Ollama),
            "processes" => Some(TabType::Processes),
            "services" => Some(TabType::Services),
            "disk_analyzer" => Some(TabType::DiskAnalyzer),
            "settings" => Some(TabType::Settings),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn all() -> Vec<TabType> {
        vec![
            TabType::Cpu,
            TabType::Gpu,
            TabType::Ram,
            TabType::Disk,
            TabType::Network,
            TabType::Ollama,
            TabType::Processes,
            TabType::Services,
            TabType::DiskAnalyzer,
            TabType::Settings,
        ]
    }
}

pub struct TabManager {
    pub tabs: Vec<TabType>,
    pub current_index: usize,
}

impl TabManager {
    pub fn new(enabled_tabs: Vec<String>, default_tab: &str) -> Self {
        let tabs: Vec<TabType> = enabled_tabs
            .iter()
            .filter_map(|s| TabType::from_str(s))
            .collect();

        let current_index = tabs
            .iter()
            .position(|t| t.as_str().to_lowercase() == default_tab.to_lowercase())
            .unwrap_or(0);

        Self {
            tabs,
            current_index,
        }
    }

    pub fn current(&self) -> TabType {
        self.tabs[self.current_index]
    }

    pub fn next(&mut self) {
        self.current_index = (self.current_index + 1) % self.tabs.len();
    }

    pub fn previous(&mut self) {
        if self.current_index == 0 {
            self.current_index = self.tabs.len() - 1;
        } else {
            self.current_index -= 1;
        }
    }

    pub fn select(&mut self, tab: TabType) {
        if let Some(index) = self.tabs.iter().position(|&t| t == tab) {
            self.current_index = index;
        }
    }
}
