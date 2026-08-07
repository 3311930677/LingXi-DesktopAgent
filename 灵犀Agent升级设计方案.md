# 灵犀 Agent 化升级设计方案

> 版本：v2.0 设计稿
> 日期：2026-08-07
> 目标：将灵犀从"选区辅助输入工具"升级为"有丰富工具能力的 AI 桌面 Agent"

---

## 一、设计理念

### 1.1 从"单次变换"到"自主完成任务"

**当前**：用户选中文字 → 触发热键 → 模型单次变换 → 写回。每次只做一件事，无记忆、无规划、无工具。

**目标**：用户用自然语言描述任务 → Agent 自主规划步骤 → 调用工具执行 → 观察结果 → 继续或完成。支持多轮对话、任务记忆、工具组合。

### 1.2 核心原则

1. **工具优先**：Agent 的能力边界 = 工具集边界。每个能力都是一个可注册、可描述、可调度的工具
2. **人在回路**：危险操作（发消息、删文件、执行命令）必须用户确认
3. **泛化现有能力**：选区读写、QQ 集成、键盘模拟不是特例，而是通用工具
4. **与 OwO 分工**：OwO 管系统输入法，灵犀管跨应用智能 Agent

---

## 二、目标架构总览

```
┌──────────────────────────────────────────────────────────┐
│                     用户交互层                             │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐  │
│  │ 对话面板  │  │ 选区热键  │  │ 全局命令  │  │ 后台任务  │  │
│  │ (主交互)  │  │ (保留)    │  │ (语音/键) │  │ (定时/触发)│  │
│  └─────┬────┘  └─────┬────┘  └─────┬────┘  └─────┬────┘  │
├────────┼─────────────┼─────────────┼─────────────┼───────┤
│        └─────────────┴─────────────┴─────────────┘       │
│                         │                                 │
│              ┌──────────▼──────────┐                     │
│              │   Agent 引擎 (新)     │                     │
│              │  ┌─────────────────┐ │                     │
│              │  │ 对话循环(ReAct)  │ │                     │
│              │  │ 任务规划与拆解   │ │                     │
│              │  │ 上下文管理      │ │                     │
│              │  │ 确认门控        │ │                     │
│              │  └────────┬────────┘ │                     │
│              └───────────┼──────────┘                     │
├──────────────────────────┼───────────────────────────────┤
│                          │                                │
│              ┌───────────▼──────────┐                     │
│              │   工具注册与调度 (新)   │                     │
│              │  ToolRegistry + Router │                    │
│              └───────────┬──────────┘                     │
│           ┌──────────────┼──────────────┐                │
│           ▼              ▼              ▼                │
│  ┌─────────────┐ ┌──────────────┐ ┌──────────────┐      │
│  │ 系统工具集   │ │ 应用交互工具集 │ │ 信息工具集    │      │
│  │ - 文件操作   │ │ - UIA 读写    │ │ - 网页搜索    │      │
│  │ - 进程管理   │ │ - 键盘模拟    │ │ - 天气/时间   │      │
│  │ - 剪贴板     │ │ - 鼠标模拟    │ │ - 计算/翻译   │      │
│  │ - 截图       │ │ - 窗口管理    │ │ - 知识检索    │      │
│  │ - 命令执行   │ │ - 应用启动    │ │ - 日程/提醒   │      │
│  └─────────────┘ └──────────────┘ └──────────────┘      │
├──────────────────────────────────────────────────────────┤
│                     基础设施层                             │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐  │
│  │ 模型后端  │  │ 持久化    │  │ 事件总线  │  │ 权限系统  │  │
│  │(云端+本地) │  │(会话/配置) │  │(工具事件) │  │(确认门控) │  │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘  │
└──────────────────────────────────────────────────────────┘
```

---

## 三、新 Crate 划分

### 3.1 现有 → 目标

