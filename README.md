# 灵犀智能输入（LingXi）

一个面向 Windows 的智能输入与桌面 Agent 原型。项目将**跨应用拼音输入、AI 文本处理、QQ 回复草稿、桌宠交互**整合到同一个 Rust/Tauri 桌面程序中。

当前版本的主要体验：

- 在任意文本框直接输入拼音，显示跟随光标的候选窗；
- 使用 rime-ice 大词库生成常用字词候选；
- 选中文字后进行润色、纠错或提示词增强；
- 读取 QQ 当前聊天并生成回复草稿，确认后写入输入框；
- 使用本地 Qwen2.5 1.5B，或切换到 OpenAI-compatible 云端模型；
- 通过桌宠展示待机、思考、回复和新消息状态。

> 当前拼音输入通过 Windows 全局低级键盘钩子和浮动候选窗实现，**不是已经注册到 Windows 的 TSF 系统输入法**。候选确认后通过受控剪贴板写入目标应用。Weasel/librime filter 位于实验目录，不是当前主运行链路。

## 快速开始

### 环境要求

- Windows 10/11 x64
- Git
- Rust stable（GNU + MSVC toolchain）
- Visual Studio Build Tools（安装“使用 C++ 的桌面开发”）
- Microsoft Edge WebView2 Runtime
- Node.js 18+

安装 Rust 工具链：

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup toolchain install stable-x86_64-pc-windows-msvc
```

根 workspace 使用 GNU；`apps/overlay` 通过自己的 `rust-toolchain.toml` 自动使用 MSVC。

### 一键启动（推荐）

```powershell
git clone https://github.com/3311930677/Intelligent-Input-Method-v2.git
cd Intelligent-Input-Method-v2
powershell -ExecutionPolicy Bypass -File .\scripts\run-dev.ps1
```

脚本会自动：

1. 检查或克隆 rime-ice（默认放在项目同级目录）；
2. 启动 `ime-server`，加载 `8105.dict.yaml` 和 `base.dict.yaml`；
3. 自动检查 `nihao`、`suoyi`、`dang`、`zhong`、`ren`、`de` 等候选；
4. 自测通过后启动 Tauri overlay。

如果 rime-ice 已放在其他位置：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-dev.ps1 -RimeDir "D:\path\to\rime-ice"
```

## 拼音输入

Overlay 启动后，拼音输入模式默认开启。将光标放在任意普通文本框中即可直接输入拼音。

| 操作 | 行为 |
|---|---|
| `A`–`Z` | 累积拼音并刷新候选 |
| `Space` / `Enter` | 提交第 1 个候选 |
| 顶部数字键 `1`–`9` | 提交对应候选 |
| `Backspace` | 删除一个拼音字母 |
| `Esc` | 清空组合串并退出拼音输入模式 |
| `Ctrl+Alt+I` | 开启或关闭拼音输入模式 |

候选窗右上角会显示当前词库状态：

- **大词库（绿色）**：已连接 `ime-server` 和 rime-ice；
- **基础词库（橙色）**：服务未连接，正在使用内置最小词典，候选会明显变少。

如果 `dang`、`zhong` 等常用字没有候选，先检查候选窗是否显示“大词库”，或运行：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\test-ime.ps1
```

### 手动启动

```powershell
# 终端 1：大词库服务
cd D:\path\to\Intelligent-Input-Method-v2
cargo run -p ime-server -- `
  --dict "D:\path\to\rime-ice\cn_dicts\8105.dict.yaml" `
  --dict "D:\path\to\rime-ice\cn_dicts\base.dict.yaml"

