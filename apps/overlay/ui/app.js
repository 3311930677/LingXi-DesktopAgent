"use strict";

// Tauri 2 exposes invoke on window.__TAURI__.core when withGlobalTauri is on.
const TAURI = window.__TAURI__ || null;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;
document.documentElement.classList.toggle("tauri-runtime", Boolean(invoke));

// Sample text used only for the in-browser preview (no Tauri backend).
const MOCK_SOURCE = "圆圆的月亮真好看";

const state = {
  mode: "polish",
  source: MOCK_SOURCE,
  transformed: "",
  diff: [],
  warning: null,
};

const el = {
  card: document.getElementById("card"),
  titlebar: document.getElementById("titlebar"),
  modes: document.getElementById("modes"),
  diff: document.getElementById("diff"),
  statAdd: document.getElementById("stat-add"),
  statDel: document.getElementById("stat-del"),
  status: document.getElementById("status"),
  applyBtn: document.getElementById("apply-btn"),
  undoBtn: document.getElementById("undo-btn"),
  closeBtn: document.getElementById("close-btn"),
  pinBtn: document.getElementById("pin-btn"),
  rewriteTab: document.getElementById("rewrite-tab"),
  qqTab: document.getElementById("qq-tab"),
  settingsBtn: document.getElementById("settings-btn"),
  backendBadge: document.getElementById("backend-badge"),
  rewriteView: document.getElementById("rewrite-view"),
  rewriteActions: document.getElementById("rewrite-actions"),
  qqView: document.getElementById("qq-view"),
  qqConversation: document.getElementById("qq-conversation"),
  qqMessage: document.getElementById("qq-message"),
  qqDraft: document.getElementById("qq-draft"),
  qqRefresh: document.getElementById("qq-refresh"),
  qqGenerate: document.getElementById("qq-generate"),
  qqWrite: document.getElementById("qq-write"),
  settingsPanel: document.getElementById("settings-panel"),
  providerPreset: document.getElementById("provider-preset"),
  backendSelect: document.getElementById("backend-select"),
  endpointInput: document.getElementById("endpoint-input"),
  modelInput: document.getElementById("model-input"),
  apiKeyInput: document.getElementById("api-key-input"),
  keyNote: document.getElementById("key-note"),
  saveSettings: document.getElementById("save-settings"),
  resizeGrip: document.getElementById("resize-grip"),
  taskProgress: document.getElementById("task-progress"),
  progressStage: document.getElementById("progress-stage"),
  progressTime: document.getElementById("progress-time"),
  progressBar: document.getElementById("progress-bar"),
  progressDetail: document.getElementById("progress-detail"),
};

let qqMessage = "";
let currentBackend = "local";

// Browser-only examples for visual preview. Real transformations always come
// from the selected local/cloud model through Tauri.
function transform(mode, text) {
  if (mode === "polish") return "一轮圆月皎洁明亮，格外动人。";
  if (mode === "proofread") return text.replace(/[.。]*$/, "。");
  if (mode === "prompt-enhance") return "# 目标\n请围绕以下内容完成任务：\n" + text;
  return text;
}

// ---- Character-level LCS diff (mirror assistant-core::diff) ----

function diffChars(oldStr, newStr) {
  const a = Array.from(oldStr);
  const b = Array.from(newStr);
  const n = a.length;
  const m = b.length;
  const dp = Array.from({ length: n + 1 }, () => new Int32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  const ops = [];
  const push = (kind, ch) => {
    const last = ops[ops.length - 1];
    if (last && last.kind === kind) last.text += ch;
    else ops.push({ kind, text: ch });
  };
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) push("equal", a[i++]), j++;
    else if (dp[i + 1][j] >= dp[i][j + 1]) push("del", a[i++]);
    else push("ins", b[j++]);
  }
  while (i < n) push("del", a[i++]);
  while (j < m) push("ins", b[j++]);
  return ops;
}