| 现有 | 目标 | 变化 |
|------|------|------|
| `assistant-core` | 保留，泛化为 `core` | InputAdapter 保留，新增 Tool trait |
| `assistant-inference` | 扩展，支持 tool use | 新增 `AgentBackend` trait |
| `assistant-windows` | 保留，能力暴露为工具 | 新增 `tools/` 子模块 |
| `apps/overlay` | 重构 UI | 新增对话面板、工具面板 |
| — | **新增 `crates/agent`** | Agent 引擎、对话循环、任务规划 |
| — | **新增 `crates/tools`** | 工具 trait + 注册表 + 各工具实现 |

### 3.2 新增 crate：`crates/agent`

**职责**：Agent 引擎核心，与平台无关。

```
crates/agent/
├── Cargo.toml          (依赖: assistant-core, serde, serde_json)
├── src/
│   ├── lib.rs           (pub use)
│   ├── engine.rs        (对话循环：think → act → observe → repeat)
│   ├── planner.rs       (任务拆解：自然语言 → 工具调用序列)
│   ├── context.rs       (对话上下文/记忆管理)
│   ├── confirm.rs       (确认门控 trait + 默认策略)
│   ├── session.rs       (会话状态：消息历史、工具调用记录)
│   └── error.rs         (AgentError)
```

### 3.3 新增 crate：`crates/tools`

**职责**：工具抽象 + 注册表 + 跨平台工具实现。

```
crates/tools/
├── Cargo.toml          (依赖: assistant-core, serde, serde_json, async-trait)
├── src/
│   ├── lib.rs           (pub use Tool trait, ToolRegistry, ToolContext)
│   ├── registry.rs      (工具注册表 + 按 schema 路由)
│   ├── schema.rs        (工具参数 schema 定义，JSON Schema 格式)
│   └── builtin/
│       ├── mod.rs
│       ├── text.rs       (文本处理：润色、翻译、摘要、格式化)
│       ├── calc.rs       (计算器、单位换算、时间)
│       └── search.rs     (网页搜索接口定义，实现可注入)
```

### 3.4 新增 crate：`crates/tools-windows`

**职责**：Windows 平台工具实现，依赖 `assistant-windows` 已有能力。

```
crates/tools-windows/
├── Cargo.toml          (依赖: tools, assistant-windows)
├── src/
│   ├── lib.rs
│   ├── uia_tools.rs     (UIA 读写工具：读选区、写文本、列控件树)
│   ├── window_tools.rs  (窗口管理：列窗口、聚焦、移动、截图)
│   ├── app_tools.rs     (应用启动、QQ 集成、浏览器控制)
│   ├── file_tools.rs    (文件读写、搜索、批量操作)
│   ├── clip_tools.rs    (剪贴板读写、历史)
│   ├── input_tools.rs   (键盘/鼠标模拟录制)
│   └── shell_tools.rs   (命令执行，需确认)
```

---

## 四、核心抽象设计

### 4.1 Tool trait（工具接口）

```rust
// crates/tools/src/lib.rs

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// 工具执行上下文，携带会话信息和权限检查回调
pub struct ToolContext<'a> {
    pub session_id: String,
    pub working_dir: std::path::PathBuf,
    pub confirm: &'a dyn ConfirmGate,
}

/// 确认门控：危险操作必须经过用户确认
pub trait ConfirmGate {
    fn confirm(&self, request: &ConfirmRequest) -> bool;
}

pub struct ConfirmRequest {
    pub tool_name: String,
    pub action_summary: String,
    pub risk_level: RiskLevel,
    pub params: Value,
}

#[derive(Clone, Copy, PartialEq)]
pub enum RiskLevel {
    Safe,       // 读操作、计算
    Moderate,   // 写文本、剪贴板
    Dangerous,  // 执行命令、删文件、发消息
}

/// 工具元数据，暴露给 LLM 用于 function calling
#[derive(Debug, Clone, Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// JSON Schema 格式的参数定义
    pub parameters: Value,
}

/// 工具执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    /// 返回给 LLM 的文本内容
    pub output: String,
    /// 结构化数据（可选，供 UI 可视化）
    pub data: Option<Value>,
}

/// 核心工具 trait —— 每个能力实现它
#[async_trait]
pub trait Tool: Send + Sync {
    /// 工具元数据（名称、描述、参数 schema）
    fn schema(&self) -> ToolSchema;

    /// 执行工具
    async fn execute(&self, params: Value, ctx: &ToolContext<'_>) -> ToolResult;

    /// 风险等级（默认 Safe）
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }
}
```

