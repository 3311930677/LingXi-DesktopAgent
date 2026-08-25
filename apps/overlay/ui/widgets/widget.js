// 灵犀小工具共享脚本 — Tauri invoke 初始化 + 统一关闭按钮逻辑
"use strict";
const TAURI = window.__TAURI__;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;

// 统一关闭按钮：调用 close_widget 关闭当前窗口
function setupWidgetClose(widgetId) {
  const btn = document.getElementById("widget-close");
  if (!btn) return;
  btn.addEventListener("click", () => {
    if (invoke) {
      invoke("close_widget", { id: widgetId }).catch(() => {});
    } else {
      window.close();
    }
  });
}

// Esc 关闭窗口
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    const btn = document.getElementById("widget-close");
    if (btn) btn.click();
  }
});

// 显示状态文字（兼容各 widget 的 status 元素）
function setStatus(text, kind) {
  const el = document.getElementById("status");
  if (!el) return;
  el.className = "status" + (kind ? " " + kind : "");
  if (text.includes("...") || text.includes("中")) {
    el.innerHTML = '<span class="spinner"></span> ' + text;
  } else {
    el.textContent = text;
  }
}
