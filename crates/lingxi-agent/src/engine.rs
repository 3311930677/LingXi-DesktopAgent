//! The agent engine: orchestrates the think → act → observe loop.

use crate::action::AgentAction;
use crate::backend::AgentBackend;
use crate::error::AgentError;
use crate::session::Session;
use lingxi_tools::{ConfirmGate, ToolContext, ToolRegistry};
use std::sync::Arc;

/// The agent engine. Holds a backend (model) and a tool registry.
///
/// The main method is [`AgentEngine::run` which executes the ReAct loop:
/// ask the model → if it wants a tool, execute it → feed the result back →
/// repeat until the model replies or the step limit is reached.
pub struct AgentEngine {
    backend: Box<dyn AgentBackend>,
    registry: std::sync::Arc<ToolRegistry>,
    max_steps: usize,
}

impl AgentEngine {
    pub fn new(backend: Box<dyn AgentBackend>, registry: std::sync::Arc<ToolRegistry>) -> Self {
        Self {
            backend,
            registry,
            max_steps: 10,
        }
    }

    pub fn with_max_steps(mut self, max: usize) -> Self {
        self.max_steps = max;
        self
    }

    /// Run one round of conversation. The user input is added to the session,
    /// then the engine loops until the model replies or asks a question.
    pub async fn run(
        &self,
        user_input: &str,
        session: &mut Session,
        confirm: Arc<dyn ConfirmGate>,
    ) -> Result<String, AgentError> {
        session.push_user(user_input);

        for _step in 0..self.max_steps {
            let action = self
                .backend
                .step(&session.messages, &self.registry.enabled_schemas())
                .await?;

            match action {
                AgentAction::Reply(text) => {
                    session.push_assistant(&text);
                    return Ok(text);
                }
                AgentAction::AskUser(question) => {
                    session.push_assistant(&question);
                    return Ok(question);
                }
                AgentAction::CallTool {
                    name,
                    arguments,
                    thought,
                } => {
                    // Log the assistant's thought + tool call.
                    if let Some(t) = &thought {
                        session.push_assistant(t);
                    }

                    let ctx = ToolContext {
                        session_id: session.id.clone(),
                        working_dir: session.working_dir.clone(),
                        confirm: confirm.clone(),
                    };

                    let result = match self.registry.execute(&name, arguments, &ctx).await {
                        Some(r) => r,
                        None => lingxi_tools::ToolResult::err(format!("未知工具: {name}")),
                    };

                    session.push_tool_result(&name, &result.output);
                }
            }
        }

        Err(AgentError::MaxStepsExceeded(self.max_steps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::AgentAction;
    use crate::backend::AgentBackend;
    use crate::session::{Message, Session};
    use async_trait::async_trait;
    use lingxi_tools::context::AutoConfirm;
    use lingxi_tools::{Tool, ToolResult, ToolSchema};
    use serde_json::json;
    use std::sync::Arc;

    /// A mock backend that replays a scripted sequence of actions.
    struct MockBackend {
        actions: std::sync::Mutex<Vec<AgentAction>>,
    }

    impl MockBackend {
        fn new(actions: Vec<AgentAction>) -> Self {
            Self {
                actions: std::sync::Mutex::new(actions),
            }
        }
    }

    #[async_trait]
    impl AgentBackend for MockBackend {
        async fn step(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
        ) -> Result<AgentAction, AgentError> {
            let mut actions = self.actions.lock().unwrap();
            if actions.is_empty() {
                Ok(AgentAction::Reply("（无更多动作）".into()))
            } else {
                Ok(actions.remove(0))
            }
        }
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: "echo".into(),
                description: "Echoes text back.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"]
                }),
            }
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &lingxi_tools::ToolContext,
        ) -> ToolResult {
            let text = params
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("(empty)");
            ToolResult::ok(text.to_string())
        }
    }

    #[tokio::test]
    async fn agent_calls_tool_then_replies() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));

        let backend = MockBackend::new(vec![
            AgentAction::CallTool {
                name: "echo".into(),
                arguments: json!({"text": "hello world"}),
                thought: Some("我需要回显这段文字".into()),
            },
            AgentAction::Reply("工具返回了: hello world".into()),
        ]);

        let engine = AgentEngine::new(Box::new(backend), Arc::new(registry));
        let mut session = Session::new("test", ".");

        let reply = engine
            .run("echo hello world", &mut session, Arc::new(AutoConfirm) as Arc<dyn ConfirmGate>)
            .await
            .unwrap();

        assert_eq!(reply, "工具返回了: hello world");
        // user + thought + tool_result + assistant_reply = 4 messages
        assert_eq!(session.messages.len(), 4);
    }

    #[tokio::test]
    async fn agent_replies_directly() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));

        let backend = MockBackend::new(vec![AgentAction::Reply("直接回答".into())]);
        let engine = AgentEngine::new(Box::new(backend), Arc::new(registry));
        let mut session = Session::new("test", ".");

        let reply = engine
            .run("你好", &mut session, Arc::new(AutoConfirm) as Arc<dyn ConfirmGate>)
            .await
            .unwrap();

        assert_eq!(reply, "直接回答");
        // user + assistant_reply = 2 messages
        assert_eq!(session.messages.len(), 2);
    }

    #[tokio::test]
    async fn agent_exceeds_max_steps() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));

        // Every step calls the tool, never replies.
        let backend = MockBackend::new(
            (0..20)
                .map(|_| AgentAction::CallTool {
                    name: "echo".into(),
                    arguments: json!({"text": "loop"}),
                    thought: None,
                })
                .collect(),
        );

        let engine = AgentEngine::new(Box::new(backend), Arc::new(registry)).with_max_steps(3);
        let mut session = Session::new("test", ".");

        let result = engine.run("loop", &mut session, Arc::new(AutoConfirm) as Arc<dyn ConfirmGate>).await;
        assert!(matches!(result, Err(AgentError::MaxStepsExceeded(3))));
    }
}
