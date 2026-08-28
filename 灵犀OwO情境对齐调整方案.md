# 灵犀 OwO 情境对齐调整方案

## Goal

把《OwO情境感知智能交互系统-产品功能与创新点》定义的 14 项产品功能与 9 大创新点，逐项落到灵犀（LingXi-DesktopAgent）的代码里，核心是补上灵犀目前完全缺失的"情境层"：

1. **情境中枢 ContextService**：全前台 500ms 采样 → 六情境分类（写作/沟通/编程/学习/勿扰/通用）→ 桌宠徽标 + 主面板徽标 + 手动纠正菜单；
2. **主动建议卡**：剪贴板英文→翻译、报错→分析、久坐→提醒（带冷却与"不再提示"）；
3. **文件拖给桌宠**：图片→OCR、代码/文本→解释；
4. **情境化表达**：10 种风格矩阵、斜杠命令、QQ 快捷回复三变体；
5. **行动可信**：确认门控从 DenyAll 接到真实确认 UI、planner 步骤进度、敏感界面自动停止、操作日志与撤回；
6. **情境记忆与个性化**：可查看/清除的偏好记忆；
7. **OwO 协作接口**：HTTP 服务把 rewrite/complete/summarize 暴露给 OwO TSF（打字底座归 OwO/rime，灵犀不做输入法）。

## Architecture

```
感知（context.rs 采样线程，纯 Win32：进程名+标题+全屏）
   → 表达（pet.js 情境徽标/建议卡；app.js 主面板徽标/斜杠命令）
   → 行动（既有 ReAct agent：tools/ + lingxi-agent/，补 OverlayConfirmGate + planner + 撤回）
   → 协作（ipc_service.rs：127.0.0.1 HTTP，供 OwO TSF 调用）
```

- 情境分类是**纯函数**（`classify`），只消费进程名/窗口标题/是否全屏，可单元测试；
- 情境事件走 Tauri `app.emit("context-changed", view)`，桌宠窗口与主面板各自监听；
- 手动覆盖（`scenario_override`）优先级高于自动检测（OwO"可纠正个性化"）。

## Tech Stack

- Rust：workspace 主体为 GNU 工具链；**`apps/overlay` 与 `crates/assistant-inference` 被 workspace exclude，是 MSVC 单独构建** —— 所有 overlay 内改动必须在 `apps/overlay` 目录下 `cargo build/test`；
- Tauri v2（`generate_handler` 注册命令、`tauri::Emitter` 发事件、`tauri://drag-drop` 拖拽）；
- windows 0.58（overlay 已启用 `Win32_Foundation`/`Win32_Graphics_Gdi`/`Win32_UI_WindowsAndMessaging`，够用，无需改 Cargo.toml）；
- 前端为无框架原生 JS：`ui/pet.js`（已有 `invoke`/`listen` 帮助函数）、`ui/app.js`、`ui/index.html`、`ui/pet.html`。

## 执行方式

本方案由 agentic workers 按任务执行：**阶段 A 是可直接执行的 bite-sized 计划**（每步 2-5 分钟、含完整代码与验证命令）；阶段 B-F 为已定稿设计的后续批次，执行到该阶段时按同样粒度展开。执行者不跳步：每步的验证命令必须真实运行且输出符合预期，才能进入下一步；每个任务结束即 `git commit`。

---

## 一、对齐原则（明确不做什么）

依据 VISION.md v2.0 与 docs/owopkg-plugin-feasibility.md 结论 A+C：

- **不做输入法**：词库/候选/上屏归 OwO TSF + rime。ime-server/ime-repl/assistant-ime 按 TODO.md P1 退役，其退役与协作线 F 合并为一次变更；
- **隐私优先**：情境规则只使用进程名 + 窗口标题 + 前台状态，**不读取、不落盘任何文本内容**；敏感界面（阶段 D）只做"检测并停止"，不截图；
- **桌宠人设保留**：PET_LINES 台词、皮肤系统、idle/thinking/speaking/alert 四状态不动；情境徽标是新增 chip，不替代任务状态；
- **安全默认**：工具确认保持现有 DenyAll 兜底，交互式确认由用户在设置页显式开启；
- 每个阶段独立可交付、可回滚（commit 粒度 = 任务粒度）。

## 二、差距分析矩阵（OwO 14 项功能 × 灵犀现状）

| # | OwO 功能 | 灵犀现状（证据） | 调整 | 阶段 |
|---|---|---|---|---|
| ① | 情境识别+模式切换+桌宠状态标签+手动纠正 | `assistant-windows/src/foreground.rs` 有前台检测；无情境系统 | context.rs 情境中枢 + 桌宠徽标 + 覆盖菜单 | A |
| ② | 情境化智能输入 | ime-server 纯本地无 LLM，TODO 计划退役 | 情境注入 rewrite prompt；打字归 OwO | C / F |
| ③ | 句子补全+快捷回复 | `qq.rs` generate_qq_draft 单草稿 | 快捷回复三变体 + 面板续写 | C |
| ④ | 10 种表达风格切换 | rewrite 仅 polish/proofread/prompt-enhance | 风格 prompt 矩阵 | C |
| ⑤ | 专业词汇与个人习惯（可查看/清除） | 无 | context_memory.json + 设置页 | E |
| ⑥ | 桌宠显示 AI 理解状态 | pet.js 4 状态气泡，无情境标签 | 情境徽标 chip | A |
| ⑦ | 桌宠主动轻量建议卡 | 无 | suggestions.rs 规则引擎 + 冷却 + 偏好 | B |
| ⑧ | 文件拖给桌宠分类处理 | 无 | tauri://drag-drop → pet_file_dropped | B |
| ⑨ | 斜杠命令 | 无 | 面板输入解析 6 命令 | C |
| ⑩ | 复杂桌面任务（步骤/进度/确认） | agent 栈完整但 `agent.rs:103` DenyAll，确认 UI 未接线 | OverlayConfirmGate + planner + 进度事件 | D |
| ⑪ | 多智能体协同 | 单 agent ReAct（engine.rs max_steps=10） | planner 多角色（reader/writer/reviewer） | E |
| ⑫ | 跨应用情境连续衔接 | 仅 QQ 前台采样（qq.rs:132） | ContextService 全前台 + 情境记忆 | A / E |
| ⑬ | 重复操作学习与快捷复用 | 无 | 流程指纹 ≥3 次询问保存 | E |
| ⑭ | 安全确认与随时撤回 | tools/context.rs 三档风险存在但未接 UI；无撤回 | 敏感停止 + op_journal + 备份撤回 | D |

## 三、核心设计：情境中枢 ContextService（阶段 A 定稿）

### 3.1 Scenario 枚举与纯函数分类

新建 `apps/overlay/src/context.rs`，与 OwO 情境模式一一对应：

```
全屏(任意进程)          → Dnd 勿扰      （优先级最高：看视频/演示中）
IM 进程                 → Chat 沟通     （qq/weixin/wechat/dingtalk/feishu/lark/telegram/discord/teams）
办公写作进程            → Writing 写作  （winword/wps/et/wpp/notion/typora/obsidian/notepad）
开发工具进程            → Coding 编程   （code/cursor/idea/pycharm/clion/goland/webstorm/devenv/终端系）
浏览器 + 学习类标题词    → Study 学习    （标题含 文档/论文/教程/课程/学习/wiki/docs/github…）
其余                    → General 通用
```

匹配一律 `to_ascii_lowercase()` 后 `ends_with`，同时兼容"文件名"与"完整路径"两种来源；进程命中不分先后按上表优先级。

### 3.2 状态扩展（state.rs）

`AppState` 新增三个字段（`scenario_window` 只存进程名与标题两个字符串，不含内容）：

```
scenario_detected : Mutex<Scenario>          // 自动识别的当前情境
scenario_override : Mutex<Option<Scenario>>  // 用户手动覆盖（Some 期间采样不生效）
scenario_window   : Mutex<(String, String)>  // (进程名, 窗口标题)，供展示
```

### 3.3 采样线程与事件

仿 `qq.rs:132-137` 的 `spawn_qq_foreground_sampler`（500ms 轮询纯 Win32，开销可忽略）：

- `foreground_info()` 取前台 → `is_fullscreen(hwnd)`（窗口矩形 ≥ 显示器矩形）→ `classify`；
- 情境**变化时**才 `emit("context-changed", view)`（避免事件风暴）；窗口元数据每轮更新；
- 手动覆盖期间跳过自动更新，但保持运行。

### 3.4 对外命令

| 命令 | 参数 | 返回 | 用途 |
|---|---|---|---|
| `get_context` | 无 | `ContextView` | 前端初始化拉取 |
| `set_scenario_override` | `scenario: Option<String>` | `ContextView` | 手动纠正；None=回到自动；广播 context-changed |

`ContextView = { scenario, label, source: "auto"|"manual", title, processName }`（serde camelCase）。

---

## 四、阶段 A：情境中枢 + 桌宠情境徽标（可直接执行）

### Task A1：新建 context.rs（纯函数 + 单元测试）

- [ ] 1. 创建 `apps/overlay/src/context.rs`，写入：

