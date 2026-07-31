# cross-app-assistant

端侧大模型跨应用智能输入辅助系统的 Windows/Rust 核心。

当前里程碑：**W0-W5 完成，W7 核心（可插拔变换器 + Diff 引擎）已就绪**。已具备 UIA 能力探测、全局快捷键、三级读取、安全快照校验、跨应用写回、写后验证、撤销，以及平台无关的变换器抽象与字符级 Diff。端侧模型与 Tauri 图形界面为后续。

## 目录结构

```text
cross-app-assistant/
├── crates/
│   ├── assistant-core/      # 平台无关：模型、InputAdapter、变换器、Diff、流程编排、Mock 测试
│   ├── assistant-ime/       # 纯 Rust 拼音输入法引擎：音节切分、词典候选生成、可插拔重排
│   └── assistant-windows/   # UIA、Win32、剪贴板、键盘、快照、写回与撤销
└── apps/
    ├── probe/               # 倒计时后探测焦点控件
    ├── watch/               # Ctrl+Alt+Space 查看焦点控件和选区
    ├── demo/                # 端到端 变换 -> Diff -> 写回/撤销 演示
    ├── smoke/               # 自动创建 Win32 Edit 控件的原生集成冒烟
    ├── ime-repl/            # 拼音输入法引擎的命令行/交互式 demo
    └── overlay/             # Tauri 浮窗（独立子项目，需 MSVC + WebView2）
```

## 浮窗（Tauri overlay）

`apps/overlay` 是一个悬浮改写窗口：玻璃拟态卡片、模式切换、字符级 Diff 高亮、应用/撤销。前后端约定：

- 后端命令：`current_selection` / `preview_transform` / `apply_transform` / `undo_last` / `hide_overlay`；
- 全局热键在后台线程复用 `assistant-windows`：`Ctrl+Alt+Space` 捕获选区并唤起浮窗（不抢焦点），`Ctrl+Alt+Backspace` 撤销；
- 前端 `ui/` 为纯静态 HTML/CSS/JS，检测到 `window.__TAURI__` 时走真实 IPC，否则回退内置 mock，便于浏览器预览。

### 先预览界面（无需编译）

```powershell
node apps/overlay/ui/preview-server.cjs   # 启动后打开 http://localhost:8777/index.html
```

浏览器里即可切换模式、看 Diff 高亮、点应用/撤销（mock 模式）。

### 编译成桌面浮窗（前置：MSVC）

Tauri 在 Windows 上需要 **MSVC 工具链 + WebView2 Runtime**，当前环境只有 GNU 工具链，因此 overlay 被 workspace `exclude`，不影响其余 crate 构建。安装 Visual Studio Build Tools（含 “使用 C++ 的桌面开发”）并添加 MSVC target 后：

```powershell
rustup target add x86_64-pc-windows-msvc
cd apps/overlay
cargo run --target x86_64-pc-windows-msvc
```

## 变换器与 Diff

变换逻辑通过 `assistant_core::Transformer` trait 抽象，端侧模型将实现同一 trait，捕获/写回代码无需改动。当前内置三个确定性变换器：

- `prefix`：加 `[AI] ` 前缀（默认）
- `tidy`：折叠行内多余空白并整理每行
- `upper`：转大写

`transform_selection` 会产出字符级 LCS Diff（`DiffOp` 序列），可内联渲染为 `[-删除-][+新增+]` 并统计增删字符数，供界面或控制台预览。

## 拼音输入法引擎（assistant-ime）

`assistant-ime` 是真正 IME 的核心——把裸拼音键流变成有序中文候选，**纯 Rust、零 C 依赖**，随 workspace 在 GNU 工具链下编译并单元测试（无需真机、无需 librime/ICU）。分层与 librime 对齐，未来可用 librime+rime-ice 在同一 `InputEngine` trait 后替换内部实现：

```text
裸拼音 ─▶ 音节切分 ─▶ 候选生成(Viterbi 最短路径) ─▶ 上下文重排 ─▶ 候选
"nihao"   [ni][hao]     你好 / 你+好 …               避免重复/覆盖加权
```

