//! Persistent reminder queue. Notification delivery is owned by the host UI.

use crate::schema::{ToolResult, ToolSchema};
use crate::{RiskLevel, Tool, ToolContext};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_MESSAGE_CHARS: usize = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Reminder {
    id: String,
    due_at: i64,
    message: String,
    cancelled: bool,
}

pub struct SetReminderTool;
pub struct ListRemindersTool;
pub struct CancelReminderTool;

#[async_trait]
impl Tool for SetReminderTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "set_reminder".into(),
            description:
                "设置一个持久化提醒。使用 Unix 秒时间戳 due_at，或使用 delay_seconds 设置相对时间。"
                    .into(),
            parameters: json!({"type":"object","properties":{"due_at":{"type":"integer","description":"到期时间的 Unix 秒时间戳"},"delay_seconds":{"type":"integer","minimum":1,"description":"从现在起延迟的秒数"},"message":{"type":"string","description":"提醒内容"}},"required":["message"]}),
        }
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let message = match params.get("message").and_then(|v| v.as_str()) {
            Some(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => return ToolResult::err("message 不能为空"),
        };
        if message.chars().count() > MAX_MESSAGE_CHARS {
            return ToolResult::err(format!("message 最多支持 {MAX_MESSAGE_CHARS} 个字符"));
        }
        let now = unix_now();
        let due_at = match (
            params.get("due_at").and_then(|v| v.as_i64()),
            params.get("delay_seconds").and_then(|v| v.as_i64()),
        ) {
            (Some(value), _) if value > now => value,
            (_, Some(value)) if value > 0 => now.saturating_add(value),
            _ => return ToolResult::err("必须提供未来的 due_at，或正数 delay_seconds"),
        };
        let reminder = Reminder {
            id: format!("r-{}", now_nanos()),
            due_at,
            message,
            cancelled: false,
        };
        if let Err(error) = append(&path(&ctx.working_dir), &reminder) {
            return ToolResult::err(format!("保存提醒失败: {error}"));
        }
        ToolResult::ok_with_data(
            format!(
                "提醒已设置，ID: {}，到期时间 Unix: {}",
                reminder.id, reminder.due_at
            ),
            json!(reminder),
        )
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Moderate
    }
}

#[async_trait]
impl Tool for ListRemindersTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "list_reminders".into(),
            description: "列出尚未取消的提醒，可筛选已到期提醒。".into(),
            parameters: json!({"type":"object","properties":{"include_expired":{"type":"boolean"}}}),
        }
    }
    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let include_expired = params
            .get("include_expired")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let now = unix_now();
        match read_all(&path(&ctx.working_dir)) {
            Ok(items) => {
                let items: Vec<_> = items
                    .into_iter()
                    .filter(|r| !r.cancelled && (include_expired || r.due_at > now))
                    .collect();
                ToolResult::ok_with_data(format!("当前有 {} 条提醒", items.len()), json!(items))
            }
            Err(error) => ToolResult::err(format!("读取提醒失败: {error}")),
        }
    }
}

#[async_trait]
impl Tool for CancelReminderTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "cancel_reminder".into(),
            description: "按 ID 取消一个提醒。".into(),
            parameters: json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}),
        }
    }
    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let id = match params.get("id").and_then(|v| v.as_str()) {
            Some(value) if !value.is_empty() => value,
            _ => return ToolResult::err("id 不能为空"),
        };
        let file = path(&ctx.working_dir);
        let mut items = match read_all(&file) {
            Ok(items) => items,
            Err(error) => return ToolResult::err(format!("读取提醒失败: {error}")),
        };
        let Some(item) = items
            .iter_mut()
            .find(|item| item.id == id && !item.cancelled)
        else {
            return ToolResult::err(format!("未找到提醒: {id}"));
        };
        item.cancelled = true;
        if let Err(error) = rewrite(&file, &items) {
            return ToolResult::err(format!("更新提醒失败: {error}"));
        }
        ToolResult::ok(format!("已取消提醒 {id}"))
    }
}

fn path(root: &Path) -> PathBuf {
    root.join(".lingxi").join("reminders.jsonl")
}
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
fn append(file: &Path, reminder: &Reminder) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut out = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut out, reminder).map_err(|e| e.to_string())?;
    out.write_all(b"\n").map_err(|e| e.to_string())
}
fn read_all(file: &Path) -> Result<Vec<Reminder>, String> {
    if !file.exists() {
        return Ok(Vec::new());
    }
    BufReader::new(fs::File::open(file).map_err(|e| e.to_string())?)
        .lines()
        .map(|line| {
            let line = line.map_err(|e| e.to_string())?;
            serde_json::from_str(&line).map_err(|e| e.to_string())
        })
        .collect()
}
fn rewrite(file: &Path, items: &[Reminder]) -> Result<(), String> {
    let temp = file.with_extension("tmp");
    let mut out = fs::File::create(&temp).map_err(|e| e.to_string())?;
    for item in items {
        serde_json::to_writer(&mut out, item).map_err(|e| e.to_string())?;
        out.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())?;
    fs::rename(temp, file).map_err(|e| e.to_string())
}