```rust
//! 情境中枢：把前台窗口（进程名 + 标题 + 是否全屏）归类为六种情境。
//! 只使用进程名与窗口标题做规则分类，不读取、不落盘任何用户内容。

/// 六种情境，对应 OwO 文档功能①的情境模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scenario {
    Writing,
    Chat,
    Coding,
    Study,
    Dnd,
    General,
}

impl Scenario {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Scenario::Writing => "写作",
            Scenario::Chat => "沟通",
            Scenario::Coding => "编程",
            Scenario::Study => "学习",
            Scenario::Dnd => "勿扰",
            Scenario::General => "通用",
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Scenario::Writing => "writing",
            Scenario::Chat => "chat",
            Scenario::Coding => "coding",
            Scenario::Study => "study",
            Scenario::Dnd => "dnd",
            Scenario::General => "general",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "writing" => Ok(Scenario::Writing),
            "chat" => Ok(Scenario::Chat),
            "coding" => Ok(Scenario::Coding),
            "study" => Ok(Scenario::Study),
            "dnd" => Ok(Scenario::Dnd),
            "general" => Ok(Scenario::General),
            other => Err(format!("未知情境: {other}")),
        }
    }
}

pub(crate) struct ScenarioInput<'a> {
    pub(crate) process_name: &'a str,
    pub(crate) title: &'a str,
    pub(crate) fullscreen: bool,
}

const CHAT_PROCS: &[&str] = &[
    "qq.exe", "weixin.exe", "wechat.exe", "dingtalk.exe", "dingtalklauncher.exe",
    "feishu.exe", "lark.exe", "telegram.exe", "discord.exe", "ms-teams.exe",
];
const WRITING_PROCS: &[&str] = &[
    "winword.exe", "wps.exe", "et.exe", "wpp.exe",
    "notion.exe", "typora.exe", "obsidian.exe", "notepad.exe",
];
const CODING_PROCS: &[&str] = &[
    "code.exe", "cursor.exe", "idea64.exe", "idea.exe", "pycharm64.exe",
    "clion64.exe", "goland64.exe", "webstorm64.exe", "devenv.exe",
    "windowsterminal.exe", "openconsole.exe", "conhost.exe",
    "powershell.exe", "pwsh.exe", "cmd.exe",
];
const BROWSER_PROCS: &[&str] = &[
    "chrome.exe", "msedge.exe", "firefox.exe", "360se.exe",
    "360chrome.exe", "qqbrowser.exe", "sogouexplorer.exe", "brave.exe",
];
const STUDY_TITLE_KEYWORDS: &[&str] = &[
    "文档", "论文", "教程", "课程", "学习", "笔记",
    "wiki", "docs", "stack overflow", "知乎", "csdn", "github", "mdn",
];

/// 纯函数分类：全屏 > IM > 办公写作 > 开发 > 浏览器(标题细分) > 通用。
pub(crate) fn classify(input: ScenarioInput<'_>) -> Scenario {
    if input.fullscreen {
        return Scenario::Dnd;
    }
    let proc = input.process_name.to_ascii_lowercase();
    let title = input.title.to_lowercase();
    let is = |list: &[&str]| list.iter().any(|p| proc.ends_with(p));
    if is(CHAT_PROCS) {
        return Scenario::Chat;
    }
    if is(WRITING_PROCS) {
        return Scenario::Writing;
    }
    if is(CODING_PROCS) {
        return Scenario::Coding;
    }
    if is(BROWSER_PROCS) {
        if STUDY_TITLE_KEYWORDS.iter().any(|k| title.contains(k)) {
            return Scenario::Study;
        }
        return Scenario::General;
    }
    Scenario::General
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(proc: &str, title: &str, fullscreen: bool) -> Scenario {
        classify(ScenarioInput { process_name: proc, title, fullscreen })
    }

    #[test]
    fn chat_app_maps_to_chat() {
        assert_eq!(s("QQ.exe", "工作群 - QQ", false), Scenario::Chat);
    }

    #[test]
    fn fullscreen_maps_to_dnd_even_for_chat() {
        assert_eq!(s("qq.exe", "QQ", true), Scenario::Dnd);
    }

    #[test]
    fn editor_maps_to_writing() {
        assert_eq!(s("winword.exe", "周报.docx - Word", false), Scenario::Writing);
    }

    #[test]
    fn ide_maps_to_coding() {
        assert_eq!(s("Code.exe", "main.rs - lingxi - Visual Studio Code", false), Scenario::Coding);
    }

    #[test]
    fn browser_with_study_title_maps_to_study() {
        assert_eq!(s("msedge.exe", "Rust 教程 - 简明教程", false), Scenario::Study);
    }

    #[test]
    fn browser_without_study_title_maps_to_general() {
        assert_eq!(s("msedge.exe", "百度一下，你就知道", false), Scenario::General);
    }

    #[test]
    fn unknown_proc_maps_to_general() {
        assert_eq!(s("explorer.exe", "此电脑", false), Scenario::General);
    }
}
```

- [ ] 2. `apps/overlay/src/main.rs` 模块声明区（`mod agent;` 之后）加入 `mod context;`
- [ ] 3. 验证：`cd d:\working OWOWOWOWOWO\LingXi-DesktopAgent\apps\overlay` 后运行 `cargo test context::` → 预期 `7 passed`
- [ ] 4. `git add -A && git commit -m "feat(context): 情境分类纯函数 Scenario/classify 与单元测试"`

### Task A2：AppState 扩展 + 两个命令

- [ ] 1. `apps/overlay/src/state.rs`：顶部加 `use crate::context::Scenario;`；`AppState` 的 `lens_image` 字段之后追加：

```rust
    /// 采样线程最近识别的情境（自动模式下的当前值）。
    pub(crate) scenario_detected: std::sync::Mutex<Scenario>,
    /// 用户手动覆盖的情境；Some 期间采样线程不更新 detected（OwO 功能①手动纠正）。
    pub(crate) scenario_override: std::sync::Mutex<Option<Scenario>>,
    /// 最近一次采样的前台窗口元数据 (进程名, 窗口标题)，仅供展示。
    pub(crate) scenario_window: std::sync::Mutex<(String, String)>,
```

- [ ] 2. `state.rs` 的 `impl Default for AppState` 中 `lens_image: Mutex::new(None),` 之后追加：

```rust
            scenario_detected: std::sync::Mutex::new(Scenario::General),
            scenario_override: std::sync::Mutex::new(None),
            scenario_window: std::sync::Mutex::new((String::new(), String::new())),
```

- [ ] 3. `apps/overlay/src/context.rs` 顶部补导入并追加视图与命令：

```rust
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::{AppState, MutexExt};

/// 情境视图：前端唯一的事实来源。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextView {
    pub(crate) scenario: &'static str,
    pub(crate) label: &'static str,
    pub(crate) source: &'static str,
    pub(crate) title: String,
    pub(crate) process_name: String,
}

pub(crate) fn context_view(state: &AppState) -> ContextView {
    let (scenario, source) = {
        let override_lock = state.scenario_override.safe_lock();
        match *override_lock {
            Some(s) => (s, "manual"),
            None => (*state.scenario_detected.safe_lock(), "auto"),
        }
    };
    let (process_name, title) = state.scenario_window.safe_lock().clone();
    ContextView {
        scenario: scenario.as_str(),
        label: scenario.label(),
        source,
        title,
        process_name,
    }
}

#[tauri::command]
pub(crate) fn get_context(state: State<'_, AppState>) -> ContextView {
    context_view(state.inner())
}

#[tauri::command]
pub(crate) fn set_scenario_override(
    app: AppHandle,
    state: State<'_, AppState>,
    scenario: Option<String>,
) -> Result<ContextView, String> {
    let parsed = match scenario.as_deref() {
        None | Some("") => None,
        Some(value) => Some(Scenario::parse(value)?),
    };
    *state.scenario_override.safe_lock() = parsed;
    let view = context_view(state.inner());
    let _ = app.emit("context-changed", &view);
    Ok(view)
}
```

- [ ] 4. `apps/overlay/src/main.rs` 的 `generate_handler!` 中 `agent::toggle_tool,` 之后插入两行：

```rust
            context::get_context,
            context::set_scenario_override,
```

- [ ] 5. 验证：`cargo build` → 预期 `Finished`（若报 `Emitter` 未找到，确认 tauri 2 的 `use tauri::Emitter;` 已导入；这是 Tauri v2 的已知坑：`emit` 是 trait 方法）
- [ ] 6. `git add -A && git commit -m "feat(context): AppState 情境字段与 get_context/set_scenario_override 命令"`

### Task A3：采样线程 + 全屏检测

- [ ] 1. `apps/overlay/src/context.rs` 末尾追加：

```rust
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

/// 窗口矩形完全覆盖显示器矩形即视为全屏（视频/演示/游戏中 → 勿扰）。
fn is_fullscreen(hwnd: isize) -> bool {
    unsafe {
        let hwnd = HWND(hwnd);
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT::default(),
            rcWork: RECT::default(),
            dwFlags: 0,
        };
        if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
            return false;
        }
        rect.left <= mi.rcMonitor.left
            && rect.top <= mi.rcMonitor.top
            && rect.right >= mi.rcMonitor.right
            && rect.bottom >= mi.rcMonitor.bottom
    }
}

/// 500ms 轮询前台窗口（与 qq::spawn_qq_foreground_sampler 同节奏）。
/// 只读进程名/标题/全屏状态，不读取任何文本内容。
pub(crate) fn spawn_context_sampler(app: tauri::AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(500));
        sample_once(&app);
    });
}

fn sample_once(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    if state.scenario_override.safe_lock().is_some() {
        return;
    }
    let info = match assistant_windows::foreground_info() {
        Ok(info) => info,
        Err(_) => return,
    };
    {
        let mut window = state.scenario_window.safe_lock();
        *window = (info.process_name.clone(), info.title.clone());
    }
    let next = classify(ScenarioInput {
        process_name: &info.process_name,
        title: &info.title,
        fullscreen: is_fullscreen(info.hwnd),
    });
    let mut detected = state.scenario_detected.safe_lock();
    if *detected != next {
        *detected = next;
        drop(detected);
        let view = context_view(state.inner());
        let _ = app.emit("context-changed", &view);
    }
}
```

- [ ] 2. `apps/overlay/src/main.rs` setup 中 `qq::spawn_qq_foreground_sampler();` 之后加：

```rust
            context::spawn_context_sampler(app.handle().clone());
```

- [ ] 3. 验证：`cargo build` → `Finished`。若 `GetWindowRect` 在当前 windows 0.58 版本返回 `BOOL` 而非 `Result`，按编译器提示把 `.is_err()` 改为 `== false`（`.as_bool()` 风格）——两种 API 形态都是 0.5x 的正常变体
- [ ] 4. `git add -A && git commit -m "feat(context): 500ms 全前台采样线程与全屏勿扰检测"`

### Task A4：桌宠情境徽标 + 模式菜单

- [ ] 1. `apps/overlay/ui/pet.html`：`skin-menu` div 之后加一行（chip 与菜单都放 body 层，避免嵌进 `<button>` 造成按钮嵌按钮）：

```html
    <div class="scenario-chip" id="scenario-chip" hidden role="button"></div>
    <div class="scenario-menu" id="scenario-menu" hidden></div>
```

- [ ] 2. `apps/overlay/ui/pet.css` 末尾追加（复刻 skin-menu 的去AI感配色：中性深底 + 单一强调色 #4cc2ff）：

```css
/* ---- 情境徽标 / 模式菜单 ---- */
.scenario-chip {
  position: fixed;
  top: 6px;
  right: 10px;
  z-index: 15;
  padding: 2px 9px;
  font-size: 11px;
  line-height: 1.5;
  color: #ececf0;
  background: rgba(32, 32, 32, .92);
  border: 1px solid rgba(255, 255, 255, .1);
  border-radius: 999px;
  cursor: pointer;
  user-select: none;
}
.scenario-chip[hidden] { display: none; }
.scenario-menu {
  position: fixed;
  left: 50%;
  bottom: 10px;
  transform: translateX(-50%);
  z-index: 21;
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 132px;
  padding: 6px;
  background: rgba(32, 32, 32, .95);
  border: 1px solid rgba(255, 255, 255, .1);
  border-radius: 10px;
  box-shadow: 0 10px 28px rgba(0, 0, 0, .45);
}
.scenario-menu[hidden] { display: none; }
.scenario-item {
  padding: 5px 10px;
  text-align: left;
  color: #ececf0;
  font-size: 12px;
  background: transparent;
  border: 0;
  border-radius: 6px;
  cursor: pointer;
}
.scenario-item:hover { background: rgba(255, 255, 255, .07); }
.scenario-item.is-active { background: rgba(76, 194, 255, .14); }
```

- [ ] 3. `apps/overlay/ui/pet.js` 末尾追加整段（文件头已有 `invoke`/`listen` 帮助函数；`sayTemp` 是函数声明会提升，可直接引用）：

