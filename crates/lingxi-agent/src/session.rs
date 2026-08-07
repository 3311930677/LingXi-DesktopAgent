//! Session: conversation history and tool-call log.

use crate::action::ToolCall;
use serde::{Deserialize, Serialize};

/// Message role in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// A tool result message (OpenAI format: role = "tool").
    Tool,
}

/// A single message in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Tool calls attached to an assistant message (for function calling).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<ToolCall>,
    /// Name of the tool that produced this message (for role = Tool).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_name: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: vec![],
            tool_name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: vec![],
            tool_name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: vec![],
            tool_name: None,
        }
    }

    pub fn tool_result(name: impl Into<String>, result: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: result.into(),
            tool_calls: vec![],
            tool_name: Some(name.into()),
        }
    }
}

/// A conversation session with history.
pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
    /// Working directory for file-based tools.
    pub working_dir: std::path::PathBuf,
}

impl Session {
    pub fn new(id: impl Into<String>, working_dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            id: id.into(),
            messages: vec![],
            working_dir: working_dir.into(),
        }
    }

    pub fn push_user(&mut self, content: impl Into<String>) {
        self.messages.push(Message::user(content));
    }

    pub fn push_assistant(&mut self, content: impl Into<String>) {
        self.messages.push(Message::assistant(content));
    }

    pub fn push_assistant_with_tools(&mut self, content: impl Into<String>, calls: Vec<ToolCall>) {
        self.messages.push(Message {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: calls,
            tool_name: None,
        });
    }

    pub fn push_tool_result(&mut self, tool_name: &str, result: &str) {
        self.messages.push(Message::tool_result(tool_name, result));
    }

    /// All messages as serde_json values, suitable for the OpenAI API.
    pub fn messages_json(&self) -> Vec<serde_json::Value> {
        self.messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                let mut obj = serde_json::json!({
                    "role": role,
                    "content": m.content,
                });
                if let Some(ref name) = m.tool_name {
                    obj["name"] = serde_json::Value::String(name.clone());
                }
                obj
            })
            .collect()
    }
}
