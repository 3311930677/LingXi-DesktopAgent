# 项目状态交接文档

> 更新时间：2026-08-25 晚 | 仓库：https://github.com/3311930677/LingXi-DesktopAgent
> 用途：换电脑继续开发。新机器 clone 后按「环境搭建」操作即可接续。

## 一、当前进度总览

### 已完成（本次会话，commit 5de3f9e + 本次新提交）

**小工具 5 轮优化全部完成：**

1. **第1轮 UI 基础修复**：工具页单层滚动容器、计算器结果区限高可滚动、
   天气"今天/明天/周X"标签 + °C + 更新时间戳
2. **第2轮 翻译配置打通**：`translation_config` 优先读设置页 API Key/端点/模型，
   环境变量 `LINGXI_OPENAI_*` 兜底，DeepSeek 默认
3. **第3轮 剪贴板真实监听**：后端 1.5s 轮询线程（接入 setup 启动）、
   `chrono_like_now` 改 Win32 `GetLocalTime`（原 PowerShell 1s/次）、
   去重方向修复、`widget_clipboard_remove` 持久化删除
4. **第4轮 UI 统一**：widget.js 统一剪贴板助手（WebView2 里
   `navigator.clipboard` 会静默失败）、天气页双 display bug 修复
5. **第5轮 工程标准 + 回归**：`docs/engineering-standards.md` 建立，
   全量构建通过，`LINGXI_OPEN_ALL_WIDGETS=1` 冒烟 6 个小工具全部正常

**追加修复（本提交）：**

- **取色器交互式取色**：原实现在点按钮瞬间读光标处像素（永远是按钮自己的颜色）。
  改为：点击后隐藏窗口 → 鼠标移到目标 → 左键确认 / Esc 取消 / 60s 超时。
  涉及 `widget_pick_color`（main.rs）+ colorpicker.html 文案。
- **改写模式 chip 恒显 bug**：`.modes { display: grid }` 压过 `[hidden]`，
  导致「润色/纠错/提示词增强」在所有页面显示。补
  `.modes[hidden] { display: none !important; }` 并把 chip 行挪到
  功能切换行下方（视觉归属改写页）。

### 待办（下次继续的方向）

- [ ] **热键冲突**：Ctrl+Alt+O / T / C 三组热键在本机被其他程序占用
  （RegisterHotKey 返回 0x80070581，非灵犀 bug）。可考虑做成设置页可配置。
- [ ] **取色器体验增强**：可加放大镜预览（当前只能盲选位置）
- [ ] **插件市场按钮**（工具页 tools-market-btn）目前 disabled，未实现
- [ ] **桌宠对话**（pet.js）与 Agent 会话打通尚在初期
- [ ] **NSIS 安装包**：ime-server + overlay 合并打包流程待建立
- [ ] Rime/小狼毫集成（见 docs/weasel-integration.md，未动工）

## 二、环境搭建（新电脑必读）

1. **路径硬约束**：仓库必须放在**纯英文路径**（如 `D:\dev\LingXi-DesktopAgent`），
   GNU 工具链链接器不支持中文路径。
2. **工具链**（两套并存）：
   - workspace（crates/* + ime-server）：**GNU** toolchain，见根 `rust-toolchain.toml`
   - overlay：**MSVC** toolchain + WebView2（独立 crate，不在 workspace），
     见 `apps/overlay/rust-toolchain.toml`，需 VS Build Tools
3. **Node.js**：仅用于前端语法校验脚本，非必需
4. **克隆后验证**：
   ```powershell
   cd apps\overlay
   cargo check        # 应无错误
   cargo run          # 主面板 + 桌宠启动
   # 冒烟测试全部小工具：
   $env:LINGXI_OPEN_ALL_WIDGETS=1; cargo run
   ```

## 三、关键文件索引

| 文件 | 职责 |
| --- | --- |
| `apps/overlay/src/main.rs` | 主入口：所有 Tauri 命令、热键、托盘、剪贴板监听（约 2300 行） |
| `apps/overlay/src/widgets.rs` | 小工具 manifest 目录 + 窗口生命周期（`destroy()` 关闭） |
| `apps/overlay/ui/` | 主面板（index/app.js/styles.css）+ widgets/ 六个小工具 |
| `apps/overlay/ui/widgets/widget.js` | 小工具共享：关闭逻辑 + 剪贴板助手（`writeClipboard/readClipboard`） |
| `apps/overlay/capabilities/default.json` | **窗口权限注册**——新小工具窗口 label 必须加进来，否则白屏 |
| `crates/tools-windows/` | 截屏/取色/输入模拟等 Windows 工具 |
| `docs/engineering-standards.md` | 工程标准（窗口管理六条铁律、UI 规范、验证清单）**必读** |

## 四、已知坑（踩过的，别再踩）

1. 小工具窗口**必须在 capabilities/default.json 注册**，否则安全隔离→白屏
2. 关窗口用 `destroy()` 不用 `close()`（后者 JS 忙时卡死 = "关不掉"）
3. 不要在 `open_widget` 里开 DevTools（Windows 上会最小化宿主窗口）
4. 主线程创建 WebView2 窗口会死锁——一律从子线程/托盘子线程创建
5. WebView2 白屏排查顺序：杀孤儿进程 → 重建缓存目录 → 重装
6. CSS 里 author 级 `display` 会压过 `[hidden]`（已修 .modes 和 .status，
   写新视图时注意同样陷阱）
7. PowerShell 子进程开销约 1s/次——热路径（轮询循环）禁止用
8. `.lock().unwrap()` 一律换 `.safe_lock()`

## 五、运行时状态

- 当前本机有运行中的 overlay 实例（cargo run 后台），换机前无需处理
- API Key 等配置存放在本机 DPAPI/内存，**不随仓库走**——
  新电脑首次使用需在「模型设置」重新填写（或设 LINGXI_OPENAI_* 环境变量）