```js
// ---- 情境徽标 / 模式菜单（OwO 功能①⑥：AI 理解状态 + 手动纠正）----
const scenarioChip = document.getElementById("scenario-chip");
const scenarioMenu = document.getElementById("scenario-menu");

const SCENARIO_OPTIONS = [
  { id: "", label: "自动（跟随应用）" },
  { id: "writing", label: "写作" },
  { id: "chat", label: "沟通" },
  { id: "coding", label: "编程" },
  { id: "study", label: "学习" },
  { id: "dnd", label: "勿扰" },
  { id: "general", label: "通用" },
];

function applyContext(view) {
  if (!view || !view.label) return;
  scenarioChip.hidden = false;
  scenarioChip.textContent = view.label;
}

function renderScenarioMenu(view) {
  const active = view && view.source === "manual" ? view.scenario : "";
  scenarioMenu.replaceChildren();
  for (const opt of SCENARIO_OPTIONS) {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "scenario-item" + (opt.id === active ? " is-active" : "");
    item.textContent = opt.label;
    item.addEventListener("click", async () => {
      scenarioMenu.hidden = true;
      if (!invoke) return;
      try {
        applyContext(await invoke("set_scenario_override", { scenario: opt.id || null }));
      } catch {
        sayTemp("切换失败");
      }
    });
    scenarioMenu.appendChild(item);
  }
  scenarioMenu.hidden = false;
}

scenarioChip.addEventListener("click", () => {
  if (!scenarioMenu.hidden) {
    scenarioMenu.hidden = true;
    return;
  }
  if (invoke) invoke("get_context").then(renderScenarioMenu).catch(() => {});
});

document.addEventListener("mousedown", (event) => {
  if (!scenarioMenu.hidden && !scenarioMenu.contains(event.target) && event.target !== scenarioChip) {
    scenarioMenu.hidden = true;
  }
});

window.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !scenarioMenu.hidden) scenarioMenu.hidden = true;
});

if (invoke) {
  invoke("get_context").then(applyContext).catch(() => {});
  if (listen) listen("context-changed", (event) => applyContext(event.payload));
}
```

- [ ] 4. 验证：`cargo build` 后运行 `.\target\debug\overlay.exe` → 桌宠右上角出现情境 chip；点击 chip 弹出七项菜单且不触发"单击打开面板"（chip 在 body 层，pet 的 pointer 事件监听不涉及它）
- [ ] 5. `git add -A && git commit -m "feat(pet): 情境徽标与手动纠正菜单"`

### Task A5：主面板情境徽标

- [ ] 1. `apps/overlay/ui/index.html` 第 52 行 `<span class="backend-badge" id="backend-badge">本地</span>` 之后加：

```html
        <span class="backend-badge" id="scenario-badge" hidden title="当前情境，点击桌宠徽标可手动切换"></span>
```

- [ ] 2. `apps/overlay/ui/app.js` 末尾追加（复用 L1221 的 `TAURI.event.listen` 模式）：

```js
// ---- 情境徽标（OwO 功能⑥）----
const scenarioBadge = document.getElementById("scenario-badge");
function applyScenarioBadge(view) {
  if (!scenarioBadge || !view || !view.label) return;
  scenarioBadge.hidden = false;
  scenarioBadge.textContent = view.label;
  scenarioBadge.style.opacity = view.source === "manual" ? "1" : "";
}
if (TAURI && TAURI.event && TAURI.event.listen) {
  TAURI.event.listen("context-changed", (event) => applyScenarioBadge(event.payload));
}
if (TAURI) {
  TAURI.core.invoke("get_context").then(applyScenarioBadge).catch(() => {});
}
```

- [ ] 3. 验证：`cargo build` + 运行 → 面板标题行出现情境标签，与桌宠 chip 同步变化
- [ ] 4. `git add -A && git commit -m "feat(panel): 主面板情境徽标"`

### Task A6：阶段 A 真机验收

- [ ] 1. `cd apps\overlay && cargo test` → 全部通过（含既有测试）
- [ ] 2. 运行 overlay，依次验证并记录：
  - 聚焦 QQ/微信 → 桌宠 chip 变「沟通」；
  - 聚焦 VS Code → 变「编程」；
  - 聚焦 Word/WPS → 变「写作」；
  - 视频播放器全屏 → 变「勿扰」；
  - 点击 chip → 菜单选「勿扰」→ 切到 QQ 后 chip 仍为「勿扰」（覆盖生效）；再选「自动」→ 恢复跟随；
  - F12 或日志确认 500ms 轮询无报错、无事件风暴（仅变化时发事件）。
- [ ] 3. 更新 docs/project-status.md 增补"情境中枢已落地"一段；`git add -A && git commit -m "docs: 阶段A情境中枢验收记录"`

---

## 五、阶段 B：主动建议卡 + 文件拖拽（OwO 功能⑦⑧）

### Task B1：suggestions.rs 规则引擎（新建 `apps/overlay/src/suggestions.rs`）

设计（执行时按 A 的粒度展开为步骤）：

```rust
pub(crate) struct Suggestion {
    pub(crate) id: &'static str,   // "translate_en" | "analyze_error" | "idle_break" | "time_todo"
    pub(crate) title: String,
    pub(crate) detail: String,
}

pub(crate) struct SuggestionPrefs {
    // 持久化到 app 配置目录 suggestion_prefs.json
    pub(crate) cooldown_until: HashMap<String, i64>, // 每规则下次可提示时间戳(ms)
    pub(crate) disabled: Vec<String>,                // 用户点"不再提示"
    pub(crate) last_text: String,                    // 同一文本 10 分钟内不重复提示
}
```

- 初始 4 条规则：剪贴板为纯英文句子→`translate_en`；剪贴板含 `error/Exception/Traceback/错误` →`analyze_error`；无操作 >45 分钟→`idle_break`；剪贴板为时间/日期表达→`time_todo`；
- 触发入口：复用 `widgets.rs` 已有的 1.5s 剪贴板轮询（L918-950），钩子调用 `suggestions::on_clipboard(app, text)`；idle 用前台标题 + 系统空闲时间 `GetLastInputInfo`；
- 冷却：每规则默认 30 分钟，命中后写回 prefs 并落盘。

### Task B2：建议卡 UI（pet.html/pet.css/pet.js）

- pet.html 加 `<div class="suggestion-card" id="suggestion-card" hidden>`；卡内：标题、一句话说明、三个按钮 **使用一次 / 忽略 / 不再提示**（OwO 功能⑦原文的三个动作）；
- 后端命令：`suggestion_action { id, action: "once"|"ignore"|"disable" }`；`once` 直接执行（translate_en→打开翻译小工具并带入文本；analyze_error→切面板对话视图并预填分析 prompt）；`ignore` 冷却 1 小时；`disable` 写入 disabled 永久关闭；
- 展示时机：后端命中规则后 `emit_to("pet", "suggestion-new", suggestion)`，pet.js 监听显示卡片；同一时刻只显示一张，新建议替换旧建议。

### Task B3：文件拖给桌宠（OwO 功能⑧）

- pet.js 监听 Tauri v2 拖拽事件：`listen("tauri://drag-drop", (e) => invoke("pet_file_dropped", { paths: e.payload.paths }))`（注意 v2 的 payload 是 `{ paths, position }`，且 dragDropEnabled 默认开启时 HTML5 drop 事件不会触发——必须走 `tauri://drag-drop` 事件（由 Tauri 拦截层转发，payload 为 `{ paths, position }`）。

**步骤 2 — 新建 `apps/overlay/src/file_drop.rs`**（完整文件）：

```rust
//! 桌宠拖拽文件分发：图片→WinRT OCR，文本/代码→解释预填，其余→宠物提示。

use serde::Serialize;
use tauri::{AppHandle, Emitter};

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp", "gif"];
const TEXT_EXT: &[&str] = &[
    "rs", "py", "js", "ts", "jsx", "tsx", "java", "c", "cpp", "h", "go", "md",
    "txt", "log", "json", "toml", "yaml", "yml", "html", "css", "csv",
];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileDropView {
    pub path: String,
    pub content: String,
}

fn ext_of(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

#[tauri::command]
pub(crate) async fn pet_file_dropped(
    app: AppHandle,
    paths: Vec<String>,
) -> Result<(), String> {
    let Some(path) = paths.first().cloned() else {
        return Ok(());
    };
    let ext = ext_of(&path);
    if IMAGE_EXT.contains(&ext.as_str()) {
        let p = path.clone();
        let lines = tauri::async_runtime::spawn_blocking(move || {
            crate::widgets::run_winrt_ocr(std::path::Path::new(&p))
        })
        .await
        .map_err(|e| format!("OCR 任务失败: {e}"))??;
        let text = lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = app.emit_to(
            "panel",
            "pet-file-text",
            FileDropView {
                path,
                content: format!("（以下文字提取自图片 OCR）\n{text}"),
            },
        );
        return Ok(());
    }
    if TEXT_EXT.contains(&ext.as_str()) {
        let content = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {e}"))?;
        let content: String = content.chars().take(4000).collect();
        let _ = app.emit_to("panel", "pet-file-text", FileDropView { path, content });
        return Ok(());
    }
    let _ = app.emit_to(
        "pet",
        "pet-say",
        "这个文件类型我还不会读，试试拖 txt、代码文件或图片",
    );
    Ok(())
}
```

**步骤 3 — 接线**：
- widgets.rs L340：`fn run_winrt_ocr(` → `pub(crate) fn run_winrt_ocr(`（供 file_drop 复用 WinRT OCR 管线）
- main.rs mod 列表（L20-36 区）加一行 `mod file_drop;`；`generate_handler!` 中 `agent::toggle_tool,` 后加 `file_drop::pet_file_dropped,`

**步骤 4 — 前端 pet.js 末尾追加**（复用文件头已有的 invoke/listen 帮助函数，pet.js L3-5）：

```js
if (listen) {
  listen("tauri://drag-drop", (e) => {
    const paths = e.payload && e.payload.paths;
    if (paths && paths.length && invoke) invoke("pet_file_dropped", { paths });
  });
  listen("pet-say", (e) => { if (e.payload) sayTemp(String(e.payload)); });
}
```

（`sayTemp` 是 pet.js L365 的函数声明，可提升引用，无需调整。）

**步骤 5 — 前端 app.js 末尾追加**：

```js
if (TAURI && TAURI.event && TAURI.event.listen) {
  TAURI.event.listen("pet-file-text", (e) => {
    const d = e.payload || {};
    if (!el.chatInput) return;
    el.chatInput.value = "帮我看看这个文件：" + d.path + "\n\n" + (d.content || "");
    el.chatInput.dispatchEvent(new Event("input"));
    el.chatInput.focus();
  });
}
```

**步骤 6 — 验证与提交**：
- [ ] `cd apps/overlay; cargo check` 通过
- [ ] 真机：拖一张截图到桌宠 → 面板聊天框出现 OCR 提取文本与解释预填；拖一个 .rs 文件 → 出现「帮我看看这个文件」预填；拖 .zip → 桌宠气泡提示不支持
- [ ] `git add -A && git commit -m "feat(overlay): pet drag-drop file dispatch (OCR/text)"`

### Task B4：建议引擎接线（剪贴板 → 建议卡）

**步骤 1 — main.rs**：mod 列表加 `mod suggestions;`；`generate_handler!` 中 `file_drop::pet_file_dropped,` 后加 `suggestions::suggestion_action,`。

**步骤 2 — widgets.rs 剪贴板监听接线**：`spawn_clipboard_listener`（L918）签名追加 `app: tauri::AppHandle` 参数；在循环内新文本通过去重检查后（`history.insert(0, …)` 之前）插入：

```rust
crate::suggestions::on_clipboard(&app, &raw);
```

同步把 main.rs setup 中调用点改为 `widgets::spawn_clipboard_listener(app.handle().clone());`。

**步骤 3 — 验证与提交**：
- [ ] `cd apps/overlay; cargo check` 通过
- [ ] 真机：复制一段含 `error` 的英文报错 → 30 秒内桌宠旁出现「分析报错」建议卡；点「不再提示」后同类建议静默（B2 的 suggestion_action 生效）
- [ ] `git add -A && git commit -m "feat(overlay): wire suggestion engine to clipboard watcher"`