### 4.2 ToolRegistry（工具注册表）

```rust
// crates/tools/src/registry.rs

use std::collections::HashMap;
use std::sync::Arc;

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.schema().name;
        self.tools.insert(name, tool);
    }

    /// 返回所有工具的 schema，供 LLM function calling
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    /// 按名称路由执行
    pub async fn execute(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext<'_>,
    ) -> Option<ToolResult> {
        self.tools.get(name).map(|t| t.execute(params, ctx))
            .map(|f| futures::executor::block_on(f))  // 实际用 async
    }
}
```

### 4.3 Agent 引擎（对话循环）

```rust
// crates/agent/src/engine.rs

pub struct AgentEngine {
    backend: Box<dyn AgentBackend>,
    registry: Arc<ToolRegistry>,
    max_steps: usize,
}

/// Agent 后端 trait：支持 tool use 的模型
#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// 发送对话历史 + 可用工具，返回模型决策
    async fn step(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<AgentAction, AgentError>;
}

pub enum AgentAction {
    /// 模型决定调用工具
    CallTool { name: String, arguments: Value },
    /// 模型给出最终回复
    Reply(String),
    /// 模型请求更多信息
    AskUser(String),
}

pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

pub enum Role { System, User, Assistant, Tool }

impl AgentEngine {
    /// 执行一轮 Agent 循环
    pub async fn run(
        &self,
        user_input: &str,
        session: &mut Session,
        confirm: &dyn ConfirmGate,
    ) -> Result<String, AgentError> {
        session.push_user(user_input);

        for step in 0..self.max_steps {
            let action = self.backend
                .step(session.messages(), &self.registry.schemas())
                .await?;

            match action {
                AgentAction::Reply(text) => {
                    session.push_assistant(&text, vec![]);
                    return Ok(text);
                }
                AgentAction::AskUser(question) => {
                    session.push_assistant(&question, vec![]);
                    return Ok(question);
                }
                AgentAction::CallTool { name, arguments } => {
                    // 确认门控
                    let tool = self.registry.get(&name);
                    let risk = tool.map(|t| t.risk_level()).unwrap_or(RiskLevel::Safe);
                    if risk >= RiskLevel::Dangerous {
                        let req = ConfirmRequest {
                            tool_name: name.clone(),
                            action_summary: format!("执行 {}", name),
                            risk_level: risk,
                            params: arguments.clone(),
                        };
                        if !confirm.confirm(&req) {
                            session.push_tool_result(&name, "用户取消了此操作");
                            continue;
                        }
                    }

                    // 执行工具
                    let ctx = ToolContext {
                        session_id: session.id.clone(),
                        working_dir: session.working_dir.clone(),
                        confirm,
                    };
                    let result = self.registry.execute(&name, arguments, &ctx).await;

                    session.push_tool_call(&name, &result);
                }
            }
        }
        Err(AgentError::MaxStepsExceeded)
    }
}
```

### 4.4 AgentBackend 实现（云端 function calling）

