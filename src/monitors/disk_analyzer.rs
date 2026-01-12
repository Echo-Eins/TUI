use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::time::timeout;

use crate::integrations::PowerShellExecutor;
use crate::utils::parse_json_array;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskAnalyzerData {
    pub drives: Vec<AnalyzedDrive>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzedDrive {
    pub letter: String,
    pub name: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub root_folders: Vec<RootFolderInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootFolderInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderContents {
    pub subfolders: Vec<RootFolderInfo>,
    pub files: Vec<FileInfo>,
    pub file_count: usize,
    pub folder_count: usize,
    pub extension_counts: std::collections::HashMap<String, usize>,
}

pub struct DiskAnalyzerMonitor {
    ps: PowerShellExecutor,
    es_executable: String,
    max_results: usize,
    timeout: Duration,
}

const LOGICAL_DRIVES_SCRIPT: &str = r#"
    try {
        $drives = Get-CimInstance Win32_LogicalDisk -ErrorAction Stop |
            Where-Object { $_.DriveType -eq 3 }

        $result = foreach ($drive in $drives) {
            [PSCustomObject]@{
                Letter = $drive.DeviceID
                Name = if ($drive.VolumeName) { $drive.VolumeName } else { "" }
                Total = [uint64]$drive.Size
                Free = [uint64]$drive.FreeSpace
            }
        }

        if ($result) {
            $result | ConvertTo-Json -Depth 2
        } else {
            "[]"
        }
    } catch {
        "[]"
    }
"#;

impl DiskAnalyzerMonitor {
    pub fn new(
        ps: PowerShellExecutor,
        es_executable: String,
        max_results: usize,
        timeout_seconds: u64,
    ) -> Result<Self> {
        let path = Path::new(&es_executable);
        if !path.exists() {
            anyhow::bail!("Everything CLI not found at {}", es_executable);
        }

        Ok(Self {
            ps,
            es_executable,
            max_results,
            timeout: Duration::from_secs(timeout_seconds.max(1)),
        })
    }

    pub async fn collect_data(&self) -> Result<DiskAnalyzerData> {
        #[cfg(target_os = "linux")]
        {
            anyhow::bail!("Disk analyzer is only supported on Windows");
        }

        #[cfg(not(target_os = "linux"))]
        {
            return self.collect_data_windows().await;
        }
    }

    async fn collect_data_windows(&self) -> Result<DiskAnalyzerData> {
        let drives: Vec<DriveSample> = parse_json_array(
            self.ps
                .execute(LOGICAL_DRIVES_SCRIPT)
                .await
                .context("Failed to query logical drives")?
                .as_str(),
        )
        .context("Failed to parse logical drives")?;

        if drives.is_empty() {
            return Ok(DiskAnalyzerData { drives: Vec::new() });
        }

        let mut results = Vec::new();

        for drive in drives {
            let drive_root = normalize_drive_root(&drive.Letter);
            let mut root_folders = Vec::new();
            let mut error = None;

            match self.query_root_folders(&drive_root).await {
                Ok(mut folders) => {
                    folders.sort_by(|a, b| b.size.cmp(&a.size));
                    if self.max_results > 0 && folders.len() > self.max_results {
                        folders.truncate(self.max_results);
                    }
                    root_folders = folders;
                }
                Err(e) => {
                    error = Some(e.to_string());
                }
            }

            let total = drive.Total.unwrap_or(0);
            let free = drive.Free.unwrap_or(0);
            let used = total.saturating_sub(free);

            results.push(AnalyzedDrive {
                letter: drive.Letter,
                name: drive.Name.unwrap_or_default(),
                total,
                used,
                free,
                root_folders,
                error,
            });
        }

        Ok(DiskAnalyzerData { drives: results })
    }

    async fn query_root_folders(&self, drive_root: &str) -> Result<Vec<RootFolderInfo>> {
        let count = self.max_results.to_string();
        let mut args = vec![
            "-parent",
            drive_root,
            "/ad",
            "-size",
            "-json",
            "-no-result-error",
            "-sort",
            "size-descending",
        ];

        if self.max_results > 0 {
            args.push("-count");
            args.push(&count);
        }

        let output = self
            .run_everything(&args)
            .await
            .context("Failed to query Everything CLI")?;

        Ok(parse_everything_output(&output, drive_root))
    }

    /// Query subfolders and file counts for a specific folder path
    pub async fn query_folder_contents(
        &self,
        folder_path: &str,
        track_extensions: &[String],
    ) -> Result<FolderContents> {
        let normalized_path = normalize_folder_path(folder_path);

        // Query subfolders with sizes
        let subfolder_args = vec![
            "-parent",
            &normalized_path,
            "/ad",
            "-size",
            "-json",
            "-no-result-error",
            "-sort",
            "size-descending",
        ];

        let subfolder_output = self
            .run_everything(&subfolder_args)
            .await
            .context("Failed to query subfolders")?;

        let subfolders = parse_everything_output(&subfolder_output, &normalized_path);
        let folder_count = subfolders.len();

        // Query files with sizes
        let file_args = vec![
            "-parent",
            &normalized_path,
            "/af",
            "-size",
            "-json",
            "-no-result-error",
            "-sort",
            "size-descending",
        ];

        let file_output = self
            .run_everything(&file_args)
            .await
            .context("Failed to query files")?;

        let files = parse_files_with_size(&file_output, &normalized_path);
        let file_count = files.len();

        // Count extensions if specified
        let mut extension_counts = std::collections::HashMap::new();
        if !track_extensions.is_empty() {
            for file in &files {
                let file_lower = file.name.to_lowercase();
                for ext in track_extensions {
                    let ext_lower = ext.to_lowercase();
                    let ext_with_dot = if ext_lower.starts_with('.') {
                        ext_lower
                    } else {
                        format!(".{}", ext_lower)
                    };
                    if file_lower.ends_with(&ext_with_dot) {
                        *extension_counts.entry(ext.clone()).or_insert(0) += 1;
                    }
                }
            }
        }

        Ok(FolderContents {
            subfolders,
            files,
            file_count,
            folder_count,
            extension_counts,
        })
    }

    /// Get the path to Everything executable
    pub fn es_executable(&self) -> &str {
        &self.es_executable
    }

    async fn run_everything(&self, args: &[&str]) -> Result<String> {
        let mut child = Command::new(&self.es_executable)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn Everything CLI")?;

        let stdout = child
            .stdout
            .take()
            .context("Failed to capture Everything stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("Failed to capture Everything stderr")?;

        let stdout_handle = tokio::spawn(read_to_end(stdout));
        let stderr_handle = tokio::spawn(read_to_end(stderr));

        let status = match timeout(self.timeout, child.wait()).await {
            Ok(result) => result.context("Failed to wait for Everything CLI")?,
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                stdout_handle.abort();
                stderr_handle.abort();
                anyhow::bail!("Everything CLI timed out after {}s", self.timeout.as_secs());
            }
        };

        let stdout = stdout_handle
            .await
            .context("Failed to read Everything stdout")??;
        let stderr = stderr_handle
            .await
            .context("Failed to read Everything stderr")??;

        let stdout = decode_bytes(&stdout);
        let stderr = decode_bytes(&stderr);

        if !status.success() {
            if stderr.trim().is_empty() {
                anyhow::bail!("Everything CLI failed with empty stderr");
            }
            anyhow::bail!("Everything CLI failed: {}", stderr.trim());
        }

        Ok(stdout)
    }
}

async fn read_to_end<R>(mut reader: R) -> Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .await
        .context("Failed to read Everything output")?;
    Ok(buf)
}

fn decode_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16(bytes, true);
    }

    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16(bytes, false);
    }

    if bytes.iter().skip(1).take(8).any(|b| *b == 0) {
        let decoded = decode_utf16(bytes, true);
        if !decoded.is_empty() {
            return decoded;
        }
    }

    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }

    #[cfg(windows)]
    {
        if let Some(decoded) = decode_with_system_codepage(bytes) {
            return decoded;
        }
    }

    String::from_utf8_lossy(bytes).to_string()
}