// ---- Rendering ----

function renderDiff(ops) {
  el.diff.replaceChildren();
  let inserted = 0;
  let deleted = 0;
  for (const op of ops) {
    if (op.kind === "equal") {
      el.diff.appendChild(document.createTextNode(op.text));
    } else {
      const span = document.createElement("span");
      span.className = op.kind === "ins" ? "ins" : "del";
      span.textContent = op.text;
      el.diff.appendChild(span);
      if (op.kind === "ins") inserted += Array.from(op.text).length;
      else deleted += Array.from(op.text).length;
    }
  }
  if (state.warning) {
    const note = document.createElement("div");
    note.className = "quality-note";
    const text = document.createElement("span");
    text.textContent = state.warning;
    const retry = document.createElement("button");
    retry.type = "button";
    retry.textContent = "换一个版本";
    retry.addEventListener("click", refreshPreview);
    note.append(text, retry);
    el.diff.prepend(note);
  }
  el.statAdd.textContent = "+" + inserted;
  el.statDel.textContent = "-" + deleted;
}

function renderPreviewState(kind, title, detail = "", source = "", retry = false) {
  el.diff.replaceChildren();
  const box = document.createElement("div");
  box.className = "preview-state " + kind;
  const heading = document.createElement("strong");
  heading.textContent = title;
  box.appendChild(heading);
  if (detail) {
    const text = document.createElement("span");
    text.textContent = detail;
    box.appendChild(text);
  }
  if (source) {
    const original = document.createElement("p");
    original.className = "preview-source";
    original.textContent = "原文：" + source;
    box.appendChild(original);
  }
  if (retry) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "preview-retry";
    button.textContent = "重新生成";
    button.addEventListener("click", refreshPreview);
    box.appendChild(button);
  }
  el.diff.appendChild(box);
  el.statAdd.textContent = "+0";
  el.statDel.textContent = "-0";
}

let loadingDismissed = false;

function dismissStatus() {
  clearTimeout(showStatus._t);
  if (el.status.querySelector(".spinner")) loadingDismissed = true;
  el.status.hidden = true;
}

function statusContents(message, spinner = false) {
  el.status.replaceChildren();
  if (spinner) {
    const spin = document.createElement("span");
    spin.className = "spinner";
    el.status.appendChild(spin);
  }
  const text = document.createElement("span");
  text.className = "status-message";
  text.textContent = message;
  const close = document.createElement("button");
  close.type = "button";
  close.className = "status-close";
  close.setAttribute("aria-label", "关闭提示");
  close.textContent = "×";
  close.addEventListener("click", dismissStatus, { once: true });
  el.status.append(text, close);
}

function showStatus(message, kind) {
  clearTimeout(showStatus._t);
  el.status.className = "status " + kind;
  statusContents(message);
  el.status.hidden = false;
  showStatus._t = setTimeout(dismissStatus, kind === "err" ? 5000 : 2600);
}

// Persistent loading state: the banner can be dismissed without cancelling the
// task, while the apply lock remains until the backend reports readiness.
function showLoading(message, lockApply = true) {
  clearTimeout(showStatus._t);
  if (!loadingDismissed) {
    el.status.className = "status warn";
    statusContents(message, true);
    el.status.hidden = false;
  }
  if (lockApply) setApplyLocked(true);
}

function hideLoading(unlockApply = true) {
  clearTimeout(showStatus._t);
  el.status.hidden = true;
  loadingDismissed = false;
  if (unlockApply) setApplyLocked(false);
}

/// Enable/disable the apply action (button + Ctrl+Enter) as one switch.
let applyLocked = false;
function setApplyLocked(locked) {
  applyLocked = locked;
  el.applyBtn.disabled = locked;
}

// ---- Model task progress ----

let progressTimer = null;
let progressPoll = null;
let progressStartedAt = 0;
let progressKey = "";
let progressMode = "polish";
let progressSourceChars = 0;