```rust
// crates/assistant-inference/src/agent_cloud.rs

pub struct CloudAgentBackend {
    config: CloudConfig,
}

#[async_trait]
impl AgentBackend for CloudAgentBackend {
    async fn step(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<AgentAction, AgentError> {
        let payload = json!({
            "model": self.config.model,
            "messages": messages,
            "tools": tools.iter().map(|t| json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })).collect::<Vec<_>>(),
            "tool_choice": "auto",
            "stream": false,
        });
        // POST /v1/chat/completions
        let response = self.post(&payload).await?;
        let choice = &response["choices"][0]["message"];

        // 检查是否有 tool_calls
        if let Some(tool_calls) = choice["tool_calls"].as_array() {
            if !tool_calls.is_empty() {
                let call = &tool_calls[0];
                return Ok(AgentAction::CallTool {
                    name: call["function"]["name"].as_str().unwrap().to_string(),
                    arguments: serde_json::from_str(
                        call["function"]["arguments"].as_str().unwrap_or("{}")
                    ).unwrap_or(Value::Null),
                });
            }
        }

        Ok(AgentAction::Reply(
            choice["content"].as_str().unwrap_or("").to_string()
        ))
    }
}
```

---

## 五、工具清单设计

### 5.1 应用交互工具集（`tools-windows`）

| 工具名 | 描述 | 参数 | 风险 | 复用现有 |
|--------|------|------|------|----------|
| `read_selection` | 读取当前前台应用选中的文字 | — | Safe | `WindowsAdapter::capture_selection` |
| `write_text` | 向当前焦点控件写入/替换文本 | `{text, mode: "replace"\|"append"}` | Moderate | `WindowsAdapter::write_back` |
| `undo_write` | 撤销上次写入 | `{receipt_id}` | Moderate | `WindowsAdapter::undo` |
| `list_windows` | 列出所有可见窗口 | `{filter?}` | Safe | 新增 |
| `focus_window` | 聚焦指定窗口 | `{title_contains}` | Moderate | 新增 |
| `capture_screen` | 截取屏幕/窗口截图 | `{region?, window_title?}` | Safe | 新增 |
| `find_element` | 按条件查找 UIA 元素 | `{control_type?, name?, automation_id?}` | Safe | `UiaClient::descendants` |
| `click_element` | 点击 UIA 元素 | `{selector, button: "left"\|"right"}` | Moderate | `keyboard::click_at` 泛化 |
| `type_text` | 模拟键盘输入文本 | `{text}` | Moderate | `keyboard::type_unicode` |
| `send_keys` | 发送快捷键组合 | `{keys: "ctrl+c"}` | Moderate | 新增 |
| `qq_read_selection` | 读取 QQ 选中的消息 | — | Safe | `capture_qq_selection_text` |
| `qq_write_draft` | 向 QQ 聊天框写入草稿 | `{draft}` | Moderate | `qq_write_draft` |
| `open_app` | 启动应用程序 | `{name_or_path}` | Dangerous | 新增 |

### 5.2 系统工具集（`tools-windows`）

| 工具名 | 描述 | 参数 | 风险 |
|--------|------|------|------|
| `read_file` | 读取文本文件 | `{path}` | Safe |
| `write_file` | 写入文本文件 | `{path, content}` | Dangerous |
| `list_dir` | 列出目录内容 | `{path}` | Safe |
| `search_files` | 按名称搜索文件 | `{pattern, root?}` | Safe |
| `read_clipboard` | 读取剪贴板 | — | Safe |
| `write_clipboard` | 写入剪贴板 | `{text}` | Moderate |
| `run_command` | 执行 shell 命令 | `{command, cwd?}` | Dangerous |
| `get_system_info` | 获取系统信息 | `{what: "os"\|"cpu"\|"memory"\|"disk"}` | Safe |

### 5.3 信息工具集（`tools`，跨平台）

| 工具名 | 描述 | 参数 | 风险 |
|--------|------|------|------|
| `web_search` | 网页搜索 | `{query, max_results?}` | Safe |
| `web_fetch` | 获取网页内容 | `{url}` | Safe |
| `translate` | 翻译文本 | `{text, from?, to}` | Safe |
| `calculate` | 数学计算 | `{expression}` | Safe |
| `get_time` | 获取当前时间 | `{timezone?}` | Safe |
| `set_reminder` | 设置提醒 | `{time, message}` | Moderate |
| `polish_text` | 润色文本 | `{text, style?}` | Safe |
| `summarize` | 摘要文本 | `{text, max_length?}` | Safe |