## 五、阶段 C：表达升级（对齐「统一产品入口」与「情境感知表达」）

### Task C1：10 种风格矩阵

**背景**：index.html L57-59 现有 polish/proofread/prompt-enhance 三个模式 chip；后端 `model_task()`（rewrite.rs L35-42）只认这三个 mode，其余走 `transformer_by_name` 本地规则（L71-73）。新增风格全部走模型，用 `ModelTask::Polish` 承载风格指令。

**步骤 1 — rewrite.rs**：在 `fn model_task`（L42）之后新增风格表与查询函数：

```rust
/// OwO 对齐：风格矩阵（mode 名 → 指令），全部经 ModelTask::Polish 承载。
const STYLE_PROMPTS: &[(&str, &str)] = &[
    ("formal", "把下面的文字改得更正式、更书面，保持原意"),
    ("polite", "把下面的文字改得更礼貌委婉，适合职场沟通"),
    ("concise", "把下面的文字压缩得更简短，只保留关键信息"),
    ("casual", "把下面的文字改得更口语化、更自然"),
    ("expand", "把下面的文字适当扩写，补充合理细节，不要编造事实"),
    ("shorten", "把下面的文字缩写到一半以内的长度"),
    ("translate_en", "把下面的文字翻译成地道英文，只输出译文"),
    ("grammar", "修正下面的文字中的语病和错别字，尽量少改动"),
    ("academic", "把下面的文字改写成学术风格，严谨客观"),
    ("social", "把下面的文字改写成适合社交媒体发布的风格"),
];

fn style_prompt(mode: &str) -> Option<&'static str> {
    STYLE_PROMPTS.iter().find(|(k, _)| *k == mode).map(|(_, v)| *v)
}
```

**步骤 2 — transform_text 接入**（替换 rewrite.rs L67-74 的函数体开头部分）：

```rust
fn transform_text(settings: &BackendSettings, mode: &str, input: &str) -> Result<String, String> {
    if let Some(task) = model_task(mode) {
        return run_model(settings, task, input);
    }
    if let Some(prompt) = style_prompt(mode) {
        return run_model(settings, ModelTask::Polish, &format!("{prompt}：\n{input}"));
    }
    transformer_by_name(mode)
        .map(|transformer| transformer.transform(input))
        .ok_or_else(|| format!("unknown mode: {mode}"))
}
```

说明：`preview_transform` 的模型任务判断（L123）、spawn_blocking 与 last_preview 缓存逻辑无需改动——风格模式会走与 polish 相同的管线；唯一差别是 `quality_warning`（L156 `model_task(&mode)` 返回 None）不覆盖风格模式，warning 为 None，可接受。

**步骤 3 — index.html 模式 chips 第二行**：先执行 `Select-String -Path apps/overlay/ui/index.html -Pattern "polish"` 查看现有 chip 的确切 class 写法，然后按相同结构在 L59 现有三 chip 之后追加（class 名以 grep 结果为准，保证 app.js 现有的事件逻辑能选中）：

```html
<button class="mode-chip" data-mode="formal">更正式</button>
<button class="mode-chip" data-mode="polite">更礼貌</button>
<button class="mode-chip" data-mode="concise">更简洁</button>
<button class="mode-chip" data-mode="casual">口语化</button>
<button class="mode-chip" data-mode="expand">扩写</button>
<button class="mode-chip" data-mode="shorten">缩写</button>
<button class="mode-chip" data-mode="translate_en">译英</button>
<button class="mode-chip" data-mode="grammar">改语病</button>
<button class="mode-chip" data-mode="academic">学术</button>
<button class="mode-chip" data-mode="social">社媒</button>
```

若 chips 是 flex 容器，检查 styles.css 中该容器的换行设置，必要时给容器加 `flex-wrap: wrap`。

**步骤 4 — 验证与提交**：
- [ ] `cd apps/overlay; cargo check` 通过
- [ ] 真机：选中一段文字按改写热键 → 点「更简洁」→ diff 预览出现压缩版 → 应用写回原窗口
- [ ] `git add -A && git commit -m "feat(overlay): 10-style rewrite matrix"`

### Task C2：聊天斜杠命令（6 条）

**背景**：聊天发送入口在 app.js L947 `const msg = el.chatInput.value.trim();`（`agent_chat` 调用在 L963）。

**步骤 1 — 抽出公共发送函数**：定位 L947 所在的 async 函数，把「从构造入参到 `invoke("agent_chat", { message: msg })` 再到渲染回复/历史」的主体抽成 `async function sendChatMessage(text)`（内部用 text 替代 msg，其余 loading/渲染逻辑保持不变）；原键盘（L1234 keydown）与按钮入口改为调用它。

**步骤 2 — 发送入口接斜杠**（精确锚点 app.js L947）：

```js
// 旧
const msg = el.chatInput.value.trim();
// 新
const raw = el.chatInput.value.trim();
const slash = parseSlash(raw);
if (slash) { handleSlash(slash); return; }
const msg = raw;
```

**步骤 3 — app.js 末尾追加**：

```js
const SLASH_ALIASES = {
  "/总结": (t) => "请总结以下内容，用不超过 5 条要点：\n" + t,
  "/解释": (t) => "请解释以下内容，通俗一点：\n" + t,
  "/翻译": (t) => "请翻译以下内容（中文译英，英文译中），只输出译文：\n" + t,
  "/改写": (t) => "请改写以下文字，使其更通顺自然，直接给出结果：\n" + t,
  "/执行": (t) => t,
};

function parseSlash(raw) {
  const m = raw.match(/^(\/\S+)(?:\s+([\s\S]+))?$/);
  if (!m) return null;
  const cmd = m[1];
  const rest = (m[2] || "").trim();
  if (cmd === "/宠物") return { cmd, rest };
  const build = SLASH_ALIASES[cmd];
  if (!build || !rest) return null;
  return { cmd, rest, prompt: build(rest) };
}

async function handleSlash(s) {
  if (s.cmd === "/宠物") {
    if (TAURI && TAURI.event && s.rest) TAURI.event.emit("pet-say", s.rest);
    return;
  }
  await sendChatMessage(s.prompt);
}
```

说明：`/宠物` 走 v2 前端事件广播（webview emit 可达 pet 窗口的 `pet-say` 监听，即 B3 步骤 4 添加的那个），无需后端命令。

**步骤 4 — 验证与提交**：
- [ ] 输入 `/解释 闭包` → 聊天返回解释；`/宠物 我在呢` → 桌宠气泡说话
- [ ] 普通消息（无斜杠）行为与改动前完全一致
- [ ] `git add -A && git commit -m "feat(ui): slash commands in chat"`

### Task C3：QQ 快捷回复三变体（正式/自然/简短）

**背景**：现链路 app.js `generateQqDraft`（L800-812）→ `generate_qq_draft` 命令 → `run_model(&settings, ModelTask::ChatReply, &message)`（qq.rs L101）→ 写入 `el.qqDraft`（`#qq-draft` 文本域）。三变体 = 语气后缀。

**步骤 1 — qq.rs**：`generate_qq_draft` 命令签名追加 `style: Option<String>`（Tauri 对 Option 参数自动可选，旧调用不传也能编译）；把 L101 的调用改为拼接语气后缀：

```rust
let style_prompt = match style.as_deref() {
    Some("formal") => "\n\n语气要求：正式得体，适合工作场合。",
    Some("natural") => "\n\n语气要求：轻松自然，像朋友聊天。",
    Some("brief") => "\n\n语气要求：尽量简短，一两句话。",
    _ => "",
};
let reply = crate::rewrite::run_model(
    &settings,
    ModelTask::ChatReply,
    &format!("{message}{style_prompt}"),
)?;
```

（以 qq.rs 内实际变量名为准；核心只有一点：语气后缀拼进 message 后传入 run_model。）

**步骤 2 — index.html**：L88 `.qq-actions` 行之后追加：

```html
<div class="qq-actions">
  <button class="btn ghost qq-style" data-style="formal">正式</button>
  <button class="btn ghost qq-style" data-style="natural">自然</button>
  <button class="btn ghost qq-style" data-style="brief">简短</button>
</div>
```

**步骤 3 — app.js**：`generateQqDraft` 中 L808 的 invoke 改为携带当前选中风格：

```js
const styleBtn = document.querySelector(".qq-style.is-active");
el.qqDraft.value = await invoke("generate_qq_draft", {
  message: qqMessage,
  style: styleBtn ? styleBtn.dataset.style : null,
});
```

app.js 末尾追加切换逻辑（再点一次取消，回到默认语气）：

```js
document.querySelectorAll(".qq-style").forEach((btn) => {
  btn.addEventListener("click", () => {
    const wasActive = btn.classList.contains("is-active");
    document.querySelectorAll(".qq-style").forEach((b) => b.classList.remove("is-active"));
    if (!wasActive) btn.classList.add("is-active");
    generateQqDraft();
  });
});
```

**步骤 4 — 验证与提交**：
- [ ] QQ 选中一条消息 → 读取选中 → 点「简短」→ 草稿明显短于默认；点「正式」→ 语气变化
- [ ] `git add -A && git commit -m "feat(overlay): qq draft tone variants"`

### Task C4：情境注入改写提示（「情境感知表达」落地）

**步骤 1 — rewrite.rs**：`preview_transform`（L117）签名追加 `scenario: Option<String>`；在 `*state.pet_status.safe_lock() = "thinking".into();`（L134）之前，对模型类模式给 text 拼情境后缀：

```rust
let is_style = style_prompt(&mode).is_some();
if scenario.is_some() && (model_task(&mode).is_some() || is_style) {
    let s = scenario.as_deref().unwrap_or("");
    text = format!("{text}\n\n（当前情境：{s}，表达请贴合该场景的习惯）");
}
```

（`text` 参数需改为 `mut text: String`；style 模式同样受益。）`apply_transform`（L185）的**回退分支**（L206-211 重新推理处）做同样拼接；缓存命中路径不用改——预览阶段已含情境，缓存的是拼接后的结果。

**步骤 2 — app.js**：`refreshPreview`（L462）里 `invoke("preview_transform", …)` 前先取一次情境，多传一个参数：

```js
let scenario = null;
try { scenario = (await invoke("get_context")).scenarioLabel; } catch (e) {}
```

然后把 `scenario` 加进 preview_transform 的调用参数（字段名 `scenario`，后端 Option<String> 自动对应；取值用 `label`——A2 ContextView 的字段，`rename_all="camelCase"` 对单词字段不变，所以前端拿到的就是 `label` 而不是 `scenarioLabel`）。

**步骤 3 — 验证与提交**：
- [ ] 前台停在 VSCode 时选中中文注释做「更正式」→ 预览语气贴合技术场景；切到 Word 再试 → 语气偏公文（人工比对两次输出）
- [ ] `cargo check` 通过
- [ ] `git add -A && git commit -m "feat(overlay): scenario-aware rewrite prompt"`

### Task C5：面板续写模式

**步骤 1 — rewrite.rs**：`STYLE_PROMPTS` 表末尾追加一行：

```rust
("continue", "接着下面的文字自然续写一段（不超过 200 字），不要重复原文"),
```

**步骤 2 — app.js**：C2 的 `SLASH_ALIASES` 加一条：

```js
"/续写": (t) => "请续写以下内容，保持风格连贯，直接输出续写部分：\n" + t,
```

**步骤 3 — index.html**：模式 chips 区追加 `<button class="mode-chip" data-mode="continue">续写</button>`（与 C1 第二行同容器即可）。

