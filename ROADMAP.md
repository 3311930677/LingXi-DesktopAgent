# 灵犀 Agent 升级路线图

## 当前阶段：Phase 1 — Agent 引擎骨架

### Phase 1：Agent 引擎骨架 ✅ 进行中
- [ ] `crates/tools` — Tool trait + ToolRegistry + schema/context/result 类型
- [ ] `crates/agent` — AgentEngine + AgentBackend + Session + ReAct 循环
- [ ] `assistant-inference` 扩展 CloudAgentBackend (function calling)
- [ ] 单元测试：Mock 工具 + Mock backend 验证对话闭环

### Phase 2：Windows 工具集
- [ ] `crates/tools-windows` — 包装 assistant-windows 为工具
- [ ] 系统工具 (file/clipboard/shell/screen)

### Phase 3：UI 重构
- [ ] 对话面板视图
- [ ] 工具管理视图
- [ ] 确认门控 UI

### Phase 4：会话持久化
- [ ] SQLite 会话存储
- [ ] 上下文窗口管理

### Phase 5：信息工具与后台任务
- [ ] web_search / web_fetch / translate / calculate
- [ ] 后台任务调度器

### Phase 6：优化与生态
- [ ] 本地模型 tool use (ReAct)
- [ ] OwO 深度协作
- [ ] 工具市场