function progressAction() {
  return {
    polish: "润色",
    proofread: "纠错",
    "prompt-enhance": "提示词增强",
  }[progressMode] || "改写";
}

function setProgress(stage, detail, percent = null) {
  el.progressStage.textContent = stage;
  el.progressDetail.textContent = detail;
  if (percent == null) {
    el.progressBar.classList.add("indeterminate");
    el.progressBar.style.width = "28%";
  } else {
    el.progressBar.classList.remove("indeterminate");
    el.progressBar.style.transform = "none";
    el.progressBar.style.width = Math.max(2, Math.min(100, percent)) + "%";
  }
}

async function updateModelProgress() {
  const elapsed = (performance.now() - progressStartedAt) / 1000;
  el.progressTime.textContent = elapsed.toFixed(1) + "s";

  if (invoke && currentBackend === "local") {
    try {
      const progress = await invoke("model_progress");
      if (progress.phase === "download") {
        const percent = progress.total > 0 ? progress.current / progress.total * 100 : null;
        const current = (progress.current / 1024 / 1024).toFixed(0);
        const total = progress.total > 0 ? (progress.total / 1024 / 1024).toFixed(0) : "?";
        setProgress("首次使用：正在下载本地模型", `已下载 ${current} / ${total} MB，下载完成后会自动继续`, percent);
        return;
      }
      if (progress.phase === "load") {
        setProgress("正在载入 Qwen2.5 1.5B", "正在解析约 1.1GB 权重，通常需要数秒", null);
        return;
      }
      if (progress.phase === "inference") {
        const percent = progress.total > 0 ? progress.current / progress.total * 100 : null;
        const detail = progressSourceChars >= 80
          ? `已生成 ${progress.current} 个 token；正在保留全部信息并丰富表达，本地最长等待 120 秒`
          : `已生成 ${progress.current} 个 token；正在检查内容是否比原文更丰富且原意不变`;
        setProgress(`正在生成${progressAction()}结果`, detail, percent);
        return;
      }
      if (progress.phase === "error") {
        setProgress("本地模型加载失败", "请检查网络或在模型设置中切换云端后端", null);
        return;
      }
    } catch {
      /* Keep the staged fallback below if progress IPC is temporarily busy. */
    }
  }

  if (elapsed < 1.2) setProgress("正在读取原文", "识别句式、对象和表达意图", null);
  else if (elapsed < 3.5) setProgress("正在判断语言场景", "匹配聊天、描写、工作、技术或正式语体", null);
  else if (elapsed < 7) setProgress("正在保留原意并丰富表达", "补充表达层次、逻辑衔接和细节，同时检查事实不被改变", null);
  else if (currentBackend === "cloud") setProgress("云端模型正在丰富润色", "等待模型返回更完整、更充实的表达并执行质量检查", null);
  else setProgress("本地模型正在丰富润色", progressSourceChars >= 80 ? "长文本最长等待 120 秒；追求更高质量和速度建议切换云端" : "正在生成比原文更丰富的表达，请稍候", null);
}

function startTaskProgress(mode, source) {
  const key = mode + "\u0000" + source;
  if (!el.taskProgress.hidden && progressKey === key) return;
  stopTaskProgress(false);
  renderPreviewState("working", "正在生成预览…", `已读取 ${Array.from(source).length} 个字符，处理完成后将在这里显示结果`, source);
  progressKey = key;
  progressMode = mode;
  progressSourceChars = Array.from(source.trim()).length;
  progressStartedAt = performance.now();
  el.taskProgress.hidden = false;
  el.diff.setAttribute("aria-busy", "true");
  setApplyLocked(true);
  setProgress("正在准备丰富润色", "灵犀会先识别原文场景，再在保留全部原意的前提下扩展表达", null);
  updateModelProgress();
  progressTimer = setInterval(() => {
    const elapsed = (performance.now() - progressStartedAt) / 1000;
    el.progressTime.textContent = elapsed.toFixed(1) + "s";
  }, 100);
  progressPoll = setInterval(updateModelProgress, 450);
}