**步骤 4 — 验证与提交**：
- [ ] 聊天输入 `/续写 <半段文字>` → 返回连贯续写；改写视图选中文字点「续写」→ diff 显示新增段落
- [ ] `git add -A && git commit -m "feat(overlay): continue-writing mode"`

## 六、阶段 D：执行护栏（对齐「三级交互」与「执行闭环」）

设计原则：**默认拒绝（deny-by-default）不放松**；新增的是「看得见的确认」与「可回滚的操作」，而不是放开权限。

### Task D1：敏感情境自动暂停

**设计**：前台是密码管理器/银行/支付类进程，或窗口标题含 password/密码 时进入敏感态——agent 拒绝执行、桌宠 chip 显示「敏感」。只依据进程名与标题，不读任何内容（隐私边界不变）。

**步骤 1 — context.rs 追加**（敏感判定，纯函数可测）：

```rust
/// 敏感进程关键词（密码管理器/银行/支付类）。命中即暂停 agent 执行。
const SENSITIVE_PROCESSES: &[&str] = &[
    "keepass", "keepassxc", "1password", "bitwarden", "lastpass", "enpass",
    "bank", "alipay", "tenpay", "wxpay", "unionpay",
];

pub(crate) fn is_sensitive(process: &str, title: &str) -> bool {
    let p = process.to_ascii_lowercase();
    let t = title.to_ascii_lowercase();
    SENSITIVE_PROCESSES.iter().any(|k| p.contains(k))
        || t.contains("password")
        || title.contains("密码")
}

#[cfg(test)]
mod sensitive_tests {
    use super::is_sensitive;
    #[test]
    fn detects_process_and_title() {
        assert!(is_sensitive("keepassxc.exe", "KeePassXC"));
        assert!(is_sensitive("chrome.exe", "修改密码 - 网站"));
        assert!(!is_sensitive("code.exe", "main.rs - VSCode"));
    }
}
```

- [ ] `cd apps/overlay; cargo test context::` — 原有 7 个 + 新 1 个测试通过

**步骤 2 — state.rs**：`AppState` 加字段（A2 三字段旁）：`pub(crate) scenario_sensitive: std::sync::atomic::AtomicBool,`；`Default` 中对应 `scenario_sensitive: std::sync::atomic::AtomicBool::new(false),`。

**步骤 3 — context.rs 的 sample_once**（A3 已写的采样循环内，拿到 process/title 之后）追加：

```rust
let sensitive = is_sensitive(&process, &title);
if state.scenario_sensitive.swap(sensitive, std::sync::atomic::Ordering::AcqRel) != sensitive {
    let _ = app.emit("context-changed", ());
}
```

**步骤 4 — ContextView 扩展**（A2 的结构体加一个字段，camelCase 序列化自动生效）：`pub sensitive: bool,`；`context_view()` 里从 `state.scenario_sensitive.load(Ordering::Acquire)` 取值。

**步骤 5 — agent.rs 的 agent_chat 守卫**（插在 L74-76 的 API Key 检查块之后）：

```rust
if state
    .scenario_sensitive
    .load(std::sync::atomic::Ordering::Acquire)
{
    return Err("当前处于敏感界面（密码/支付类窗口），灵犀已自动暂停，换个窗口再叫我。".into());
}
```

**步骤 6 — 前端 chip/徽标显示敏感态**：pet.js A4 的 `applyContext(ctx)` 中设置文本前加：

```js
chip.classList.toggle("is-sensitive", !!ctx.sensitive);
if (ctx.sensitive) { chip.textContent = "敏感"; }
```

pet.css 追加：

```css
.scenario-chip.is-sensitive { color: #ff7a7a; border-color: rgba(255, 122, 122, 0.45); }
```

app.js A5 的 `applyScenarioBadge` 同样加 `badge.classList.toggle("is-sensitive", !!ctx.sensitive)`（复用同一 CSS 类）。

**步骤 7 — 验证与提交**：
- [ ] 真机：打开 KeePassXC（或标题含「密码」的页面）→ 桌宠 chip 变「敏感」红字；此时发一条 agent 消息 → 返回自动暂停提示
- [ ] `cd apps/overlay; cargo test context::` 通过；`git add -A && git commit -m "feat(overlay): sensitive-context auto pause"`

### Task D2：交互式确认门控（替代 DenyAll，默认仍拒绝）

**设计**：实现 `ConfirmGate`（crates/tools/src/context.rs L36-38，`fn confirm(&self, request: &ConfirmRequest) -> bool`）的 `OverlayConfirmGate`：dangerous 工具执行前把确认请求广播到面板，用户点「允许/拒绝」或 60 秒超时（按拒绝）后放行/拦截。`confirm_mode` 三档：`deny`（默认，等同现状）/ `ask`（逐次询问）/ `auto`（自动允许，不推荐）。

**步骤 1 — state.rs**：`AppState` 加待确认表：

```rust
#[derive(Default)]
pub(crate) struct PendingConfirms {
    pub(crate) next_id: u64,
    pub(crate) map: std::collections::HashMap<u64, std::sync::mpsc::Sender<bool>>,
}
```

字段 `pub(crate) pending_confirms: Mutex<PendingConfirms>,`（Default 中 `Mutex::new(PendingConfirms::default())`）。

**步骤 2 — settings.rs**：`BackendSettings` 加字段 `pub(crate) confirm_mode: String,`（Default 中 `confirm_mode: "deny".into()`；结构体已有 `#[serde(default)]`（L18），旧 settings.json 缺字段也能反序列化）。`BackendSettingsView` 与 `BackendSettingsInput` 各加 `confirm_mode: String`，get/set 命令按现有 `panel_auto_hide` 字段的写法透传。

**步骤 3 — agent.rs**：导入扩展——L7 改为 `use lingxi_tools::{ConfirmGate, ConfirmRequest, DenyAll, RiskLevel, ToolRegistry};`，顶部加 `use tauri::{AppHandle, Manager};`（若已有则并入）。文件末尾追加门控与解决命令：

```rust
/// 交互式确认门控：把确认请求广播到面板，等待 tool_confirm_resolve；
/// 60 秒无响应按拒绝。deny（默认）/ ask / auto 三档见 settings.confirm_mode。
pub(crate) struct OverlayConfirmGate {
    pub(crate) app: AppHandle,
}

impl ConfirmGate for OverlayConfirmGate {
    fn confirm(&self, request: &ConfirmRequest) -> bool {
        let mode = crate::settings::load_backend_settings().confirm_mode;
        if mode == "auto" {
            return true;
        }
        if mode != "ask" {
            return false;
        }
        let state = self.app.state::<AppState>();
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        let id = {
            let mut pending = state.pending_confirms.safe_lock();
            pending.next_id += 1;
            let id = pending.next_id;
            pending.map.insert(id, tx);
            id
        };
        let _ = self.app.emit(
            "tool-confirm-request",
            serde_json::json!({
                "id": id,
                "toolName": request.tool_name,
                "summary": request.action_summary,
                "riskLevel": format!("{:?}", request.risk_level),
            }),
        );
        let allowed = rx
            .recv_timeout(std::time::Duration::from_secs(60))
            .unwrap_or(false);
        state.pending_confirms.safe_lock().map.remove(&id);
        let _ = self.app.emit("tool-confirm-done", serde_json::json!({ "id": id }));
        allowed
    }
}

#[tauri::command]
pub(crate) fn tool_confirm_resolve(
    app: tauri::AppHandle,
    id: u64,
    allow: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let tx = state
        .pending_confirms
        .safe_lock()
        .map
        .remove(&id)
        .ok_or("确认请求不存在或已超时")?;
    let _ = tx.send(allow);
    Ok(())
}
```

注意：`emit` 是 trait 方法，agent.rs 需 `use tauri::Emitter;`（v2 的坑，A2 已遇到过）。

**步骤 4 — 替换 agent.rs L100-103**：`agent_chat` 签名追加 `app: tauri::AppHandle`（放 `state` 参数前）；原「DenyAll」三行注释 + L103 一行替换为：

```rust
// 交互式确认门控：deny（默认，等同原 DenyAll）/ ask / auto。
let confirm =
    std::sync::Arc::new(OverlayConfirmGate { app: app.clone() })
        as std::sync::Arc<dyn ConfirmGate>;
```

同时检查 `DenyAll` 导入是否变为未使用，未使用则从 L7 的 use 中移除。

**步骤 5 — main.rs**：`generate_handler!` 中 `agent::toggle_tool,` 后加 `agent::tool_confirm_resolve,`。

**步骤 6 — 验证与提交**：
- [ ] `cd apps/overlay; cargo check` 通过
- [ ] `git add -A && git commit -m "feat(overlay): interactive confirm gate (deny by default)"`

**步骤 7 — settings.rs 透传（对步骤 2 的落实修正）**：核实后，前端「窗口行为」设置区走的是 `get_window_options`/`set_window_options` 命令（settings.rs L190/L201），不是泛用 settings 视图。confirm_mode 与 `panel_auto_hide` 同路透传：

- `WindowOptionsView`（L183-187）加字段 `pub(crate) confirm_mode: String,`；
- `get_window_options` 的构造处（L192-195）加 `confirm_mode: settings.confirm_mode.clone(),`；
- `set_window_options` 签名（L201-205）加参 `confirm_mode: String,`，在 L209 `panel_remember_position` 赋值后加 `settings.confirm_mode = confirm_mode;`（同一处 `persist_backend_settings(&settings)?`（L210）一并落盘）。

**步骤 8 — index.html**：

① L118「记住面板拖动位置」label 之后加：

```html
<label class="remember-key">危险操作确认
  <select id="confirm-mode">
    <option value="deny">全部拒绝（默认）</option>
    <option value="ask">每次询问</option>
    <option value="auto">自动允许（不推荐）</option>
  </select>
</label>
```

② `</body>` 前（聊天区之后）加确认条：

```html
<div id="confirm-bar" hidden>
  <span id="confirm-text"></span>
  <span class="confirm-actions">
    <button class="btn ghost" id="confirm-deny">拒绝</button>
    <button class="btn" id="confirm-allow">允许本次</button>
  </span>
</div>
```

**步骤 9 — styles.css** 末尾追加（深色浮条，沿用现有 #3a3a3a 边框风格）：

```css
#confirm-bar{position:fixed;left:16px;right:16px;bottom:16px;z-index:60;
  display:flex;align-items:center;justify-content:space-between;gap:12px;
  padding:10px 14px;border:1px solid #3a3a3a;border-radius:8px;
  background:rgba(32,32,32,.95);color:#e8e8e8;font-size:12px;}
#confirm-bar[hidden]{display:none;}
#confirm-text{flex:1;word-break:break-all;}
#confirm-bar .confirm-actions{display:flex;gap:8px;flex-shrink:0;}
#confirm-deny{color:#ff7a7a;border-color:#5a2a2a;}
```

（`display:flex` 会覆盖 `[hidden]` 默认值，故必须带 `#confirm-bar[hidden]` 规则。）

**步骤 10 — app.js 接线**：

① el 映射（L157 `saveWindowOptions` 之后）加：

```js
confirmMode: document.getElementById("confirm-mode"),
confirmBar: document.getElementById("confirm-bar"),
confirmText: document.getElementById("confirm-text"),
confirmAllow: document.getElementById("confirm-allow"),
confirmDeny: document.getElementById("confirm-deny"),
```