### 5.4 OwO 协作工具集（`tools-windows`，可选）

| 工具名 | 描述 | 参数 | 风险 |
|--------|------|------|------|
| `owo_switch_mode` | 切换 OwO 输入模式 | `{mode: "chinese"\|"english"}` | Moderate |
| `owo_add_phrase` | 添加用户词组 | `{phrase, pinyin}` | Moderate |
| `owo_get_candidates` | 获取拼音候选 | `{pinyin}` | Safe |

---

## 六、UI 重构设计

### 6.1 三视图架构

```
┌─────────────────────────────────────────┐
│  灵犀 Agent                              │
├────────┬────────────────────────────────┤
│ 侧边栏  │           主区域               │
│        │                                │
│ 💬对话  │  ┌──────────────────────────┐  │
│ ✍选区  │  │                          │  │
│ 🔧工具  │  │     当前视图内容          │  │
│ ⚙设置  │  │                          │  │
│        │  │                          │  │
│ ─────  │  └──────────────────────────┘  │
│ 📋历史  │  ┌──────────────────────────┐  │
│        │  │ 输入框          [发送]    │  │
│ 状态:  │  └──────────────────────────┘  │
│ 就绪   │                                │
└────────┴────────────────────────────────┘
```

### 6.2 对话视图（主交互）

```html
<!-- apps/overlay/ui/index.html 新增 chat 视图 -->
<div id="chat-view" class="view">
  <div id="chat-messages" class="chat-messages">
    <!-- 消息气泡：user / assistant / tool -->
    <!-- 工具调用卡片：折叠式，显示工具名+参数+结果 -->
  </div>
  <div class="chat-input-area">
    <textarea id="chat-input" placeholder="描述你想做的事..."></textarea>
    <button id="chat-send">发送</button>
    <button id="chat-attach">📎</button>  <!-- 附加选区/文件 -->
  </div>
</div>
```

**消息类型**：
- **user**：用户输入（右对齐气泡）
- **assistant**：Agent 回复（左对齐气泡，支持 markdown 渲染）
- **tool_call**：工具调用卡片（折叠式，显示工具名、参数、结果、耗时）
- **confirm**：确认请求卡片（危险操作，用户点确认/取消）

### 6.3 选区视图（保留，简化）

原有的选区辅助功能保留为独立视图，但底层调用通用工具。

### 6.4 工具视图（新增）

```
┌────────────────────────────────┐
│ 已注册工具 (24)                 │
├────────────────────────────────┤
│ 🔍 搜索工具...                  │
├────────────────────────────────┤
│ 📁 应用交互                     │
│  ├ read_selection    [Safe]    │
│  ├ write_text        [Moderate]│
│  ├ click_element     [Moderate]│
│  └ ...                         │
│ 📁 系统工具                     │
│  ├ read_file         [Safe]    │
│  ├ run_command    [Dangerous]   │
│  └ ...                         │
│ 📁 信息工具                     │
│  ├ web_search        [Safe]    │
│  └ ...                         │
└────────────────────────────────┘
```

每个工具可单独启用/禁用、查看 schema、手动测试。

---

## 七、Tauri 命令新增

### 7.1 Agent 相关命令

