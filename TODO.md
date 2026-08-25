# TODO

## P0 — 功能正确性
- [ ] 验证 capture_qq_selection_text 实际效果（真机测试）
- [ ] qq_write_draft 坐标硬编码（6/10, 85/100）改为动态探测

## P1 — 架构整洁
- [x] qq.rs 废弃函数清理（capture_qq_selection_text 已替换旧方案）
- [ ] 拆分 qq.rs 为 window/selection/composer 子模块
- [ ] 退役 ime-server/ime-repl/assistant-ime → archived/
- [ ] 重写 README 聚焦当前实际能力

## P2 — 安全
- [x] 设置 CSP (tauri.conf.json 已配置 default-src 'self' 等)
- [x] capabilities 补全 tray-icon 权限（core:tray:default）
- [x] assistant-inference flush 错误处理 (lib.rs:632 已改为 context 传播)

## P3 — CI/CD
- [ ] CI 显式指定 target triple (gnu/msvc)
- [ ] assistant-inference 独立 CI job
- [ ] JS 检查覆盖 app.js/pet.js
- [ ] 添加 cargo build/check 步骤

## P4 — 代码质量
- [ ] 拆分 app.js (26KB) 为模块
- [ ] eprintln → log crate
- [x] 创建 CLAUDE.md
- [ ] 修复 assistant-inference tests::long_rewrite_guard_rejects_truncation 失败（预存问题，validate_output 截断检测阈值）