# 终端 2：桌面程序
cd apps\overlay
cargo run
```

`ime-server` 默认监听 `127.0.0.1:9527`，使用每连接一行 JSON 请求/响应的本地 TCP 协议。

## AI 文本处理

在记事本、浏览器、QQ 等支持 UIA/剪贴板写回的应用中：

1. 选中一段文本；
2. 按 `Ctrl+Alt+Space`；
3. 在浮窗中预览字符级 Diff；
4. 选择处理模式并确认应用。

当前 Overlay 提供三个正式模式：

- **润色**：保留原意，根据文本场景改善表达并适度扩写；
- **纠错**：只修正错别字、标点和语法问题；
- **提示词增强**：补全角色、目标、背景、约束、步骤和输出格式。

快捷键：

| 快捷键 | 行为 |
|---|---|
| `Ctrl+Alt+Space` | 捕获当前选区并打开 AI 处理面板 |
| `Ctrl+Alt+Backspace` | 撤销最近一次成功写回 |
| `Ctrl+Alt+I` | 开关拼音输入模式 |

写回前会检查前台窗口、UIA RuntimeId、选区内容和全文哈希。目标或内容发生漂移时会拒绝写入，避免修改错误窗口。

## 模型后端

### 本地模型

默认本地后端为 **Qwen2.5 1.5B Instruct GGUF（Q4_K_M）**。首次使用时会从 Hugging Face 下载并缓存模型和 tokenizer。

可通过环境变量覆盖：

| 环境变量 | 用途 |
|---|---|
| `LINGXI_GGUF_PATH` | 使用本地 GGUF 文件 |
| `LINGXI_TOKENIZER_PATH` | 使用本地 `tokenizer.json` |
| `LINGXI_MODEL_REPO` | 覆盖 Hugging Face GGUF 仓库 |
| `LINGXI_GGUF_FILE` | 覆盖 GGUF 文件名 |
| `LINGXI_TOKENIZER_REPO` | 覆盖 tokenizer 仓库 |

### 云端模型

设置页支持 OpenAI-compatible `chat/completions` 接口，并提供：

- DeepSeek：`deepseek-chat`
- 通义千问：`qwen-plus`
- OpenAI：`gpt-4o-mini`
- 自定义 Endpoint 与 Model

API Key 仅保存在当前进程内，不写入设置文件；也可以通过 `LINGXI_OPENAI_API_KEY` 提供。

## QQ 回复草稿与桌宠

### QQ 回复草稿

- 通过 UI Automation 读取当前 QQ/QQNT 聊天窗口的可见文本；
- 调用当前模型生成一条回复草稿；
- 用户可以编辑草稿并写入 QQ 输入框；
- 程序**不会自动发送消息**，最终发送操作必须由用户确认。

### 桌宠

- 默认显示在当前显示器工作区右下角；
- 支持待机、思考、说话和提醒状态；
- 单击桌宠可显示或隐藏主面板；
- 定时检测 QQ 消息变化并显示提醒。

## 架构

```text
全局键盘钩子 ──▶ 拼音缓冲 ──▶ ime-server ──▶ rime-ice 词典
      │                                  │
      └──────── 候选窗 ◀── 候选排序 ◀────┘
                         │
候选确认 ──▶ 受控剪贴板 / Ctrl+V ──▶ 目标应用光标

选区热键 ──▶ UIA 捕获与安全快照 ──▶ 本地/云端模型
      └──────────────────────▶ Diff 预览 ──▶ 安全写回/撤销
```

主要目录：

```text
crates/
├── assistant-core/       # 平台无关领域类型、Transformer、Diff 和流程编排
├── assistant-ime/        # 拼音切分、词典、候选生成与可插拔重排
├── assistant-windows/    # UIA、Win32、键盘、剪贴板、写回、撤销与 QQ 适配
└── assistant-inference/  # 本地 Candle/Qwen 和 OpenAI-compatible 云端后端

apps/
├── overlay/              # Tauri 桌面 Agent、候选窗、桌宠与设置页
├── ime-server/           # rime-ice 大词库候选服务（127.0.0.1:9527）
├── ime-repl/             # 拼音引擎命令行调试工具
├── probe/                # UIA 焦点控件探测
├── watch/                # 热键监视与选区诊断
├── smoke/                # Win32 Edit 原生读写冒烟测试
└── demo/                 # 早期确定性变换管线演示程序

scripts/
├── setup-rime-ice.ps1    # 准备 rime-ice
├── run-dev.ps1           # 一键启动开发环境
└── test-ime.ps1          # 常用拼音候选协议自测
```

`plugins/lingxi-rime-filter` 是 librime/Weasel 集成实验，不属于当前默认运行链路。

## 开发与测试

根 workspace：

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Overlay：

```powershell
cd apps\overlay
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
node --check ui\ime.js
```

拼音候选协议：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\test-ime.ps1
```

GitHub Actions 会对 Pull Request 自动执行根 workspace 和 Overlay 检查。参与共创、分支和 PR 约定见 [`CONTRIBUTING.md`](CONTRIBUTING.md)。

## 当前限制

- 仅支持 Windows；
- 当前输入方式是全局低级键盘钩子 + 浮动候选窗，不是 TSF 系统输入法；
- 候选提交依赖受控剪贴板和 `Ctrl+V`，少数限制剪贴板/模拟按键的应用可能无法写入；
- 候选上下文目前只包含本次灵犀会话提交的文本，不会自动读取目标光标前的完整上下文；
- IME 开启时字母键会被全局截获；在 API Key、密码或英文输入框中输入前应按 `Ctrl+Alt+I` 关闭；
- QQ 功能依赖 QQ 当前 UIA 结构，QQ 更新后可能需要重新适配；
- OCR 识屏、神经候选重排、真正的 TSF 注册和完整安装包尚未成为当前正式功能。

## 安全说明

- API Key 不写入配置文件；
- 密码框和敏感控件的 AI 选区读写会被拒绝；
- QQ 草稿只写入输入框，不自动发送；
- rime-ice、模型权重、日志和本地配置不会提交到仓库。