② `loadWindowOptions`（L743）try 块内加一行 `el.confirmMode.value = options.confirm_mode || "deny";`；
`saveWindowOptions`（L754）的 invoke 参数对象加 `confirmMode: el.confirmMode.value,`。

③ 文件末尾追加监听与按钮：

```js
let pendingConfirmId = null;
TAURI.event.listen("tool-confirm-request", (e) => {
  pendingConfirmId = e.payload.id;
  el.confirmText.textContent =
    "允许执行？" + e.payload.toolName + "：" + e.payload.summary +
    "（风险：" + e.payload.riskLevel + "）";
  el.confirmBar.hidden = false;
});
TAURI.event.listen("tool-confirm-done", () => {
  pendingConfirmId = null;
  el.confirmBar.hidden = true;
});
el.confirmAllow.addEventListener("click", () => {
  if (pendingConfirmId != null) invoke("tool_confirm_resolve", { id: pendingConfirmId, allow: true });
});
el.confirmDeny.addEventListener("click", () => {
  if (pendingConfirmId != null) invoke("tool_confirm_resolve", { id: pendingConfirmId, allow: false });
});
```

**步骤 11 — 真机验证与提交**：
- [ ] `cd apps/overlay; cargo check` 通过；面板启动无报错，设置下拉回显正确
- [ ] 选「每次询问」保存 → `%APPDATA%/lingxi/settings.json` 中 `confirm_mode` 变 `"ask"`
- [ ] 聊天输入「在我的桌面新建 hello.txt 写入你好」（dangerous 工具）→ 面板底部弹确认条 → 点「拒绝」→ agent 回复工具被拒、无文件；重试点「允许」→ 文件创建成功
- [ ] 切回「全部拒绝」→ 同请求直接拒绝且不弹条；「每次询问」下 60 秒不点 → 超时按拒绝
- [ ] `git add -A && git commit -m "feat(overlay): confirm gate panel UI"`

### Task D3：任务步骤可视化（复用 task-progress 条）

**背景**：index.html L71-79 已有 `task-progress` UI（进度文本 + 步骤条），目前仅模型推理在用。D3 让 agent 多步任务把每步广播出来。

**步骤 1 — 后端**：agent 引擎执行处（agent.rs `run_with_trace` 调用 L110 之后遍历返回 trace 的循环里，或引擎回调处——以 `run_with_trace` 实际返回结构为准）对每个已完成步骤 emit：

```rust
let _ = app.emit(
    "task-progress",
    serde_json::json!({
        "phase": "running",
        "stepIndex": i + 1,
        "stepTotal": trace.len(),
        "label": step.summary,
    }),
);
```

全部结束后补发一条 `{ "phase": "done", "stepTotal": trace.len() }`。若 `run_with_trace` 是完成后才返回 trace，则在 await 后循环补发（视觉上为快速回放，可接受，不阻塞主流程）。

**步骤 2 — app.js**：文件末尾追加：

```js
TAURI.event.listen("task-progress", (e) => {
  const p = e.payload;
  el.taskProgress.hidden = false;
  el.taskStatus.textContent = p.phase === "done"
    ? "任务完成"
    : "步骤 " + p.stepIndex + "/" + p.stepTotal + "：" + p.label;
});
```

（`el.taskProgress`/`el.taskStatus` 已存在于 L133 附近的 el 映射；若名不同以实际 id 为准对齐。）

**步骤 3 — 验证与提交**：
- [ ] 让 agent 执行一个多工具任务（如「先读剪贴板再写文件」）→ 面板出现「步骤 1/2…」→ 完成后显示「任务完成」
- [ ] `git add -A && git commit -m "feat(overlay): agent task progress events"`

### Task D4：危险操作日志与文件级回滚

**步骤 1 — crates/tools**：新增 `op_journal`（或并入现有 fs 工具模块）：`write_file` 执行前，若目标已存在，先复制为 `<target>.bak-<unix_ts>`，并向 `%APPDATA%/lingxi/op-journal.jsonl` 追加一行：

```json
{"ts":1730000000,"op":"write_file","target":"C:/.../hello.txt","backup":"C:/.../hello.txt.bak-1730000000"}
```

新建文件（原不存在）记 `"backup":null`，回滚时改为删除。

**步骤 2 — 暴露 undo 命令**（apps/overlay）：

```rust
#[tauri::command]
pub(crate) fn undo_last_op() -> Result<String, String> {
    let path = op_journal_path();
    let last = read_last_line(&path).ok_or("没有可回滚的操作")?;
    let rec: serde_json::Value = serde_json::from_str(&last).map_err(|e| e.to_string())?;
    let target = rec["target"].as_str().ok_or("记录损坏")?;
    match rec["backup"].as_str() {
        Some(bak) => std::fs::copy(bak, target).map_err(|e| e.to_string())?,
        None => std::fs::remove_file(target).map_err(|e| e.to_string())?,
    }
    Ok(format!("已回滚：{target}"))
}
```

（`read_last_line`：`BufReader` 逐行取最后一行，约 10 行辅助函数。）main.rs 注册 `undo_last_op,`。

**步骤 3 — 面板入口**：index.html 改写区按钮行（apply/undo 现有按钮旁）加 `<button class="btn ghost" id="undo-op">撤销上次文件操作</button>`；app.js 绑定 `invoke("undo_last_op")` → `showStatus(结果, "ok"/"err")`。

**步骤 4 — 验证与提交**：
- [ ] 让 agent 改一个测试文件两次 → 点「撤销上次文件操作」→ 内容回到上一版；op-journal.jsonl 行数与操作数一致
- [ ] `git add -A && git commit -m "feat(tools): op journal with file-level rollback"`

### Task D5：阶段 D 验收

- [ ] KeePassXC 前台时 agent 自动暂停（D1）
- [ ] 「每次询问」模式下危险工具逐次弹条，拒绝生效（D2）
- [ ] 多步任务有步骤进度（D3）
- [ ] 文件写入可一键回滚（D4）
- [ ] 默认配置（deny）下行为与改造前完全一致（回归底线）

## 阶段 E：情境记忆与个性化（OwO 功能⑦ + 创新点⑤）

边界先行：记忆只存「统计值」与「用户显式写下的偏好」，**绝不**存窗口内容、剪贴板文本、聊天记录——隐私边界与阶段 A 相同。

### Task E1：场景停留统计（context-memory.json）

**步骤 1 — context.rs 末尾追加**（路径模式与 agent.rs 的 agent_session_path 一致）：

```rust
/// 情境记忆：只统计各场景采样 tick 数（1 tick = 0.5s），不记录窗口内容。
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct ContextMemory {
    #[serde(default)]
    pub(crate) scenario_ticks: std::collections::HashMap<String, u64>,
}

pub(crate) fn memory_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("lingxi")
        .join("context-memory.json")
}

fn memory_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
        OnceLock::new();
    CACHE.get_or_init(|| {
        std::sync::Mutex::new(
            std::fs::read_to_string(memory_path())
                .ok()
                .and_then(|s| serde_json::from_str::<ContextMemory>(&s).ok())
                .unwrap_or_default()
                .scenario_ticks,
        )
    })
}

fn persist_memory(map: &std::collections::HashMap<String, u64>) {
    let mem = ContextMemory { scenario_ticks: map.clone() };
    let path = memory_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(&mem) {
        let _ = std::fs::write(path, json);
    }
}

#[tauri::command]
pub(crate) fn context_memory_get() -> ContextMemory {
    ContextMemory { scenario_ticks: memory_cache().safe_lock().clone() }
}

#[tauri::command]
pub(crate) fn context_memory_clear() -> Result<(), String> {
    memory_cache().safe_lock().clear();
    persist_memory(&memory_cache().safe_lock());
    Ok(())
}
```

（`safe_lock` 来自 A2 已导入的 `crate::state::MutexExt`，对 `std::sync::Mutex` 同样适用。）

**步骤 2 — sample_once 累计**：A3 代码中 `let mut detected = state.scenario_detected.safe_lock();`（L432）之前插入：

```rust
    // 情境记忆：每 tick 累计 1 次（0.5s），每 120 tick（约 1 分钟）落盘一次。
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TICKS: AtomicU64 = AtomicU64::new(0);
        let mut map = memory_cache().safe_lock();
        *map.entry(next.as_str().to_string()).or_insert(0) += 1;
        if TICKS.fetch_add(1, Ordering::Relaxed) % 120 == 0 {
            persist_memory(&map);
        }
    }
```

（手动 override 期间函数已提前 return，不累计——语义为「自动识别到的停留」。）

**步骤 3 — 注册命令**：main.rs `context::set_scenario_override,` 后加：

```rust
            context::context_memory_get,
            context::context_memory_clear,
```

**步骤 4 — 设置页记忆卡片**：index.html「窗口行为」divider 之前加：

```html
        <div class="settings-divider"><span>情境记忆</span></div>
        <ol id="memory-list" class="memory-list"></ol>
        <button class="btn ghost" id="clear-memory">清除情境记忆</button>
```

app.js 末尾追加：

```js
async function loadMemory() {
  if (!invoke) return;
  try {
    const mem = await invoke("context_memory_get");
    const items = Object.entries(mem.scenarioTicks || {})
      .sort((a, b) => b[1] - a[1]).slice(0, 3);
    el.memoryList.innerHTML = items.length
      ? items.map(([k, v]) => `<li>${labelOf(k)} 约 ${Math.max(1, Math.round(v / 120))} 分钟</li>`).join("")
      : "<li>暂无记录</li>";
  } catch (e) {}
}
function labelOf(key) {
  return { writing: "写作", chat: "沟通", coding: "编程", study: "学习", dnd: "勿扰", general: "通用" }[key] || key;
}
el.clearMemory.addEventListener("click", async () => {
  await invoke("context_memory_clear");
  loadMemory();
});
loadMemory();
```

（el 映射加 `memoryList`/`clearMemory` 两行；serde camelCase 后字段为 `scenarioTicks`。）

**步骤 5 — 验证与提交**：
- [ ] `cargo check` 通过；正常使用 2 分钟后 `%APPDATA%/lingxi/context-memory.json` 出现且每约 1 分钟更新
- [ ] 设置页显示 top3 场景分钟数；点「清除」后归零
- [ ] `git add -A && git commit -m "feat(context): scenario dwell memory"`

### Task E2：个人语气偏好（style_profile 注入）

**步骤 1 — settings.rs**：`BackendSettings`（L19）加 `pub(crate) style_profile: String,`（Default 中 `style_profile: String::new()`；结构体级 `#[serde(default)]` 兜底旧文件）。`BackendSettingsView`（L57）与 `BackendSettingsInput`（L66）各加同名字段；`get_backend_settings`（L116）构造处与 `save_backend_settings`（L128）赋值处各加一行透传（仿现有 `remember_api_key` 字段写法；api_key 那套 skip 掩码逻辑不适用于纯文本，直接存取即可）。

**步骤 2 — 注入推理链路**：

① rewrite.rs：C4 情境注入的同一块之后追加：

```rust
let profile = crate::settings::load_backend_settings().style_profile;
if !profile.is_empty() {
    text = format!("{text}\n\n（用户语气偏好：{profile}）");
}
```

`apply_transform` 的回退分支做同样处理（缓存命中路径天然已含偏好）。

② qq.rs：`generate_qq_draft` 中 C3 的 `style_prompt` 拼接处，把 profile 一并拼入传入 run_model 的文本。

**步骤 3 — 设置 UI**：index.html 模型设置区（API 配置附近）加：