- `segment`：把键流切成合法拼音音节，枚举全部合法切分（如 `xian` = 先 或 西安），并支持 `xi'an` 显式边界与"边打边出"的最长前缀兜底；
- `dict::Dictionary`：词 ↔ 拼音 ↔ 词频，支持内置最小词库、rime-ice 风格 `词<TAB>拼音<TAB>权重` 文本加载（含 YAML 前言跳过、拼音列可含空格）、`load_file` / `from_files` 从磁盘加载、模糊音（`zh↔z`、`in↔ing`、`l↔n` 等）；
- `engine::PinyinInputEngine`：对每种切分做基于对数词频的 Viterbi 句子搜索，叠加整词/前缀词/单字兜底，产出排序候选；
- `rerank::CandidateReranker`：可插拔重排接口，内置 `FrequencyReranker`（词频基线）与 `PrefixContextReranker`（上下文启发式），**为未来神经重排（对 top-K 打分）预留同一接口**。

```rust
use assistant_ime::{InputEngine, InputContext, PinyinInputEngine};

let engine = PinyinInputEngine::builtin();
let cands = engine.candidates("nihao", &InputContext::with_limit(5));
assert_eq!(cands[0].text, "你好");
```

### 命令行体验（ime-repl）

无需真机、无需 GUI，直接在终端跑引擎：

```powershell
# 一次性模式：传入拼音打印候选
cargo run -p ime-repl -- nihao woaizhongguo xian

# 交互式 REPL：不带拼音参数，逐行输入
cargo run -p ime-repl

# 选项：--dict <文件>（可重复，加载 rime-ice 词库）、--limit N、--context <前文>、--no-fuzzy
cargo run -p ime-repl -- --limit 5 --context 你好 shijie
```

输出示例：

```text
nihao:
  1. 你好  [ni hao]  score=19.902
  2. 你  [ni]  score=9.560
woaizhongguo:
  1. 我爱中国  [wo ai zhong guo]  score=40.345
```

## 构建与自动测试

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

原生 UIA 冒烟测试：

```powershell
cargo run -p smoke
```

它会短暂创建一个真实 Win32 `Edit` 控件，自动完成“选区捕获 → 写回 → 验证 → 撤销”。无交互桌面的 CI/IDE 命令会话无法获得前台窗口时会明确显示 `SMOKE SKIP`，不会误报通过。

## 一次性体验完整效果

先关闭旧的 `watch`/`demo` 进程，再运行（可选变换模式）：

```powershell
cargo run -p demo            # 默认 prefix
cargo run -p demo -- tidy    # 整理空白
cargo run -p demo -- upper   # 转大写
```

然后在记事本、QQ 或浏览器输入框中：

1. 选中一段文字；
2. 按 `Ctrl+Alt+Space`，程序应用变换、打印 Diff 预览并安全替换选区；
3. 按 `Ctrl+Alt+Backspace`，撤销本次写回。

写回前会校验前台窗口、UIA RuntimeId、密码框标记、选区文本和全文哈希。期间切换窗口、移动焦点或修改内容时会返回 `TargetChanged` 并拒绝写回。

## 真实读写策略

读取：

```text
TextPattern.GetSelection → ValuePattern.CurrentValue → 受控 Ctrl+C
```

写入：

```text
ValuePattern.SetValue（整值控件）
受控剪贴板 + Ctrl+V（TextPattern/选区控件）
SendInput/Delete（底层键盘通道）
```

`TextPattern` 本身只有读取和选择能力，**没有插入/替换文本接口**。因此不存在 `TextPatternInsert`；富文本写回使用已校验选区上的粘贴/键盘通道。

剪贴板操作使用 OLE `IDataObject` 保存并恢复全部格式，而不是只备份纯文本。

## 里程碑

- **W0**：COM/UIA 地基 + `probe` 能力探测（完成）
- **W1**：前台窗口 / 焦点 / 全局快捷键（完成）
- **W2**：TextPattern 读选区（完成）
- **W3**：TextPattern / ValuePattern / 剪贴板三级读取（完成）
- **W4**：RuntimeId + 窗口 + 内容快照校验、安全写回与验证（完成）
- **W5**：文本剪贴板恢复、回执与安全撤销（完成）
- **W7 核心**：可插拔 `Transformer` 抽象与字符级 Diff 引擎（完成，平台无关、单元测试覆盖）
- W6：Word/Outlook 等 P1 程序的真机适配矩阵（待目标程序可用后执行）
- W7 余项：接入端侧模型（实现 `Transformer`）与 Tauri Diff 浮窗