function stopTaskProgress(unlockApply = true) {
  clearInterval(progressTimer);
  clearInterval(progressPoll);
  progressTimer = null;
  progressPoll = null;
  progressKey = "";
  el.taskProgress.hidden = true;
  el.diff.removeAttribute("aria-busy");
  if (unlockApply) setApplyLocked(false);
}

// ---- Data flow: real backend via Tauri, or local mock in the browser ----

async function refreshPreview() {
  const version = (refreshPreview._version || 0) + 1;
  refreshPreview._version = version;
  if (invoke) {
    const mode = state.mode;
    const source = state.source;
    state.warning = null;
    startTaskProgress(mode, source);
    try {
      const res = await invoke("preview_transform", { mode, text: source });
      // A slower previous mode must never overwrite the newer selection/mode.
      if (version !== refreshPreview._version) return;
      state.transformed = res.transformed;
      state.diff = res.diff.map((d) => ({ kind: d.kind, text: d.text }));
      state.warning = res.warning || null;
      if (res.pending) {
        renderPreviewState("working", "本地模型仍在准备", "首次使用需下载并载入约 1.1GB 模型，进度会显示在上方", source);
        clearTimeout(refreshPreview._retry);
        refreshPreview._retry = setTimeout(refreshPreview, 1200);
      } else {
        clearTimeout(refreshPreview._retry);
        stopTaskProgress();
      }
    } catch (e) {
      if (version !== refreshPreview._version) return;
      clearTimeout(refreshPreview._retry);
      stopTaskProgress();
      const message = String(e);
      const timedOut = message.includes("exceeded") || message.includes("seconds");
      const rejected = message.includes("rejected") || message.includes("truncated");
      const title = timedOut ? "本地模型处理超时" : rejected ? "结果未通过质量检查" : "预览生成失败";
      const detail = timedOut
        ? "不是字数超过限制，而是本地 CPU 在时限内未生成完整结果。可缩短段落或在“模型设置”切换云端。"
        : rejected
          ? "模型返回了截断、句式功能改变或异常扩写的内容，因此已拦截，不会写回原文；请重新生成或切换云端。"
          : message.replace(/^.*?:\s*/, "") || "请重试或切换模型后端。";
      renderPreviewState("error", title, detail, source, true);
      showStatus(title, "err");
      return;
    }
  } else {
    state.transformed = transform(state.mode, state.source);
    state.diff = diffChars(state.source, state.transformed);
  }
  renderDiff(state.diff);
}

// Poll the backend for a freshly captured selection (replaces event listening
// so we do not depend on the Emitter API).
let lastSelectionRevision = null;
async function pollSelection() {
  if (!invoke) return;
  try {
    const selection = await invoke("current_selection");
    if (selection.revision !== lastSelectionRevision) {
      lastSelectionRevision = selection.revision;
      state.source = selection.text;
      if (state.source.trim()) {
        await refreshPreview();
      } else {
        renderPreviewState("empty", "还没有读取到选中文本", "请在目标应用中选中文字，然后按 Ctrl+Alt+Space。");
      }
    }
  } catch {
    renderPreviewState("error", "无法读取当前选区", "请关闭浮窗后重新选择文字并按快捷键。");
  }
}

// ---- Backend settings + QQ semi-automatic assistant ----

