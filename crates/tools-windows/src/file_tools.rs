//! File operation tools: read, write, list directory.

use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;
use std::path::Path;

/// Read a text file's contents.
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "read_file".into(),
            description: "读取文本文件的内容。路径可以是绝对路径或相对于工作目录的路径。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "文件路径"}
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err("缺少 path 参数"),
        };

        let full = resolve_path(path, &ctx.working_dir);
        match tokio::fs::read_to_string(&full).await {
            Ok(content) => {
                let truncated = if content.len() > 50_000 {
                    format!("{}\n\n...(文件已截断，共 {} 字节)", &content[..50_000], content.len())
                } else {
                    content
                };
                ToolResult::ok(truncated)
            }
            Err(e) => ToolResult::err(format!("读取文件失败: {e}")),
        }
    }
}

/// Write text to a file (creates or overwrites).
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_file".into(),
            description: "将文本写入文件（覆盖已有内容）。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "文件路径"},
                    "content": {"type": "string", "description": "要写入的内容"}
                },
                "required": ["path", "content"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Dangerous
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err("缺少 path 参数"),
        };
        let content = match params.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::err("缺少 content 参数"),
        };

        let full = resolve_path(path, &ctx.working_dir);
        match tokio::fs::write(&full, content).await {
            Ok(()) => ToolResult::ok(format!("已写入 {}", full.display())),
            Err(e) => ToolResult::err(format!("写入文件失败: {e}")),
        }
    }
}

/// List the contents of a directory.
pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "list_dir".into(),
            description: "列出目录中的文件和子目录。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "目录路径（默认为工作目录）"}
                }
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let full = resolve_path(path, &ctx.working_dir);
        let mut entries = match tokio::fs::read_dir(&full).await {
            Ok(e) => e,
            Err(e) => return ToolResult::err(format!("读取目录失败: {e}")),
        };

        let mut items = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry
                .file_type()
                .await
                .map(|t| t.is_dir())
                .unwrap_or(false);
            items.push(json!({"name": name, "is_dir": is_dir}));
        }

        items.sort_by(|a, b| {
            let a_dir = a["is_dir"].as_bool().unwrap_or(false);
            let b_dir = b["is_dir"].as_bool().unwrap_or(false);
            b_dir.cmp(&a_dir).then_with(|| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["name"].as_str().unwrap_or(""))
            })
        });

        ToolResult::ok_with_data(
            format!("共 {} 项", items.len()),
            json!(items),
        )
    }
}

/// Resolve a path relative to the working directory if not absolute.
fn resolve_path(path: &str, working_dir: &Path) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        working_dir.join(p)
    }
}
