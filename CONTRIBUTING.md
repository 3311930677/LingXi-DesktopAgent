# 参与共创

感谢参与灵犀智能输入项目。本文约定开发环境、启动、自测和协作流程。

## 1. 环境要求

- Windows 10/11 x64
- Git
- Rust stable，包含 GNU 与 MSVC toolchain
- Visual Studio Build Tools（勾选“使用 C++ 的桌面开发”）
- Microsoft Edge WebView2 Runtime
- Node.js 18+

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup toolchain install stable-x86_64-pc-windows-msvc
```

根 workspace 使用 GNU；`apps/overlay` 通过目录内的 `rust-toolchain.toml` 自动使用 MSVC。

## 2. 克隆与一键启动

```powershell
git clone https://github.com/3311930677/Intelligent-Input-Method-v2.git
cd Intelligent-Input-Method-v2
powershell -ExecutionPolicy Bypass -File .\scripts\run-dev.ps1
```

脚本会：

1. 将 rime-ice 克隆到项目同级的 `rime-ice` 目录（若已存在则复用）；
2. 启动加载 `8105.dict.yaml` 与 `base.dict.yaml` 的 `ime-server`；
3. 检查 `nihao`、`suoyi`、`dang` 以及简拼 `nh`、`zgr` 等候选；
4. 在另一个窗口启动 Tauri overlay。

启动后 IME 模式默认开启。在任意文本框直接输入拼音，空格选首选、数字键选择候选；`Esc` 退出，`Ctrl+Alt+I` 再次开启/关闭。

如 rime-ice 不在默认位置：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run-dev.ps1 -RimeDir "D:\path\to\rime-ice"
```

## 3. 独立启动与测试

```powershell
# 终端 1：大词库服务
cargo run -p ime-server -- --dict "D:\path\to\rime-ice\cn_dicts\8105.dict.yaml" --dict "D:\path\to\rime-ice\cn_dicts\base.dict.yaml"

# 终端 2：桌面前端
cd apps\overlay
cargo run

# 终端 3：候选协议自测
cd ..\..
powershell -ExecutionPolicy Bypass -File .\scripts\test-ime.ps1
```

候选窗右上角显示：

- `大词库`（绿色）：已连接 `ime-server` + rime-ice；
- `基础词库`（橙色）：服务未连接，正在使用内置最小词库，候选会明显变少。

## 4. 提交前检查

```powershell
# 根 workspace
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Overlay（MSVC）
cd apps\overlay
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
node --check ui\ime.js
```

## 5. Git 协作流程

不要直接在 `main` 上开发：

```powershell
git checkout main
git pull
git checkout -b feature/<简短功能名>
# 修改并完成上述检查
git add -A
git commit -m "feat: 功能说明"
git push -u origin feature/<简短功能名>
```

随后在 GitHub 创建 Pull Request。PR 请说明：

- 改了什么、为什么；
- 如何验证；
- 输入法行为或 UI 变化请附截图/录屏；
- 已知限制和后续事项。

推荐提交前缀：`feat:`、`fix:`、`refactor:`、`test:`、`docs:`。

## 6. 目录分工

- `crates/assistant-ime`：拼音切分、词典、候选生成与重排；
- `apps/ime-server`：大词库/AI 重排 IPC 服务；
- `apps/overlay`：全局键盘钩子、候选窗和桌面 Agent；
- `crates/assistant-windows`：UIA、剪贴板、键盘写回；
- `crates/assistant-inference`：本地/云端模型；
- `plugins/lingxi-rime-filter`：librime filter 实验代码。

代码注释使用英文。不要提交 `target/`、模型权重、API Key、本地日志或 rime-ice 全量数据。