function showView(view) {
  dismissStatus();
  const rewrite = view === "rewrite";
  el.rewriteView.hidden = !rewrite;
  el.rewriteActions.hidden = !rewrite;
  el.qqView.hidden = view !== "qq";
  el.settingsPanel.hidden = view !== "settings";
  el.modes.hidden = !rewrite;
  for (const tab of [el.rewriteTab, el.qqTab, el.settingsBtn]) tab.classList.remove("is-active");
  ({ rewrite: el.rewriteTab, qq: el.qqTab, settings: el.settingsBtn })[view].classList.add("is-active");
  // The rewrite panel must stay non-activating so write-back's focus-drift
  // check passes; settings/QQ need real keyboard focus for their text fields.
  if (invoke) {
    invoke("set_panel_focusable", { focusable: !rewrite }).catch(() => {});
  }
}

async function loadSettings() {
  if (!invoke) return;
  try {
    const settings = await invoke("get_backend_settings");
    currentBackend = settings.backend;
    el.providerPreset.value = "";
    el.backendSelect.value = settings.backend;
    el.endpointInput.value = settings.endpoint;
    el.modelInput.value = settings.model;
    el.backendBadge.textContent = settings.backend === "cloud" ? "云端" : "本地";
    el.keyNote.textContent = settings.api_key_configured
      ? "API Key 已配置（仅本次运行内存中；留空可保持不变）。"
      : "密钥不会写入配置文件；也可用 LINGXI_OPENAI_API_KEY 环境变量。";
  } catch (e) {
    showStatus("读取设置失败: " + e, "err");
  }
}

// One-click cloud provider presets. Selecting one fills the endpoint/model and
// switches the backend to cloud; the user only needs to paste their API key.
// All targets speak the OpenAI-compatible chat/completions protocol.
const PROVIDER_PRESETS = {
  deepseek: { endpoint: "https://api.deepseek.com", model: "deepseek-chat" },
  dashscope: {
    endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-plus",
  },
  openai: { endpoint: "https://api.openai.com", model: "gpt-4o-mini" },
};

function applyProviderPreset() {
  const preset = PROVIDER_PRESETS[el.providerPreset.value];
  if (!preset) return;
  el.backendSelect.value = "cloud";
  el.endpointInput.value = preset.endpoint;
  el.modelInput.value = preset.model;
  el.apiKeyInput.focus();
  showStatus("已填入预设，粘贴 API Key 后点保存即可", "ok");
}

async function saveSettings() {
  if (!invoke) return;
  try {
    const settings = await invoke("save_backend_settings", { input: {
      backend: el.backendSelect.value,
      endpoint: el.endpointInput.value,
      model: el.modelInput.value,
      api_key: el.apiKeyInput.value,
    }});
    el.apiKeyInput.value = "";
    currentBackend = settings.backend;
    el.backendBadge.textContent = settings.backend === "cloud" ? "云端" : "本地";
    showView("rewrite");
    showStatus("模型设置已保存", "ok");
    refreshPreview();
  } catch (e) {
    showStatus("保存失败: " + e, "err");
  }
}

async function readQqMessage() {
  if (!invoke) return;
  try {
    const result = await invoke("qq_poll_latest");
    if (!result) {
      showStatus("请先把 QQ 聊天窗口切到前台", "warn");
      return;
    }
    qqMessage = result.message;
    el.qqConversation.textContent = result.conversation || "QQ 会话";
    el.qqMessage.textContent = result.message;
  } catch (e) {
    showStatus("读取 QQ 失败: " + e, "err");
  }
}

async function generateQqDraft() {
  if (!invoke || !qqMessage) {
    showStatus("请先读取一条 QQ 消息", "warn");
    return;
  }
  el.qqGenerate.disabled = true;
  showLoading("正在生成回复草稿…", false);
  try {
    el.qqDraft.value = await invoke("generate_qq_draft", { message: qqMessage });
    hideLoading(false);
  } catch (e) {
    hideLoading(false);
    showStatus("草稿生成失败: " + e, "err");
  } finally {
    el.qqGenerate.disabled = false;
  }
}