```rust
// apps/overlay/src/main.rs 新增

#[tauri::command]
async fn agent_chat(
    state: State<'_, AppState>,
    message: String,
) -> Result<ChatResponse, String> {
    let engine = state.agent_engine.lock().await;
    let mut session = state.session.lock().await;
    let confirm = TauriConfirmGate::new(app_handle);
    let reply = engine.run(&message, &mut session, &confirm).await
        .map_err(|e| e.to_string())?;
    Ok(ChatResponse { reply, tool_calls: session.last_tool_calls() })
}

#[tauri::command]
async fn agent_confirm(
    state: State<'_, AppState>,
    request_id: String,
    approved: bool,
) -> Result<(), String> {
    state.confirm_resolver.lock().await.resolve(&request_id, approved);
    Ok(())
}

#[tauri::command]
fn list_tools(state: State<'_, AppState>) -> Vec<ToolView> {
    state.tool_registry.schemas().iter().map(|s| ToolView {
        name: s.name.clone(),
        description: s.description.clone(),
        risk_level: state.tool_registry.risk_of(&s.name),
        enabled: state.tool_registry.is_enabled(&s.name),
    }).collect()
}

#[tauri::command]
fn toggle_tool(state: State<'_, AppState>, name: String, enabled: bool) -> Result<(), String> {
    state.tool_registry.set_enabled(&name, enabled);
    Ok(())
}

#[tauri::command]
async fn execute_tool(
    state: State<'_, AppState>,
    name: String,
    params: Value,
) -> Result<ToolResult, String> {
    let ctx = ToolContext::from_state(&state);
    state.tool_registry.execute(&name, params, &ctx).await
        .map_err(|e| e.to_string())
}
```

### 7.2 确认门控（Tauri 实现）

```rust
struct TauriConfirmGate {
    app: AppHandle,
}

impl ConfirmGate for TauriConfirmGate {
    fn confirm(&self, request: &ConfirmRequest) -> bool {
        let id = uuid::Uuid::new_v4().to_string();
        // 发事件到前端，弹出确认卡片
        self.app.emit("tool-confirm-request", ConfirmCard {
            id: id.clone(),
            tool_name: request.tool_name.clone(),
            action_summary: request.action_summary.clone(),
            risk_level: request.risk_level,
            params: request.params.clone(),
        }).ok();
        // 阻塞等待用户响应（用 oneshot channel）
        self.wait_for_confirmation(&id)
    }
}
```

---

## 八、迁移路线（6 阶段）

### 阶段 1：Agent 引擎骨架（1 周）

1. 新建 `crates/agent`：定义 `Tool` trait、`ToolRegistry`、`AgentEngine`、`AgentBackend` trait、`Session`
2. 新建 `crates/tools`：`ToolSchema`、`ToolContext`、内置文本工具
3. 在 `assistant-inference` 实现 `CloudAgentBackend`（function calling）
4. 单元测试：Mock 工具 + Mock backend 验证对话循环

**交付物**：Agent 能跑通"用户问 → 模型决定调工具 → 执行 → 回复"的最小闭环

### 阶段 2：Windows 工具集（1.5 周）

5. 新建 `crates/tools-windows`
6. 把 `assistant-windows` 现有能力包装成工具：`read_selection`、`write_text`、`undo_write`、`click_element`、`type_text`、`send_keys`、`qq_read_selection`、`qq_write_draft`
7. 新增：`list_windows`、`focus_window`、`capture_screen`、`find_element`
8. 新增：`read_file`、`write_file`、`list_dir`、`read_clipboard`、`write_clipboard`、`run_command`

**交付物**：Agent 能通过自然语言操作 Windows 应用

### 阶段 3：UI 重构（1.5 周）

9. `index.html` 新增对话视图：消息列表、输入框、工具卡片、确认弹窗
10. `app.js` 新增 `agent_chat`、`agent_confirm`、`list_tools`、`execute_tool` 调用
11. 保留选区视图，但底层改调 `read_selection`/`write_text` 工具
12. 新增工具视图：列出/启用/禁用/手动测试工具

**交付物**：完整的 Agent 对话 UI

### 阶段 4：会话持久化与多轮记忆（1 周）

13. `crates/agent/session.rs`：会话存储（SQLite 或 JSON），跨重启恢复
14. 上下文管理：滑动窗口 + 摘要压缩（防止 token 爆炸）
15. 工具调用历史可视化

**交付物**：Agent 有记忆，能引用之前的对话

### 阶段 5：信息工具与后台任务（1.5 周）

16. `web_search`、`web_fetch`、`translate`、`calculate` 工具
17. 后台任务调度：定时执行、事件触发（如"每次 QQ 收到消息时读取并生成草稿"）
18. 提醒系统：`set_reminder`

