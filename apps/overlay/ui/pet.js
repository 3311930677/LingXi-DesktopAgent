"use strict";

const TAURI = window.__TAURI__ || null;
const invoke = TAURI && TAURI.core ? TAURI.core.invoke : null;
const pet = document.getElementById("pet");
const avatar = document.getElementById("pet-avatar");
const bubble = document.getElementById("bubble");
const avatars = {
  idle: "assets/lingxi-hamster/idle.png",
  thinking: "assets/lingxi-hamster/thinking.png",
  speaking: "assets/lingxi-hamster/speaking.png",
  alert: "assets/lingxi-hamster/alert.png",
};
let status = "idle";
let down = null;

function render(next) {
  status = next || "idle";
  pet.className = "pet " + status;
  const nextAvatar = avatars[status] || avatars.idle;
  if (!avatar.src.endsWith(nextAvatar)) avatar.src = nextAvatar;
  bubble.textContent = {
    idle: "灵犀",
    thinking: "思考中…",
    speaking: "建议好了",
    alert: "QQ 新消息",
  }[status] || "灵犀";
}

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
  poll();
  setInterval(poll, 1800);
}