async function writeQqDraft() {
  const draft = el.qqDraft.value.trim();
  if (!invoke || !draft) {
    showStatus("请先生成或填写草稿", "warn");
    return;
  }
  try {
    const result = await invoke("write_qq_draft", { draft });
    showStatus(result.verified ? "草稿已写入 QQ，请确认后手动发送" : "已尝试写入，请在 QQ 中确认", result.verified ? "ok" : "warn");
  } catch (e) {
    showStatus("写入 QQ 失败: " + e, "err");
  }
}

// ---- Actions ----

async function apply() {
  // Blocked while the model is still loading: applying now would write back a
  // not-yet-ready (no-op) result.
  if (applyLocked) {
    showStatus("模型仍在加载，请稍候…", "warn");
    return;
  }
  if (invoke) {
    try {
      await invoke("apply_transform", { mode: state.mode });
      showStatus("已应用改写 ✓", "ok");
      // Clear the workspace quickly so another selection can be made without
      // the always-on-top panel covering the editor.
      setTimeout(close, 650);
    } catch (e) {
      showStatus("应用失败: " + e, "err");
    }
  } else {
    showStatus("已应用改写 ✓ (预览模式)", "ok");
  }
}

async function undo() {
  if (invoke) {
    try {
      await invoke("undo_last");
      showStatus("已撤销 ↩", "ok");
      setTimeout(close, 650);
    } catch (e) {
      showStatus("撤销失败: " + e, "err");
    }
  } else {
    showStatus("已撤销 ↩ (预览模式)", "ok");
  }
}

function close() {
  if (invoke) {
    invoke("hide_overlay").catch(() => {});
  } else {
    el.card.style.display = "none";
  }
}

// ---- Wiring ----

el.modes.addEventListener("click", (e) => {
  const chip = e.target.closest(".chip");
  if (!chip || chip.disabled) return;
  for (const c of el.modes.querySelectorAll(".chip")) c.classList.remove("is-active");
  chip.classList.add("is-active");
  state.mode = chip.dataset.mode;
  refreshPreview();
});

el.applyBtn.addEventListener("click", apply);
el.undoBtn.addEventListener("click", undo);
el.closeBtn.addEventListener("click", close);
el.pinBtn.addEventListener("click", () => el.pinBtn.classList.toggle("is-on"));
const quitBtn = document.getElementById("quit-btn");
if (quitBtn) {
  quitBtn.addEventListener("click", () => {
    if (confirm("确定退出灵犀？退出后桌宠和快捷键都会关闭。")) {
      if (invoke) invoke("quit_app").catch(() => {});
      else window.close();
    }
  });
}
el.rewriteTab.addEventListener("click", () => showView("rewrite"));
el.qqTab.addEventListener("click", () => { showView("qq"); readQqMessage(); });
el.settingsBtn.addEventListener("click", () => { showView("settings"); loadSettings(); });
el.providerPreset.addEventListener("change", applyProviderPreset);
el.saveSettings.addEventListener("click", saveSettings);
el.qqRefresh.addEventListener("click", readQqMessage);
el.qqGenerate.addEventListener("click", generateQqDraft);
el.qqWrite.addEventListener("click", writeQqDraft);

// Native drag: begin a Win32 window move on the titlebar. `data-tauri-drag-region`
// is unreliable for a non-activating window, so we drive it explicitly. Ignore
// presses that land on the titlebar's own buttons.
el.titlebar.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || e.target.closest("button")) return;
  if (invoke) {
    e.preventDefault();
    invoke("start_window_drag").catch(() => {});
  }
});

el.resizeGrip.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || !invoke) return;
  e.preventDefault();
  e.stopPropagation();
  invoke("start_window_resize").catch((error) => showStatus("无法调整窗口大小: " + error, "err"));
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") close();
  else if (e.ctrlKey && e.key === "Enter") apply();
});

// Kick off: poll in Tauri, or render the mock once in a browser.
if (invoke) {
  loadSettings();
  pollSelection();
  setInterval(pollSelection, 350);
} else {
  refreshPreview();
}