**交付物**：Agent 有丰富的信息获取能力 + 自动化

### 阶段 6：优化与生态（持续）

19. 本地模型支持 function calling（candle 端的 tool use 解析）
20. 工具市场：用户可注册自定义工具（脚本/插件）
21. OwO 深度协作：输入法状态共享、候选增强
22. 语音输入集成
23. 多显示器/多桌面支持

---

## 九、关键技术决策

### 9.1 为什么用 OpenAI function calling 而非 ReAct 纯文本

| 维度 | Function Calling | ReAct 文本 |
|------|-----------------|-----------|
| 可靠性 | 高（结构化 JSON） | 中（需解析模型自由文本） |
| 模型支持 | OpenAI/Anthropic/通义/智谱 | 任意模型 |
| 本地模型 | 需微调或 prompt 工程 | 原生支持 |

**决策**：云端优先 function calling；本地模型走 ReAct 降级（prompt 里描述工具，解析 `Action: tool_name\nArgs: {...}`）。

### 9.2 确认门控设计

```rust
pub trait ConfirmGate {
    fn confirm(&self, request: &ConfirmRequest) -> bool;
}
```

**三档风险**：
- **Safe**：读操作、计算 → 直接执行
- **Moderate**：写文本、剪贴板 → 首次确认，可"本会话不再询问"
- **Dangerous**：执行命令、删文件、发消息 → 每次必须确认

**实现**：Tauri `emit` 发事件到前端 → 前端弹确认卡片 → 用户点确认 → `agent_confirm` 命令回传 → 后端 oneshot channel 解除阻塞。

### 9.3 异步工具执行

工具 trait 用 `async_trait`，因为：
- `web_search`、`web_fetch` 需要网络 IO
- `run_command` 可能长时间运行
- UIA 遍历可能耗时

Agent 循环本身也是 async，用 `tokio` runtime（Tauri 已内置）。

### 9.4 工具参数校验

每个工具的 `ToolSchema.parameters` 是 JSON Schema，在 `registry.execute` 入口做校验（用 `jsonschema` crate），参数不合法直接返回错误，不进入工具逻辑。

---

## 十、示例场景

### 场景 1："帮我把这段话翻译成英文并写入光标处"

```
用户：帮我把这段话翻译成英文并写入光标处
Agent: [think] 需要先读取选区 → 翻译 → 写回
Agent: [call read_selection] → "你好世界"
Agent: [call translate {text:"你好世界", to:"en"}] → "Hello World"
Agent: [call write_text {text:"Hello World", mode:"replace"}] → success
Agent: 已完成。选中的"你好世界"已替换为"Hello World"。
```

### 场景 2："QQ 里 cyc 说了什么，帮我回复"

```
用户：QQ 里 cyc 说了什么，帮我回复
Agent: [call list_windows {filter:"QQ"}] → 找到 QQ 窗口
Agent: [call focus_window {title:"cyc"}] → 聚焦到 cyc 的聊天
Agent: [call qq_read_selection] → 需要用户先选中消息
Agent: 请在 QQ 里选中 cyc 的消息，然后告诉我
用户：选好了
Agent: [call qq_read_selection] → "明天下午3点开会"
Agent: [call generate_reply {message:"明天下午3点开会"}] → "好的，收到。明天3点准时到。"
Agent: [call qq_write_draft {draft:"好的，收到。明天3点准时到。"}] → 已写入草稿框
Agent: 草稿已写入 QQ 输入框，检查后按发送即可。
```

### 场景 3："把桌面上所有 .log 文件里的 ERROR 行提取出来汇总"

```
用户：把桌面上所有 .log 文件里的 ERROR 行提取出来汇总
Agent: [call list_dir {path:"~/Desktop"}] → 找到 3 个 .log 文件
Agent: [call read_file {path:"app1.log"}] → ...
Agent: [call read_file {path:"app2.log"}] → ...
Agent: [call read_file {path:"app3.log"}] → ...
Agent: [汇总] 发现 12 条 ERROR
Agent: [call write_file {path:"~/Desktop/error_summary.txt", content:"..."}]
Agent: 已汇总到桌面的 error_summary.txt，共 12 条 ERROR。
```

