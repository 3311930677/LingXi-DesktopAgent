// LingXi IME floating candidate bar — no input box.
// Polls backend ime_state every 30ms to display pinyin + candidates.
// User selects via number keys (handled by the global hook); clicks also work.

const { invoke } = window.__TAURI__
  ? window.__TAURI__.core
  : { invoke: mockInvoke };

const pinyinEl = document.getElementById('pinyin-display');
const candidatesEl = document.getElementById('candidates');
const backendStatusEl = document.getElementById('backend-status');
const backendLabelEl = document.getElementById('backend-label');

let lastPinyin = '';
let lastBackend = '';

async function poll() {
  try {
    const state = await invoke('ime_state');
    if (!state.active && lastPinyin === '' && candidatesEl.children.length === 0) {
      requestAnimationFrame(schedulePoll);
      return;
    }
    if (state.backend !== lastBackend) {
      lastBackend = state.backend;
      const large = state.backend === 'large';
      backendStatusEl.className = `status ${large ? 'large' : 'basic'}`;
      backendLabelEl.textContent = large ? '大词库' : '基础词库';
    }
    if (state.pinyin !== lastPinyin || candidatesEl.children.length !== state.candidates.length) {
      lastPinyin = state.pinyin;
      pinyinEl.textContent = state.pinyin;
      render(state.candidates);
    }
  } catch (e) {
    // ignore
  }
  requestAnimationFrame(schedulePoll);
}

function schedulePoll() {
  setTimeout(poll, 30);
}

function render(candidates) {
  candidatesEl.innerHTML = '';
  // Keep the compact bar readable; number keys 1-6 match visible entries.
  candidates.slice(0, 6).forEach((c, i) => {
    const el = document.createElement('div');
    el.className = 'candidate';
    el.innerHTML = `<span class="idx">${i + 1}</span><span class="word">${esc(c.text)}</span>`;
    el.addEventListener('click', () => commitAt(i));
    candidatesEl.appendChild(el);
  });
}

async function commitAt(index) {
  try {
    await invoke('ime_commit', { index });
  } catch (e) {
    console.error('ime_commit error:', e);
  }
}

function esc(s) {
  const d = document.createElement('span');
  d.textContent = s;
  return d.innerHTML;
}

// Start polling
schedulePoll();

// Mock for browser preview
function mockInvoke(cmd) {
  if (cmd === 'ime_state') {
    return Promise.resolve({ active: true, backend: 'large', pinyin: 'nihao', candidates: [
      { text: '你好', score: 19.9 }, { text: '你', score: 9.5 }
    ]});
  }
  return Promise.resolve();
}