```html
<label class="remember-key">常用语气偏好（如：直接一点，少用敬语）
  <input id="style-profile" type="text" placeholder="留空则不注入" />
</label>
```

app.js：el 映射加 `styleProfile`；`loadSettings`（L575）回显 `el.styleProfile.value = s.style_profile || "";`；`saveSettings`（L618）参数加 `styleProfile: el.styleProfile.value,`（沿用现有「保存模型设置」按钮）。

**步骤 4 — 验证与提交**：
- [ ] 偏好填「直接一点，少用敬语」→ 保存 → settings.json 出现该字段
- [ ] 对同一段文字做「更正式」改写，清空与填写偏好各试一次 → 输出语气有可感知差异
- [ ] `git add -A && git commit -m "feat(overlay): personal style profile injection"`

### Task E3：一键指令（重复指令 → 建议保存 → 一键重放）

**设计**：同一条 agent 指令**成功执行** ≥3 次 → 桌宠建议卡「保存为一键指令」（复用 B1/B2 通道）；保存后设置页列出，点击即重发该指令——重放走完整 agent 链路，危险工具照常过 D2 确认门控，**安全不旁路**。

**步骤 1 — 新建 `apps/overlay/src/flows.rs`**（完整文件）：

```rust
//! 一键指令：保存常用 agent 指令；重放由前端调 agent_chat 完成。

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SavedFlow {
    pub(crate) name: String,
    pub(crate) message: String,
}

pub(crate) fn flows_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("lingxi")
        .join("saved-flows.json")
}

pub(crate) fn load_flows() -> Vec<SavedFlow> {
    std::fs::read_to_string(flows_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn persist_flows(flows: &[SavedFlow]) {
    if let Some(dir) = flows_path().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(flows) {
        let _ = std::fs::write(flows_path(), json);
    }
}

/// 保存（按 message 去重）；供建议卡 once 动作与 agent 计数建议共用。
pub(crate) fn save_flow(message: String) -> SavedFlow {
    let mut flows = load_flows();
    if let Some(existing) = flows.iter().find(|f| f.message == message) {
        return existing.clone();
    }
    let flow = SavedFlow { name: message.chars().take(12).collect(), message };
    flows.push(flow.clone());
    persist_flows(&flows);
    flow
}

#[tauri::command]
pub(crate) fn flow_list() -> Vec<SavedFlow> {
    load_flows()
}

#[tauri::command]
pub(crate) fn flow_delete(index: usize) -> Result<(), String> {
    let mut flows = load_flows();
    if index >= flows.len() {
        return Err("索引越界".into());
    }
    flows.remove(index);
    persist_flows(&flows);
    Ok(())
}
```

**步骤 2 — agent.rs 计数与建议**：文件末尾加辅助：

```rust
fn flow_counts() -> &'static std::sync::Mutex<std::collections::HashMap<u64, u32>> {
    use std::sync::OnceLock;
    static COUNTS: OnceLock<std::sync::Mutex<std::collections::HashMap<u64, u32>>> =
        OnceLock::new();
    COUNTS.get_or_init(Default::default)
}

fn message_hash(message: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    message.trim().hash(&mut hasher);
    hasher.finish()
}
```

`agent_chat` 中 `engine.run_with_trace(...).await`（L110）成功返回后插入：

```rust
    // 一键流程：同一条指令成功执行 ≥3 次 → 建议保存。
    let hash = message_hash(&message);
    let already = crate::flows::load_flows()
        .iter()
        .any(|f| message_hash(&f.message) == hash);
    if !already {
        let mut counts = flow_counts().safe_lock();
        let count = counts.entry(hash).and_modify(|c| *c += 1).or_insert(1);
        if *count >= 3 {
            drop(counts);
            let _ = app.emit_to(
                "pet",
                "suggestion-new",
                crate::suggestions::Suggestion {
                    id: "save_flow",
                    title: "保存为一键指令".into(),
                    detail: message.clone(),
                },
            );
        }
    }
```

（`emit_to` 同样来自 `use tauri::Emitter;`——D2 已加；`id` 字段是 `&'static str`，字面量直接可用。）

**步骤 3 — suggestion_action 分发**：suggestions.rs 的 `once` 动作 match 加分支：

```rust
        "save_flow" => {
            let _ = crate::flows::save_flow(payload_detail); // detail 即原指令全文
        }
```

（以 B1 实现的 id→动作分发结构与变量名为准：动作 = 把建议 detail 存盘。）

**步骤 4 — main.rs**：`mod flows;`；handler 加 `flows::flow_list,` 与 `flows::flow_delete,`。

**步骤 5 — 设置页流程区**：index.html「情境记忆」divider 之后加：

```html
        <div class="settings-divider"><span>一键指令</span></div>
        <ul id="flow-list" class="flow-list"></ul>
```

app.js 末尾（el 映射加 `flowList`）：

```js
async function loadFlows() {
  if (!invoke) return;
  try {
    const flows = await invoke("flow_list");
    el.flowList.innerHTML = flows.length
      ? flows.map((f, i) =>
          `<li><span title="${f.message}">${f.name}</span>` +
          `<button class="btn ghost flow-run" data-i="${i}">执行</button>` +
          `<button class="btn ghost flow-del" data-i="${i}">删除</button></li>`).join("")
      : "<li>暂无（同一条指令成功执行 3 次后会建议保存）</li>";
  } catch (e) {}
}
el.flowList.addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn || !invoke) return;
  const i = Number(btn.dataset.i);
  const flows = await invoke("flow_list");
  if (btn.classList.contains("flow-del")) {
    await invoke("flow_delete", { index: i });
  } else if (btn.classList.contains("flow-run")) {
    await sendChatMessage(flows[i].message); // C2 抽出的带参函数；危险工具仍走确认门控
  }
  loadFlows();
});
loadFlows();
```

**步骤 6 — 验证与提交**：
- [ ] 对 agent 说同一句话 3 次（如「总结剪贴板」）→ 第 3 次后桌宠出现「保存为一键指令」卡 → 点「使用一次」→ 设置页列表出现该指令；重复保存不产生重复项
- [ ] 点「执行」→ 对话视图重新执行该指令；构造含危险工具的指令验证：确认条照常弹出（D2 门控不旁路）
- [ ] `git add -A && git commit -m "feat(overlay): saved one-shot flows"`

### Task E4：深度改写（reader→structurer→writer→reviewer 四角色管线）

**步骤 1 — rewrite.rs `transform_text` 函数体开头**（`model_task(&mode)` 分发之前）加 deep 分支：

```rust
    if mode == "deep" {
        let points = run_model(
            settings,
            ModelTask::Polish,
            &format!("提取以下文字的核心信息点，逐条列出，不要扩写：\n{input}"),
        )?;
        let outline = run_model(
            settings,
            ModelTask::Polish,
            &format!("把这些要点整理成重写大纲，保留原文立场与事实：\n{points}"),
        )?;
        let draft = run_model(
            settings,
            ModelTask::Polish,
            &format!("按大纲重写原文，保持原意，输出完整重写稿：\n原文：{input}\n大纲：{outline}"),
        )?;
        return run_model(
            settings,
            ModelTask::Polish,
            &format!("校对以下重写稿与原意的一致性与通顺度，直接输出最终修正版：\n原文：{input}\n重写稿：{draft}"),
        );
    }
```

（`run_model` 是同步 fn，`preview_transform` 已用 spawn_blocking 包裹 transform_text，4 次串行调用不会卡 UI 线程；预览期间 pet_status 已显示 thinking。）

**步骤 2 — index.html**：C5 的 chips 容器加 `<button class="mode-chip" data-mode="deep">深度改写</button>`。

**步骤 3 — 验证与提交**：
- [ ] 选中一段长中文 → 「深度改写」→ 预览为整体重写版，结构明显比单轮改写更清晰；耗时约为单轮 4 倍属预期
- [ ] `git add -A && git commit -m "feat(overlay): 4-role deep rewrite pipeline"`

## 阶段 F：OwO 集成——本地 IPC 服务（OwO 功能⑬⑭ + 创新点⑨）

分工边界：打字底座（按键、候选、上屏）始终归 OwO/rime；灵犀只暴露**模型能力**（改写/续写/总结）给 OwO 调用。本阶段做完，OwO 侧不再需要内置模型进程。

### Task F1：新建 `apps/overlay/src/ipc_service.rs`（完整文件）

```rust
//! 本地 IPC：127.0.0.1:9528 上的极简 HTTP 服务，供 OwO 等本机进程调用。
//! 仅绑定回环地址（本机边界），无路径写操作，只读模型能力。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

#[derive(serde::Deserialize)]
struct IpcRequest {
    text: String,
    #[serde(default)]
    scenario: Option<String>,
}

#[derive(serde::Serialize)]
struct IpcResponse {
    ok: bool,
    result: String,
}

pub(crate) fn spawn_ipc_server() {
    std::thread::spawn(|| {
        // 端口被占（通常已有实例在跑）→ 静默退出。
        let Ok(listener) = TcpListener::bind("127.0.0.1:9528") else { return };
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || handle(stream));
        }
    });
}

fn handle(mut stream: TcpStream) {
    let outcome = parse_request(&mut stream).and_then(|(path, req)| {
        // /rewrite /complete /summarize 当前同实现，保留路径语义供 OwO 端区分；
        // 后续可按端点映射不同 ModelTask。
        let _ = path;
        let settings = crate::settings::load_backend_settings();
        let mut text = req.text;
        if let Some(s) = &req.scenario {
            text = format!("{text}\n\n（当前情境：{s}，表达请贴合该场景的习惯）");
        }
        crate::rewrite::run_model(&settings, assistant_inference::ModelTask::ChatReply, &text)
    });
    let (code, body) = match outcome {
        Ok(result) => (200u16, IpcResponse { ok: true, result }),
        Err(err) => (500u16, IpcResponse { ok: false, result: err }),
    };
    let _ = respond(&mut stream, code, &body);
}

fn parse_request(stream: &mut TcpStream) -> Result<(String, IpcRequest), String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or("bad request")?;
    let path = parts.next().ok_or("bad request")?.to_string();
    if method != "POST" {
        return Err("仅支持 POST".into());
    }
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).map_err(|e| e.to_string())?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some(v) = header.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().map_err(|_| "bad length")?;
        }
    }
    if content_length > 64 * 1024 {
        return Err("body 过大（上限 64KB）".into());
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    let req: IpcRequest = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    Ok((path, req))
}

fn respond(stream: &mut TcpStream, code: u16, body: &IpcResponse) -> std::io::Result<()> {
    let json = serde_json::to_string(body)?;
    let status = if code == 200 { "OK" } else { "ERR" };
    write!(
        stream,
        "HTTP/1.1 {code} {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
        json.len()
    )
}
```

**接线 — main.rs**：`mod ipc_service;`；setup 中 `context::spawn_context_sampler(...)` 之后加 `ipc_service::spawn_ipc_server();`。

**验证与提交**：
- [ ] `cargo check` 通过；启动后 `Invoke-RestMethod -Uri http://127.0.0.1:9528/rewrite -Method Post -Body '{"text":"帮我把这句话写得更正式：这个事儿得赶紧办","scenario":"writing"}' -ContentType "application/json"` 返回 `{ok:true,result:...}`
- [ ] 未配后端（local 模型缺失）时返回 `{ok:false,...}` 而非进程崩溃；重复启动第二个实例不报错（端口占用静默退出）
- [ ] `git add -A && git commit -m "feat(overlay): local ipc service on 127.0.0.1:9528"`