### 场景 4：后台自动化

```
用户：每次 QQ 收到新消息时，帮我生成一个回复草稿（不要自动发）
Agent: [注册后台触发器]
Agent: [触发] → qq_read_selection → generate_reply → qq_write_draft
(用户回到 QQ 看到草稿，自己决定发不发)
```

---

## 十一、与现有代码的融合点

| 现有代码 | 融合方式 |
|----------|----------|
| `InputAdapter` trait | 保留，`read_selection`/`write_text` 工具内部调用它 |
| `WindowsAdapter` | 保留，作为工具的底层实现 |
| `qq.rs` 全部函数 | 包装为 `qq_*` 工具，不再直接被 overlay 调用 |
| `ModelBackend` trait | 保留，`polish_text`/`summarize` 工具用它 |
| `Transformer` trait | 保留，作为 `polish_text` 等工具的实现 |
| 选区热键流程 | 保留，热键触发时也可以走 Agent（"对选区做 X"） |
| `on_transform` | 保留为快捷路径，直接调 transformer 不走 Agent |
| 全局热键 `Ctrl+Alt+Space` | 保留：快速选区变换。新增 `Ctrl+Alt+A`：打开 Agent 对话 |

---

## 十二、风险与对策

| 风险 | 对策 |
|------|------|
| Agent 死循环（工具互相调用） | `max_steps` 限制（默认 10），超出报错 |
| 工具执行超时 | 每个工具有超时（默认 30s），`run_command` 60s |
| Token 爆炸 | 滑动窗口 + 旧消息摘要压缩 |
| 危险操作误执行 | 三档确认门控 + 操作日志（可审计） |
| 本地模型不支持 function calling | ReAct 降级模式 |
| UIA 遍历性能 | 工具内加缓存 + 范围限定参数 |
| 工具数量爆炸难管理 | 分类 + 搜索 + 启用/禁用 |

---

## 十三、文件结构总览（目标）

```
cross-app-assistant/
├── Cargo.toml                    # workspace（新增 agent, tools, tools-windows 成员）
├── crates/
│   ├── assistant-core/           # 保留（InputAdapter, Transformer, Diff）
│   ├── assistant-inference/      # 扩展（新增 CloudAgentBackend）
│   ├── assistant-windows/        # 保留（底层能力不变）
│   ├── agent/                    # 🆕 Agent 引擎
│   ├── tools/                    # 🆕 工具 trait + 注册表 + 信息工具
│   └── tools-windows/            # 🆕 Windows 工具实现
├── apps/
│   ├── overlay/                  # UI 重构（新增对话视图）
│   ├── probe/                    # 保留
│   └── watch/                    # 保留
├── archived/                     # 退役模块归档
│   ├── ime-server/
│   ├── ime-repl/
│   └── assistant-ime/
└── docs/
    ├── 灵犀Agent升级设计方案.md   # 本文档
    └── 灵犀项目状态报告与修改建议.md
```

---

## 总结

本方案的核心转变是：**从"单次变换函数"到"自主工具调用 Agent"**。

关键设计决策：
1. **Tool trait 统一抽象**：所有能力（UIA、文件、剪贴板、搜索）都是工具，Agent 通过统一接口调度
2. **Agent 引擎独立于平台**：`crates/agent` 不依赖 Windows，可测试可移植
3. **现有代码不重写**：`assistant-windows` 的能力包装为工具，不重写底层
4. **人在回路**：三档风险确认门控，危险操作必经用户同意
5. **渐进迁移**：6 阶段路线，每阶段独立可交付，不破坏现有功能

这个方案让灵犀从"输入辅助工具"变成"能看、能写、能搜索、能执行命令的桌面 AI Agent"，同时保留现有的选区辅助和 QQ 集成能力作为 Agent 工具集的一部分。
