# 灵犀 v2.0 执行标准

> 本文件是长程项目自主推进的执行规范，所有代码变更必须遵循。

## 一、代码规范

### Rust
- 模块顶部必须有 `//!` 模块注释说明用途
- `unsafe` 块必须有安全注释说明为什么安全
- 错误处理用 `Result<T, String>`（工具层）或 `anyhow::Result`（应用层）
- 公开函数必须有文档注释 `///`
- 遵循现有风格：`#[cfg(windows)]` 条件编译 + 非 Windows 优雅降级
- 工具实现遵循 `Tool` trait 模式（schema + execute + risk_level）

### JavaScript
- 严格模式 `"use strict"`
- 事件绑定用 `addEventListener`，禁止内联 `onclick`
- DOM 查询缓存到 `el` 对象，禁止重复 `getElementById`
- 异步操作必须有错误处理（try/catch + 用户可见提示）

### CSS
- 使用 CSS 变量（`--brand`、`--text` 等），禁止硬编码颜色
- 响应式：移动端单列，桌面端多列
- 暗色模式通过 `:root[data-widget-theme="dark"]` 覆盖

### HTML
- 语义化标签（`<section>`、`<nav>`、`<header>`）
- 所有交互元素必须有 `aria-label`

## 二、验证流程

每个模块完成后必须通过：
1. `cargo build` — 编译通过
2. `cargo test` — 测试通过（如有测试）
3. `cargo clippy` — 无 warning
4. 集成构建：`cd apps/overlay && cargo build --release`
5. 运行时验证：启动 overlay.exe 确认无崩溃

## 三、提交规范

- Conventional Commits：`feat(scope): description`
- scope 示例：`tools`、`overlay`、`widgets`
- 每个完整功能一次提交

## 四、文件组织

```
crates/tools-windows/src/
  ├── screen_capture.rs    # 新增：屏幕捕获 + OCR
  ├── window_tools.rs      # 修改：注册 capture_screen
  └── lib.rs               # 修改：注册新工具

apps/overlay/
  ├── src/
  │   ├── main.rs          # 修改：小工具窗口管理 + 命令
  │   └── widgets.rs       # 新增：小工具注册表
  ├── ui/
  │   ├── index.html       # 修改：工具页美化
  │   ├── styles.css       # 修改：卡片网格样式
  │   ├── app.js           # 修改：工具页逻辑
  │   └── widgets/         # 新增：小工具前端
  │       ├── ocr.html
  │       ├── ocr.css
  │       └── ocr.js
  └── Cargo.toml           # 修改：新增依赖
```

## 五、执行顺序

见 ROADMAP.md 阶段 1 优先级排序：
1. 屏幕捕获基础设施
2. 工具页美化（并行）
3. OCR 小工具
4. 天气 / 取色器 / 计算器
5. 剪贴板历史 / 截图标注
