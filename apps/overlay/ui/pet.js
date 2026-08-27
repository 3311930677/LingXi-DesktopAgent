"use strict";

const TAURI = window.__TAURI__ || null;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;
const listen = TAURI && TAURI.event ? TAURI.event.listen : null;
const pet = document.getElementById("pet");
const avatar = document.getElementById("pet-avatar");
const bubble = document.getElementById("bubble");
const fxLayer = document.getElementById("fx");
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
let clickTimer = 0;

function render(next) {
  status = next || "idle";
  // 只切状态类，保留 dragging/dropped/antic 等临时动画类。
  for (const s of ["idle", "thinking", "speaking", "alert"]) {
    pet.classList.toggle(s, s === status);
  }
  const nextAvatar = config.images[status] || FALLBACK.images.idle;
  if (!avatar.src.endsWith(nextAvatar)) avatar.src = nextAvatar;
  // 互动反应气泡优先：临时台词没说完前不被状态轮询覆盖。
  if (!sayActive) bubble.textContent = config.bubbles[status] || FALLBACK.bubbles.idle;
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
// 拖动带物理感：抓起时轻微倾斜缩放，松手时 squash & stretch 弹跳。

pet.addEventListener("mousedown", (event) => {
  if (event.button !== 0) return;
  down = { x: event.screenX, y: event.screenY };
  pet.classList.add("dragging");
  pet.classList.remove("dropped");
});

pet.addEventListener("mouseup", async (event) => {
  if (!down) return;
  const moved = Math.hypot(event.screenX - down.x, event.screenY - down.y) > 5;
  down = null;
  pet.classList.remove("dragging");
  if (moved) {
    pet.classList.add("dropped");
    setTimeout(() => pet.classList.remove("dropped"), 620);
    return;
  }
  // 短延迟区分单击/双击：双击先不打开面板，改成逗它一下。
  if (clickTimer) {
    clearTimeout(clickTimer);
    clickTimer = 0;
    pokeReact();
    return;
  }
  clickTimer = setTimeout(async () => {
    clickTimer = 0;
    if (!invoke) return;
    try {
      await invoke("toggle_panel");
      if (status === "alert") await invoke("set_pet_status", { status: "idle" });
    } catch {
      render("idle");
    }
  }, 240);
});

// 鼠标在窗口外松开时也要复位拖拽态。
window.addEventListener("mouseup", () => {
  if (down) {
    down = null;
    pet.classList.remove("dragging");
  }
});

// ---- idle 彩蛋：偶尔左右张望，让形象更“活” ----
let anticTimer = 0;
function scheduleAntic() {
  clearTimeout(anticTimer);
  anticTimer = setTimeout(() => {
    if (status === "idle" && !down) {
      pet.classList.add("antic");
      setTimeout(() => pet.classList.remove("antic"), 1700);
    }
    scheduleAntic();
  }, 9000 + Math.random() * 15000);
}
scheduleAntic();

pet.addEventListener("mousemove", (event) => {
  if (down && invoke && event.buttons === 1) {
    if (Math.hypot(event.screenX - down.x, event.screenY - down.y) > 5) {
      invoke("start_window_drag").catch(() => {});
    }
  }
  // 摸摸头：不按按键在它身上滑来滑去，滑够了就开心一下。
  if (!down && !event.buttons) {
    petDist += Math.hypot(event.movementX || 0, event.movementY || 0);
    if (petDist > 170 && Date.now() > petCooldown) {
      petDist = 0;
      petCooldown = Date.now() + 2600;
      petted();
    }
  }
});

// ---- 互动：摸摸头 / 双击逗玩 ----

const PET_LINES = {
  "lingxi-nailong": ["嘿嘿，再摸摸～", "呼噜噜…", "翅膀不许拉！", "好舒服呀"],
  "lingxi-snow": ["凉凉的，多摸一会儿～", "围巾都摸乱啦", "耳朵会抖哦", "嘿嘿…"],
  "lingxi-hamster": ["吱！别摸啦", "再摸要打滚了", "嘿嘿…"],
  "lingxi-cat": ["喵～下巴这边", "呼噜噜…", "尾巴不许碰！"],
};
const PET_FALLBACK_LINES = ["嘿嘿，好痒～", "再摸摸我嘛", "(*´▽`*)"];
const POKE_LINES = ["哇！蹦起来了", "转圈圈～", "被你逮到啦", "嘿咻！"];

let petDist = 0;
let petCooldown = 0;
let sayTimer = 0;
let sayActive = false;

function pick(list) {
  return list[Math.floor(Math.random() * list.length)];
}

// 临时说一句，几秒后回到当前状态的气泡。
function sayTemp(text, ms) {
  clearTimeout(sayTimer);
  sayActive = true;
  bubble.textContent = text;
  sayTimer = setTimeout(() => {
    sayActive = false;
    bubble.textContent = config.bubbles[status] || FALLBACK.bubbles.idle;
  }, ms || 2200);
}

function spawnFx(glyph, count) {
  for (let i = 0; i < count; i++) {
    const s = document.createElement("span");
    s.className = "fx";
    s.textContent = glyph;
    s.style.left = 40 + Math.floor(Math.random() * 120) + "px";
    s.style.top = 55 + Math.floor(Math.random() * 60) + "px";
    s.style.animationDelay = (Math.random() * 0.3).toFixed(2) + "s";
    s.style.setProperty("--fx-rot", Math.floor(Math.random() * 50 - 25) + "deg");
    fxLayer.appendChild(s);
    setTimeout(() => s.remove(), 1700);
  }
}

function petted() {
  pet.classList.add("happy");
  setTimeout(() => pet.classList.remove("happy"), 1100);
  spawnFx("❤", 4);
  const lines = PET_LINES[currentSkinId] || PET_FALLBACK_LINES;
  sayTemp(pick(lines));
}

function pokeReact() {
  pet.classList.remove("jump");
  void pet.offsetWidth;
  pet.classList.add("jump");
  setTimeout(() => pet.classList.remove("jump"), 950);
  spawnFx("✦", 5);
  sayTemp(pick(POKE_LINES));
}

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
