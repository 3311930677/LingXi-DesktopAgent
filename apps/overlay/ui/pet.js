"use strict";

const TAURI = window.__TAURI__ || null;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;
const listen = TAURI && TAURI.event ? TAURI.event.listen : null;
const pet = document.getElementById("pet");
const avatar = document.getElementById("pet-avatar");
const bubble = document.getElementById("bubble");
const skinMenu = document.getElementById("skin-menu");

// 浏览器预览回落：Tauri 后端不可用时仍能显示默认皮肤。
const FALLBACK = {
  images: {
    idle: "assets/skins/lingxi-hamster/idle.png",
    thinking: "assets/skins/lingxi-hamster/thinking.png",
    speaking: "assets/skins/lingxi-hamster/speaking.png",
    alert: "assets/skins/lingxi-hamster/alert.png",
  },
  bubbles: { idle: "灵犀", thinking: "思考中…", speaking: "建议好了", alert: "QQ 新消息" },
};

let config = FALLBACK;
let currentSkinId = "lingxi-hamster";
let status = "idle";
let down = null;

function render(next) {
  status = next || "idle";
  pet.className = "pet " + status;
  const nextAvatar = config.images[status] || FALLBACK.images.idle;
  if (!avatar.src.endsWith(nextAvatar)) avatar.src = nextAvatar;
  bubble.textContent = config.bubbles[status] || FALLBACK.bubbles.idle;
}

function applyConfig(view) {
  if (!view || !view.images || !view.skin) return;
  config = { images: view.images, bubbles: view.bubbles };
  currentSkinId = view.skin.id;
  render(status);
}

// ---- 右键皮肤快捷菜单 ----

function closeSkinMenu() {
  skinMenu.hidden = true;
  skinMenu.replaceChildren();
}

function placeSkinMenu(x, y) {
  const rect = skinMenu.getBoundingClientRect();
  const left = Math.max(6, Math.min(x - rect.width / 2, 214 - rect.width));
  const top = Math.max(6, Math.min(y - 10, 254 - rect.height));
  skinMenu.style.left = left + "px";
  skinMenu.style.top = top + "px";
}

async function openSkinMenu(x, y) {
  if (!invoke) return;
  let skins = [];
  try {
    skins = await invoke("list_pet_skins");
  } catch {
    return;
  }
  if (!skins.length) return;
  skinMenu.replaceChildren();
  for (const skin of skins) {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "skin-item" + (skin.id === currentSkinId ? " is-active" : "");
    const thumb = document.createElement("img");
    thumb.src = skin.thumbnail;
    thumb.alt = "";
    thumb.draggable = false;
    const label = document.createElement("span");
    label.textContent = skin.name;
    item.append(thumb, label);
    item.addEventListener("click", async () => {
      closeSkinMenu();
      if (skin.id === currentSkinId) return;
      try {
        // 返回值即时热换；后端同时广播 pet-config-changed 兜底。
        applyConfig(await invoke("set_pet_skin", { skinId: skin.id }));
      } catch {
        bubble.textContent = "切换失败";
      }
    });
    skinMenu.appendChild(item);
  }
  skinMenu.hidden = false;
  placeSkinMenu(x, y);
}

pet.addEventListener("contextmenu", (event) => {
  event.preventDefault();
  if (!skinMenu.hidden) {
    closeSkinMenu();
    return;
  }
  openSkinMenu(event.clientX, event.clientY);
});

document.addEventListener("mousedown", (event) => {
  if (!skinMenu.hidden && !skinMenu.contains(event.target)) closeSkinMenu();
});

window.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !skinMenu.hidden) closeSkinMenu();
});

// ---- 单击 / 拖动 ----

pet.addEventListener("mousedown", (event) => {
  if (event.button !== 0) return;
  down = { x: event.screenX, y: event.screenY };
});

pet.addEventListener("mouseup", async (event) => {
  if (!down || !invoke) return;
  const moved = Math.hypot(event.screenX - down.x, event.screenY - down.y) > 5;
  down = null;
  if (moved) return;
  try {
    await invoke("toggle_panel");
    if (status === "alert") await invoke("set_pet_status", { status: "idle" });
  } catch {
    render("idle");
  }
});

pet.addEventListener("mousemove", (event) => {
  if (!down || !invoke || event.buttons !== 1) return;
  if (Math.hypot(event.screenX - down.x, event.screenY - down.y) > 5) {
    invoke("start_window_drag").catch(() => {});
  }
});

async function poll() {
  if (!invoke) return;
  try {
    render(await invoke("pet_status"));
    // QQ access is opt-in by context: the backend returns null unless a QQ
    // window is foreground. A new message only changes the pet to alert; it
    // never generates or sends a reply by itself.
    await invoke("qq_poll_latest");
  } catch {
    /* transient UIA/provider errors are expected while windows switch */
  }
}

render("idle");
if (invoke) {
  (async () => {
    try {
      applyConfig(await invoke("current_pet_config"));
    } catch {
      /* 后端未就绪时保持默认皮肤 */
    }
  })();
  poll();
  setInterval(poll, 1800);
}
if (listen) {
  listen("pet-config-changed", (event) => applyConfig(event.payload)).catch(() => {});
}
