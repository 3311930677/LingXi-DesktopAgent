//! Shell command execution tool.

use async_trait::async_trait;
use lingxi_tools::schema::{ToolResult, ToolSchema};
use lingxi_tools::{RiskLevel, Tool, ToolContext};
use serde_json::json;
use std::process::Command;

/// Execute a shell command and return its stdout/stderr.
pub struct RunCommandTool;

#[async_trait]
impl Tool for RunCommandTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "run_command".into(),
            description: "执行系统命令并返回输出。此工具具有风险，每次执行前需要用户确认。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "要执行的命令"},
                    "cwd": {"type": "string", "description": "工作目录（可选）"}
                },
                "required": ["command"]
            }),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Dangerous
    }

    async fn execute(&self, params: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let command = match params.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::err("缺少 command 参数"),
        };

        let cwd = params
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        // Determine the shell based on the platform. Own the strings so the
        // spawn_blocking closure is 'static.
        let (program, args): (&'static str, Vec<String>) = if cfg!(target_os = "windows") {
            ("cmd", vec!["/C".into(), command.into()])
        } else {
            ("sh", vec!["-c".into(), command.into()])
        };

        let result = tokio::task::spawn_blocking(move || {
            Command::new(program).args(&args).current_dir(&cwd).output()
        })
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);

                if exit_code == 0 {
                    let text = if stdout.is_empty() && stderr.is_empty() {
                        "命令执行成功（无输出）".to_string()
                    } else if stderr.is_empty() {
                        stdout
                    } else {
                        format!("stdout:\n{stdout}\nstderr:\n{stderr}")
                    };
                    ToolResult::ok(text)
                } else {
                    ToolResult::err(format!(
                        "命令退出码 {exit_code}\nstdout:\n{stdout}\nstderr:\n{stderr}"
                    ))
                }
            }
            Ok(Err(e)) => ToolResult::err(format!("启动命令失败: {e}")),
            Err(e) => ToolResult::err(format!("任务调度失败: {e}")),
        }
    }
}