### Task F2：协议文档 + OwO 侧交接清单

**步骤 1 — 新建 `docs/owo-ipc-protocol.md`**（全文）：

```markdown
# 灵犀 ↔ OwO 本地 IPC 协议 v1

- 地址：`http://127.0.0.1:9528`（仅回环，不监听外网）
- 编码：请求/响应均为 UTF-8 JSON；请求需带 Content-Length
- 上限：body ≤ 64KB

## 端点（POST）
| 路径 | 语义 | 请求体 | 响应 |
| --- | --- | --- | --- |
| /rewrite | 按情境改写 | `{"text":"原文","scenario":"writing|chat|coding|study|general"}` | `{"ok":true,"result":"改写结果"}` |
| /complete | 续写补全 | 同上（scenario 可省略） | 同上 |
| /summarize | 摘要 | 同上 | 同上 |

- `ok:false` 时 `result` 为错误说明（如后端未配置）。
- v1 三端点同实现；后续版本按端点映射不同任务类型，OwO 端无需变更请求格式。

## 安全边界
- 仅本机回环可连；灵犀不读取 OwO 的按键内容，只处理 OwO 显式发来的 text；
- 情境（scenario）由 OwO 按其输入法上下文传入，灵犀只用于语气贴合。

## OwO 侧改造清单（在 OwO 仓库执行，不在本仓库）
1. 输入法 AI 请求统一改为转发上述三个端点（HTTP 客户端即可）；
2. `ime-server` / `ime-repl` / `assistant-ime` 标记 deprecated，过渡一个版本后移除；
3. OwO 侧将当前情境映射为上表 scenario 枚举后随请求携带。
```

**步骤 2 — 验证与提交**：
- [ ] 文档与 F1 实际行为一致（端点/字段/错误形态逐项对照）
- [ ] `git add -A && git commit -m "docs: owo ipc protocol v1"`

## 九、OwO 九大创新点 × 灵犀落地对照

| # | OwO 创新点 | 灵犀落地方式 | 任务 |
|---|---|---|---|
| 1 | 三个独立功能变成一个统一产品 | 桌宠+改写面板+agent 同为一个 Tauri 进程的三个视图；情境徽标把三者挂在同一个「AI 理解状态」下 | A |
| 2 | 从词频预测升级为情境表达 | C4 情境注入 + E2 偏好注入改写/续写/草稿全链路；打字侧经 F1 IPC 获得同样能力 | C/E/F |
| 3 | 用桌宠提升 AI 可解释性 | 徽标显示当前情境、pet_status 显示推理状态、建议卡显示「为什么推荐」 | A/B |
| 4 | 即时辅助、主动建议、长任务三级交互 | 即时=改写矩阵（C）、建议=规则引擎（B）、长任务=agent+进度+确认（D） | B/C/D |
| 5 | 「感知—表达—行动」完整闭环 | A 感知（前台采样）→ C 表达（情境化改写）→ D 行动（工具执行）→ E3 学习（复用沉淀） | A-F |
| 6 | 跨应用延续任务而非重复对话 | 500ms 全前台采样 + context-changed 事件；改写提示随前台应用自动换语气（C4 实测项） | A/C |
| 7 | 用户可纠正的个性化 | 手动情境覆盖（A2）+ 场景记忆可查看/清除（E1）+ 偏好文本可随时改（E2） | A/E |
| 8 | 隐私优先的情境感知 | 只取进程名+标题+全屏布尔；敏感界面只「停」不读（D1）；记忆只存统计值（E1）；IPC 只处理显式发来的 text（F） | D/E/F |
| 9 | 多智能体协作对用户保持简单 | 用户只见一个结果：D3 步骤进度透明化 + E4 四角色深度改写管线（内部多轮、外部单结果） | D/E |

取舍说明：OwO 创新点 9 的「引擎级多智能体编排」不在本期范围——现有 ReAct 引擎（max_steps=10）+ 四角色 prompt 管线已能兑现「对用户保持简单」的产品主张；引擎级编排留作后续演进。

## 十、OwO 四演示场景 × 灵犀验收剧本

以下剧本在全部阶段完成后一次性走查（每阶段完成后可单独预演对应片段）。

### 场景一：课程资料阅读（验证 A4 徽标 + B3 拖拽 + agent）
- [ ] 浏览器打开机器学习教程页 → 桌宠徽标自动变「学习」
- [ ] 将 PDF 拖给桌宠 → WinRT OCR 提取文字 → 面板自动预填「帮我看看这个文件…」
- [ ] 发送 → 返回摘要与关键概念；处理期间桌宠呈 thinking 状态
- 预期失败排查：徽标未变 → 检查 classify 的浏览器+学习标题词规则（A1 单测覆盖）

### 场景二：根据资料完成写作（验证 C1 风格 + C4 情境注入 + E4）
- [ ] 打开 Word → 徽标自动变「写作」
- [ ] 选中刚生成的摘要句子 → 依次试用「学术」「扩写」「深度改写」→ diff 各自生效，且语气比在 QQ 前台时更书面（C4 情境注入的人工比对点）
- [ ] E2 偏好填「学术语气，避免口语」保存后重做一次 → 输出可感知更严谨
- 预期失败排查：两次输出无差异 → 检查 scenario 参数是否传到 preview_transform（C4 步骤 2）

### 场景三：切换到师生沟通（验证 A 自动切换 + C3 草稿变体）
- [ ] 打开 QQ → 徽标自动变「沟通」（无需手动）
- [ ] 选中老师消息「明天下午把实验报告发给我」→ 点 QQ 读取 → 生成草稿
- [ ] 分别点「正式/自然/简短」→ 三版语气差异明显，「正式」版适合回复老师
- [ ] 聊天输入 `/翻译 收到，明天交` → 返回英文（C2 斜杠）
- 预期失败排查：草稿无差异 → 检查 qq-style 按钮的 dataset.style 是否传到后端（C3 步骤 3）

### 场景四：交给智能体完成长任务（验证 D 全部 + E3）
- [ ] 设置里把确认调为「每次询问」
- [ ] 对面板说：「把刚才那份资料的要点整理成提纲，写入 D:/notes/提纲.md」
- [ ] 过程中面板显示步骤进度（D3）；写文件前弹出确认条 → 先点「拒绝」→ agent 报告被拒、无文件；重试并点「允许」→ 文件生成
- [ ] 点「撤销上次文件操作」→ 文件恢复/删除（D4）
- [ ] 同一指令再成功执行 2 次 → 桌宠建议「保存为一键指令」→ 保存后在设置页点「执行」可重放（E3）
- [ ] 任务进行中打开 KeePassXC → agent 返回敏感界面自动暂停提示（D1）
- 预期失败排查：确认条不弹 → 检查 settings.json 的 confirm_mode 是否为 "ask" 且面板监听已注册（D2 步骤 10）

## 十一、风险与依赖

| # | 风险/依赖 | 影响 | 缓解（已内建） |
|---|---|---|---|
| 1 | Tauri v2 `emit` 是 trait 方法 | 编译失败 | 所有新增 emit 处已注明 `use tauri::Emitter;`（A2/D2/E3） |
| 2 | OverlayConfirmGate 在 runtime 线程上 `recv_timeout(60s)` 阻塞 | 极端时占满 worker 线程 | tokio 多 worker 可承受；若实测卡顿，把引擎内 confirm 调用包 `spawn_blocking`（crates/agent 一行改动，接口不变） |
| 3 | windows 0.58 `GetWindowRect` 返回 BOOL/Result 形态差异 | 编译失败 | A3 步骤 3 已给两种写法，按编译器提示二选一 |
| 4 | WinRT OCR 依赖 `LINGXI_OCR_PATH` 与系统语言包 | 拖拽图片失败/首次慢 | file_drop.rs 失败走 `pet-say` 提示而非崩溃；文本类文件不依赖 OCR |
| 5 | 拖拽事件依赖窗口 `dragDropEnabled`（v2 默认开启） | HTML5 drop 不触发 | B3 已按 `tauri://drag-drop` 实现；若 tauri.conf 被改动需复核该项 |
| 6 | apps/overlay 是 MSVC 单独构建（workspace exclude） | 在仓库根跑 check 不会编译 overlay | 所有验证命令统一 `cd apps/overlay; cargo check` |
| 7 | 风格/续写模式复用 Polish 承载指令 | quality_warning 不覆盖风格模式 | 接受：预览 diff 本身可让用户判断质量 |
| 8 | 9528 端口被占（多实例） | 第二个实例 IPC 不可用 | F1 端口占用静默退出，功能由首实例承担 |
| 9 | QQ 采样与情境采样双 500ms 线程并存 | 轮询开销 | 均为纯 Win32 只读调用，开销可忽略；后续可合并为单线程 |
| 10 | 旧 settings.json 兼容 | 升级后启动失败 | BackendSettings 结构体级 `#[serde(default)]`（L18）兜底所有新增字段 |

## 十二、方案自审清单（交付前逐项核对）

- [ ] 差距矩阵 14 行：每行都有对应任务与阶段（①A ②C/F ③C ④C ⑤E ⑥A ⑦B ⑧B ⑨C ⑩D ⑪E ⑫A/E ⑬E ⑭D）
- [ ] 九创新点对照表 9 行全部有落地任务；创新点 9 的取舍已显式说明
- [ ] 四演示场景剧本与任务编号一一对应，且每场景含失败排查指引
- [ ] 全文无 TODO/占位符/伪代码（grep `TODO\|待补\|占位\|\.\.\.` 复核；代码块内 `…` 仅允许出现在文档叙述行）
- [ ] generate_handler 注册清单完整：既有命令 + `get_context` / `set_scenario_override` / `context_memory_get` / `context_memory_clear` / `pet_file_dropped` / `suggestion_action` / `tool_confirm_resolve` / `undo_last_op` / `flow_list` / `flow_delete`
- [ ] 前端 id 与 el 映射一致：`confirm-mode`/`confirm-bar`/`confirm-text`/`confirm-allow`/`confirm-deny`/`memory-list`/`clear-memory`/`flow-list`/`style-profile`
- [ ] `Scenario::as_str()` 六值与 app.js `labelOf` 键、OwO IPC scenario 枚举一致：`writing/chat/coding/study/dnd/general`
- [ ] 每个任务有：bite-sized 步骤、完整代码、验证命令、独立 commit message
- [ ] 阶段间无前向依赖：A 可独立交付；B/C 依赖 A；D 依赖 A（D2 依赖 A 的 AppState）；E 依赖 A/B/C；F 依赖 A
- [ ] 隐私与安全底线未被突破：无读取窗口文本、无键盘监听、deny 默认、重放走完整确认链

## 交付与执行方式

方案至此完整（阶段 A-F 全部任务含代码与验证）。三种推进方式：

1. **Subagent-Driven 执行阶段 A**：把 A1-A6 各 Task 分派给执行子代理（每 Task 一个代理，按文档步骤逐项执行并回报验证输出），我负责派发与验收；
2. **Inline 执行阶段 A**：我在当前会话按 A1→A6 顺序逐任务实施（每任务 cargo check + commit 后再进入下一个）；
3. **仅评审**：先人工评审本文档，把修改意见给我迭代后再执行。

默认建议：先 Inline 执行阶段 A（情境中枢是后续一切的地基，且 A1-A6 均有精确锚点，单会话可完成），A 完成后再评审决定 B-F 的推进节奏。