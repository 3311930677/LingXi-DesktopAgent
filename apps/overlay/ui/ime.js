// LingXi IME candidate panel frontend.
// - Listens to pinyin input, queries backend for candidates in real-time.
// - Number keys (1-9) or click to select a candidate and commit (write-back).
// - Esc hides the panel. Space selects the first candidate if any.

const { invoke } = window.__TAURI__
  ? window.__TAURI__.core
  : { invoke: mockInvoke };

const input = document.getElementById('pinyin-input');
const candidatesEl = document.getElementById('candidates');

let currentCandidates = [];
let committedContext = '';

// Debounce input to avoid overwhelming the backend (though it's fast).
let debounceTimer = null;
input.addEventListener('input', () => {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(fetchCandidates, 30);
});

async function fetchCandidates() {
  const pinyin = input.value.trim();
  if (!pinyin) {
    currentCandidates = [];
    render([]);
    return;
  }
  try {
    const candidates = await invoke('ime_candidates', {
      pinyin,
      context: committedContext,
      limit: 9,
    });
    currentCandidates = candidates;
    render(candidates);
  } catch (e) {
    console.error('ime_candidates error:', e);
    currentCandidates = [];
    render([]);
  }
}

function render(candidates) {
  candidatesEl.innerHTML = '';
  candidates.forEach((c, i) => {
    const el = document.createElement('div');
    el.className = 'candidate';
    el.innerHTML = `<span class="idx">${i + 1}</span><span class="word">${esc(c.text)}</span>`;
    el.addEventListener('click', () => commitCandidate(i));
    candidatesEl.appendChild(el);
  });
}

async function commitCandidate(index) {
  const c = currentCandidates[index];
  if (!c) return;
  try {
    await invoke('ime_commit', { text: c.text });
    committedContext += c.text;
    input.value = '';
    currentCandidates = [];
    render([]);
  } catch (e) {
    console.error('ime_commit error:', e);
  }
}

// Keyboard shortcuts
input.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    e.preventDefault();
    hidePanel();
    return;
  }
  // Number keys select candidates
  if (e.key >= '1' && e.key <= '9' && currentCandidates.length > 0) {
    e.preventDefault();
    const idx = parseInt(e.key) - 1;
    if (idx < currentCandidates.length) {
      commitCandidate(idx);
    }
    return;
  }
  // Space selects first candidate
  if (e.key === ' ' && currentCandidates.length > 0 && input.value.trim()) {
    e.preventDefault();
    commitCandidate(0);
    return;
  }
  // Enter also commits first candidate
  if (e.key === 'Enter' && currentCandidates.length > 0) {
    e.preventDefault();
    commitCandidate(0);
    return;
  }
});

async function hidePanel() {
  input.value = '';
  currentCandidates = [];
  committedContext = '';
  render([]);
  if (window.__TAURI__) {
    const { getCurrentWindow } = window.__TAURI__.window;
    await getCurrentWindow().hide();
  }
}

// When the window gains visibility/focus, auto-focus the input.
document.addEventListener('visibilitychange', () => {
  if (!document.hidden) {
    committedContext = '';
    input.value = '';
    render([]);
    setTimeout(() => input.focus(), 50);
  }
});
window.addEventListener('focus', () => setTimeout(() => input.focus(), 50));

function esc(s) {
  const d = document.createElement('span');
  d.textContent = s;
  return d.innerHTML;
}

// Mock for browser preview (no Tauri runtime).
function mockInvoke(cmd, args) {
  if (cmd === 'ime_candidates') {
    const mock = [
      { text: '你好', syllables: 'ni hao', score: 19.9 },
      { text: '你', syllables: 'ni', score: 9.5 },
    ];
    return Promise.resolve(args.pinyin ? mock : []);
  }
  if (cmd === 'ime_commit') {
    console.log('[mock] commit:', args.text);
    return Promise.resolve();
  }
  return Promise.resolve();
}
