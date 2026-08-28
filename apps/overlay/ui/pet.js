"use strict";

const TAURI = window.__TAURI__ || null;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;
const listen = TAURI && TAURI.event ? TAURI.event.listen : null;
const pet = document.getElementById("pet");
const avatar = document.getElementById("pet-avatar");
const avatarImg = document.getElementById("pet-avatar-img");
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
  anims: null,
  frame: null,
  bubbles: { idle: "灵犀", thinking: "思考中…", speaking: "建议好了", alert: "QQ 新消息" },
};

let config = FALLBACK;
let currentSkinId = "lingxi-hamster";
let status = "idle";
let down = null;
let lastPointer = null;
let dragDistance = 0;
let pettedThisDrag = false;
let clickTimer = 0;

// ---- 帧动画驱动（petdex spritesheet：8 列，行数按素材规格动态探测）----
// 一个状态用 sheet 的某一行循环播放；rAF 按帧时长推进 background-position。
let animRun = null; // { sheet, row, frames, durationMs, dispW, dispH, rows }
let animSheetLoaded = "";
let animRafId = 0;
let animEpoch = 0; // 状态/皮肤切换时作废进行中的循环

// petdex 素材有两种规格：v1 8×9（1536×1872）、v2 8×11（1536×2288），
// 帧尺寸都是 192×208。行数必须按图片实际高度算，写死 9 会把 v2 压扁
// 并在窗口底部漏出下一行的内容。加载完成前按 9 兜底，探测完成后
// 若正在播这张表则即时纠正 background-size。
const sheetRowsCache = {};
function sheetRows(url) {
  const hit = sheetRowsCache[url];
  if (hit) return hit;
  sheetRowsCache[url] = 9;
  const probe = new Image();
  probe.onload = () => {
    const frame = config.frame;
    const fw = probe.naturalWidth / 8;
    const fh = frame && frame.width ? (fw * frame.height) / frame.width : fw * (208 / 192);
    const rows = Math.max(1, Math.round(probe.naturalHeight / fh));
    sheetRowsCache[url] = rows;
    if (animRun && animRun.sheet === url) animRun.rows = rows;
  };
  probe.src = url;
  return 9;
}

function stopAnim() {
  animRun = null;
  cancelAnimationFrame(animRafId);
  animRafId = 0;
}

function startAnim(a) {
  const frame = config.frame;
  if (!frame || !frame.width || !frame.height) return false;
  const dispW = 200; // 与 .avatar 宽一致
  const dispH = Math.round((dispW * frame.height) / frame.width);
  avatar.style.height = dispH + "px";
  if (animSheetLoaded !== a.sheet) {
    avatar.style.backgroundImage = `url("${a.sheet}")`;
    animSheetLoaded = a.sheet;
  }
  animRun = {
    sheet: a.sheet,
    row: a.row || 0,
    frames: Math.max(1, a.frames || 1),
    durationMs: Math.max(80, a.durationMs || 900),
    dispW,
    dispH,
    rows: sheetRows(a.sheet),
  };
  // 全新起点：避免换行时从上一循环的时间戳继续跳帧。
  animEpoch = performance.now();
  if (!animRafId) animRafId = requestAnimationFrame(animTick);
  return true;
}

function animTick(now) {
  animRafId = 0;
  if (animRun) {
    const per = animRun.durationMs / animRun.frames;
    const elapsed = now - animEpoch;
    const i = Math.floor(elapsed / per) % animRun.frames;
    avatar.style.backgroundSize = `${animRun.dispW * 8}px ${animRun.dispH * animRun.rows}px`;
    avatar.style.backgroundPosition = `${-i * animRun.dispW}px ${-animRun.row * animRun.dispH}px`;
    animRafId = requestAnimationFrame(animTick);
  }
}

// ---- 皮肤缩略图 ----
// 静态图缩略：`<img>`；spritesheet 缩略（`<sheet>#<row>`）：div 背景
// 显示该行第一帧（8 列 × 9 行网格，Y 百分比 = row/8 × 100%）。
function makeSkinThumb(thumbnail) {
  const hash = thumbnail ? thumbnail.indexOf("#") : -1;
  if (hash === -1) {
    const img = document.createElement("img");
    img.src = thumbnail;
    img.alt = "";
    img.draggable = false;
    return img;
  }
  const url = thumbnail.slice(0, hash);
  const row = parseInt(thumbnail.slice(hash + 1), 10) || 0;
  const div = document.createElement("div");
  div.className = "thumb-anim";
  div.style.backgroundImage = `url("${url}")`;
  div.style.backgroundPosition = `0% ${((row / 8) * 100).toFixed(2)}%`;
  return div;
}