fn decode_utf16(bytes: &[u8], le: bool) -> String {
    let mut u16_buf = Vec::with_capacity(bytes.len() / 2);
    let mut idx = 0;
    while idx + 1 < bytes.len() {
        let pair = [bytes[idx], bytes[idx + 1]];
        let value = if le {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        };
        u16_buf.push(value);
        idx += 2;
    }
    String::from_utf16_lossy(&u16_buf)
}

#[cfg(windows)]
fn decode_with_system_codepage(bytes: &[u8]) -> Option<String> {
    use windows_sys::Win32::Globalization::{GetACP, GetOEMCP};
    use windows_sys::Win32::System::Console::GetConsoleOutputCP;

    let mut candidates = Vec::new();
    let console_cp = unsafe { GetConsoleOutputCP() };
    let oem_cp = unsafe { GetOEMCP() };
    let ansi_cp = unsafe { GetACP() };

    for cp in [console_cp, oem_cp, ansi_cp] {
        if let Some(encoding) = encoding_for_codepage(cp) {
            if !candidates.contains(&encoding) {
                candidates.push(encoding);
            }
        }
    }

    for encoding in candidates {
        let (text, _, _) = encoding.decode(bytes);
        if !text.is_empty() {
            return Some(text.into_owned());
        }
    }

    None
}

#[cfg(windows)]
fn encoding_for_codepage(codepage: u32) -> Option<&'static encoding_rs::Encoding> {
    match codepage {
        65001 => Some(encoding_rs::UTF_8),
        866 => Some(encoding_rs::IBM866),
        1251 => Some(encoding_rs::WINDOWS_1251),
        1252 => Some(encoding_rs::WINDOWS_1252),
        932 => Some(encoding_rs::SHIFT_JIS),
        936 => Some(encoding_rs::GBK),
        949 => Some(encoding_rs::EUC_KR),
        950 => Some(encoding_rs::BIG5),
        _ => None,
    }
}

fn normalize_drive_root(letter: &str) -> String {
    let trimmed = letter.trim_end_matches('\\');
    format!("{}\\", trimmed)
}

fn parse_everything_output(output: &str, drive_root: &str) -> Vec<RootFolderInfo> {
    let trimmed = output.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let entries = parse_everything_json(value, drive_root);
        if !entries.is_empty() {
            return entries;
        }
    }

    trimmed
        .lines()
        .filter_map(|line| parse_size_path_line(line))
        .filter(|(_, path)| is_root_child(path, drive_root))
        .map(|(size, path)| RootFolderInfo {
            name: folder_name(&path),
            path,
            size,
        })
        .collect()
}

fn parse_everything_json(
    value: serde_json::Value,
    drive_root: &str,
) -> Vec<RootFolderInfo> {
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(map) => {
            if let Some(results) = find_value_ci(&map, &["results", "items"]) {
                if let serde_json::Value::Array(items) = results {
                    items.clone()
                } else {
                    Vec::new()
                }
            } else {
                vec![serde_json::Value::Object(map)]
            }
        }
        _ => Vec::new(),
    };

    let mut entries = Vec::new();

    for item in items {
        let serde_json::Value::Object(map) = item else { continue };

        let mut path = get_string_ci(&map, &["path", "full_path", "fullpath", "full"]);
        let filename = get_string_ci(&map, &["filename"]);
        let mut name = get_string_ci(&map, &["name"]);

        if path.is_none() {
            if let Some(filename) = filename.clone() {
                if looks_like_full_path(&filename) {
                    path = Some(filename);
                }
            }
        }

        if path.is_none() {
            if let (Some(parent), Some(name_value)) = (
                get_string_ci(&map, &["parent", "directory"]),
                name.clone(),
            ) {
                let parent = parent.trim_end_matches('\\');
                path = Some(format!("{}\\{}", parent, name_value));
            }
        }

        let Some(path) = path else { continue };
        if !is_root_child(&path, drive_root) {
            continue;
        }

        if name.is_none() {
            name = Some(folder_name(&path));
        }

        let size = get_u64_ci(&map, &["size", "filesize"]).unwrap_or(0);

        entries.push(RootFolderInfo {
            name: name.unwrap_or_else(|| folder_name(&path)),
            path,
            size,
        });
    }

    entries
}

fn parse_size_path_line(line: &str) -> Option<(u64, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut iter = trimmed.split_whitespace();
    let first = iter.next()?;
    let rest = trimmed[first.len()..].trim();

    if let Some(size) = parse_numeric(first) {
        if !rest.is_empty() {
            return Some((size, trim_quotes(rest)));
        }
    }

    let mut rev_iter = trimmed.split_whitespace().rev();
    let last = rev_iter.next()?;
    let rest_rev = trimmed[..trimmed.len().saturating_sub(last.len())].trim();
    if let Some(size) = parse_numeric(last) {
        if !rest_rev.is_empty() {
            return Some((size, trim_quotes(rest_rev)));
        }
    }

    None
}