function render(next) {
  status = next || "idle";
  // 只切状态类，保留 dragging/dropped/antic 等临时动画类。
  for (const s of ["idle", "thinking", "speaking", "alert"]) {
    pet.classList.toggle(s, s === status);
  }
  // 帧动画皮肤优先于静态图皮肤。
  const anim = (config.anims && config.anims[status]) || null;
  if (anim && startAnim(anim)) {
    avatarImg.style.display = "none";
    avatar.classList.add("is-anim");
  } else {
    stopAnim();
    avatar.classList.remove("is-anim");
    avatar.style.backgroundImage = "";
    avatar.style.width = "";
    avatar.style.height = "";
    animSheetLoaded = "";
    avatarImg.style.display = "";
    const nextAvatar = (config.images && config.images[status]) || FALLBACK.images.idle;
    if (!avatarImg.src.endsWith(nextAvatar)) avatarImg.src = nextAvatar;
  }
  // 互动反应气泡优先：临时台词没说完前不被状态轮询覆盖。
  if (!sayActive) bubble.textContent = config.bubbles[status] || FALLBACK.bubbles.idle;
}

function applyConfig(view) {
  if (!view || !view.skin || (!view.images && !view.anims)) return;
  config = {
    images: view.images || {},
    anims: view.anims || null,
    frame: view.skin.frame || null,
    bubbles: view.bubbles,
  };
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
    item.append(makeSkinThumb(skin.thumbnail));
    const label = document.createElement("span");
    label.textContent = skin.name;
    item.append(label);
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
// 拖动用 Pointer Events + setPointerCapture：捕获后即使指针移出桌宠
// 元素（窗口移动滞后于鼠标时的常见情况），move/up 仍持续送达，
// 不会中途断流。这是“抚摸加了之后拖不动”的根治方案。

pet.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  event.preventDefault();
  try {
    pet.setPointerCapture(event.pointerId);
  } catch {
    /* capture 失败不影响拖动本身 */
  }
  down = { x: event.screenX, y: event.screenY };
  lastPointer = { x: event.screenX, y: event.screenY };
  dragDistance = 0;
  pettedThisDrag = false;
  pet.classList.add("dragging");
  pet.classList.remove("dropped");
});

pet.addEventListener("pointermove", (event) => {
  if (!down || event.buttons !== 1) return;
  const dx = event.screenX - lastPointer.x;
  const dy = event.screenY - lastPointer.y;
  lastPointer = { x: event.screenX, y: event.screenY };
  const step = Math.hypot(dx, dy);
  dragDistance += step;
  // 5px 死区：单击的微小抖动不应推动窗口。
  const total = Math.hypot(event.screenX - down.x, event.screenY - down.y);
  if (total > 5 && (dx || dy) && invoke) {
    // screenX/screenY 是 CSS 像素，而 move_pet_by 按物理像素移动窗口。
    // Windows 显示缩放 125%/150% 时两者差 devicePixelRatio 倍，不修正
    // 就会出现"桌宠追不上鼠标、像被粘住"的拖不动现象。
    const dpr = window.devicePixelRatio || 1;
    invoke("move_pet_by", {
      dx: Math.round(dx * dpr),
      dy: Math.round(dy * dpr),
    }).catch(() => {});
  }
  // 拖得够远就开心一下（摸摸头与拖动共存）。
  if (!pettedThisDrag && dragDistance > 420 && Date.now() > petCooldown) {
    pettedThisDrag = true;
    petCooldown = Date.now() + 3200;
    petted();
  }
});

function endDrag(event, movedOverride) {
  if (!down) return;
  const moved =
    movedOverride === true ||
    Math.hypot(event.screenX - down.x, event.screenY - down.y) > 5;
  try {
    pet.releasePointerCapture(event.pointerId);
  } catch {
    /* already released */
  }
  down = null;
  lastPointer = null;
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
}

pet.addEventListener("pointerup", (event) => endDrag(event));
pet.addEventListener("pointercancel", (event) => endDrag(event, true));

// 兜底：指针彻底丢失（设备拔出等）时复位拖拽态。
window.addEventListener("pointerup", () => {
  if (down) {
    down = null;
    lastPointer = null;
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

// ---- 互动：摸摸头 / 双击逗玩 ----

const PET_LINES = {
  "petdex-nailong": ["duang～再摸摸", "肚肚不许戳！", "龙龙很满意", "嘿嘿嘿"],
  "petdex-coco": ["呼噜呼噜…", "下巴这边再挠挠", "尾巴不许拽！", "喵呜～好舒服"],
  "petdex-mika": ["摸头会长不高的！", "哎呀～", "再摸要生气了哦", "诶嘿嘿"],
  "lingxi-hamster": ["吱！别摸啦", "再摸要打滚了", "嘿嘿…"],
  "lingxi-cat": ["喵～下巴这边", "呼噜噜…", "尾巴不许碰！"],
};
const PET_FALLBACK_LINES = ["嘿嘿，好痒～", "再摸摸我嘛", "(*´▽`*)"];
const POKE_LINES = ["哇！蹦起来了", "转圈圈～", "被你逮到啦", "嘿咻！"];

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