fn parse_numeric(input: &str) -> Option<u64> {
    let digits: String = input.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

fn trim_quotes(input: &str) -> String {
    input.trim_matches('"').to_string()
}

fn folder_name(path: &str) -> String {
    let trimmed = path.trim_end_matches('\\');
    Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(trimmed)
        .to_string()
}

fn looks_like_full_path(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() < 3 {
        return false;
    }

    let has_drive = trimmed.as_bytes().get(1) == Some(&b':')
        && trimmed.as_bytes().get(2) == Some(&b'\\');
    let is_unc = trimmed.starts_with("\\\\");

    has_drive || is_unc
}

fn is_root_child(path: &str, drive_root: &str) -> bool {
    let normalized_root = drive_root.replace('/', "\\").to_ascii_lowercase();
    let mut normalized_path = path.replace('/', "\\");
    while normalized_path.ends_with('\\') {
        normalized_path.pop();
    }
    let normalized_path = normalized_path.to_ascii_lowercase();

    if !normalized_path.starts_with(&normalized_root) {
        return false;
    }

    let rest = normalized_path[normalized_root.len()..].trim_start_matches('\\');
    !rest.is_empty() && !rest.contains('\\')
}

fn find_value_ci<'a>(
    map: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a serde_json::Value> {
    map.iter()
        .find(|(k, _)| keys.iter().any(|key| k.eq_ignore_ascii_case(key)))
        .map(|(_, v)| v)
}

fn get_string_ci(
    map: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    find_value_ci(map, keys).and_then(|value| match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn get_u64_ci(
    map: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<u64> {
    find_value_ci(map, keys).and_then(|value| match value {
        serde_json::Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|v| v as u64)),
        serde_json::Value::String(s) => parse_numeric(s),
        _ => None,
    })
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct DriveSample {
    Letter: String,
    Name: Option<String>,
    Total: Option<u64>,
    Free: Option<u64>,
}

fn normalize_folder_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('\\').trim_end_matches('/');
    format!("{}\\", trimmed)
}

fn parse_file_list(output: &str) -> Vec<String> {
    let trimmed = output.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // Try to parse as JSON first
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return parse_file_list_json(value);
    }

    // Fallback to line-based parsing
    trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .collect()
}

fn parse_file_list_json(value: serde_json::Value) -> Vec<String> {
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(map) => {
            if let Some(results) = find_value_ci(&map, &["results", "items"]) {
                if let serde_json::Value::Array(items) = results {
                    items.clone()
                } else {
                    Vec::new()
                }
            } else {
                vec![serde_json::Value::Object(map)]
            }
        }
        _ => Vec::new(),
    };

    items
        .into_iter()
        .filter_map(|item| {
            if let serde_json::Value::Object(map) = item {
                get_string_ci(&map, &["filename", "name", "path"])
            } else if let serde_json::Value::String(s) = item {
                Some(s)
            } else {
                None
            }
        })
        .collect()
}

fn parse_files_with_size(output: &str, parent_path: &str) -> Vec<FileInfo> {
    let trimmed = output.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return parse_files_json(value, parent_path);
    }

    // Fallback to line-based parsing
    trimmed
        .lines()
        .filter_map(|line| parse_size_path_line(line))
        .map(|(size, path)| {
            let name = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&path)
                .to_string();
            FileInfo { name, path, size }
        })
        .collect()
}

fn parse_files_json(value: serde_json::Value, parent_path: &str) -> Vec<FileInfo> {
    let items = match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(map) => {
            if let Some(results) = find_value_ci(&map, &["results", "items"]) {
                if let serde_json::Value::Array(items) = results {
                    items.clone()
                } else {
                    Vec::new()
                }
            } else {
                vec![serde_json::Value::Object(map)]
            }
        }
        _ => Vec::new(),
    };

    items
        .into_iter()
        .filter_map(|item| {
            let serde_json::Value::Object(map) = item else { return None };

            let filename = get_string_ci(&map, &["filename", "name"])?;
            let size = get_u64_ci(&map, &["size", "filesize"]).unwrap_or(0);

            let path = get_string_ci(&map, &["path", "full_path", "fullpath"])
                .unwrap_or_else(|| {
                    let parent = parent_path.trim_end_matches('\\');
                    format!("{}\\{}", parent, filename)
                });

            Some(FileInfo {
                name: filename,
                path,
                size,
            })
        })
        .collect()
}
