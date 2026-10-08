// settings.js — 設定画面。Rust 側 (get_config / set_config) とやりとりするだけ。
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const KEYS = [
  ['all', 'デスクトップ全体'],
  ['monitor', 'マウスのモニタ'],
  ['window', '手前のウィンドウ'],
  ['region', '範囲を選ぶ'],
  ['timer_window', 'タイマー: ウィンドウ'],
  ['timer_monitor', 'タイマー: モニタ'],
  ['timer_all', 'タイマー: 全体'],
  ['rec_window', '録画: ウィンドウ'],
  ['rec_monitor', '録画: モニタ'],
  ['rec_region', '録画: 範囲'],
];
const FIELDS = ['save_dir', 'template', 'digits', 'format', 'jpg_quality', 'timer_secs', 'video_template', 'video_fps', 'video_mbps', 'video_limit_secs'];
const CHECKS = ['clipboard', 'sound', 'video_cursor', 'video_audio'];
// Rust 側が数値を期待する項目。select の値は文字列で来るので、数値にして渡す
const NUMERIC = new Set(['digits', 'jpg_quality', 'timer_secs', 'video_fps', 'video_mbps', 'video_limit_secs']);

// ---- ホットキーの入力 ----
// キーの位置 (code) で名前を作る。日本語配列でも英語配列でも同じ。
function keyName(e) {
  const c = e.code;
  if (/^Key[A-Z]$/.test(c)) return c.slice(3);
  if (/^Digit[0-9]$/.test(c)) return c.slice(5);
  if (/^Numpad[0-9]$/.test(c)) return c; // Numpad1
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(c)) return c;
  if (['PrintScreen', 'Pause', 'Insert', 'Home', 'End', 'PageUp', 'PageDown', 'Space', 'Tab', 'Enter',
       'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(c)) return c;
  return null; // 修飾キーだけ、または対応していないキー
}

function buildKeyInputs() {
  const box = $('keys');
  for (const [id, label] of KEYS) {
    const l = document.createElement('div');
    l.textContent = label;
    const inp = document.createElement('input');
    inp.type = 'text';
    inp.readOnly = true;
    inp.className = 'key';
    inp.id = 'hk_' + id;
    inp.placeholder = '(使わない)';
    inp.addEventListener('keydown', (e) => {
      e.preventDefault();
      if ((e.key === 'Delete' || e.key === 'Backspace') && !e.ctrlKey && !e.altKey && !e.shiftKey) { inp.value = ''; return; }
      const k = keyName(e);
      if (!k) return;
      const mods = [];
      if (e.ctrlKey) mods.push('Ctrl');
      if (e.altKey) mods.push('Alt');
      if (e.shiftKey) mods.push('Shift');
      if (e.metaKey) mods.push('Super');
      // 修飾キー無しで使えるのは、F キーと PrintScreen だけ (文字キーを横取りすると、打てなくなる)
      if (!mods.length && !/^F\d+$/.test(k) && k !== 'PrintScreen') {
        $('msg').textContent = '文字キーだけだと、ほかの入力の邪魔になります。Ctrl / Alt / Shift と組み合わせてください。';
        return;
      }
      $('msg').textContent = '';
      inp.value = [...mods, k].join('+');
    });
    box.append(l, inp);
  }
}

// ---- 読み書き ----
function fill(cfg) {
  for (const f of FIELDS) $(f).value = cfg[f];
  for (const c of CHECKS) $(c).checked = !!cfg[c];
  for (const [id] of KEYS) $('hk_' + id).value = cfg.hotkeys[id] || '';
  previewName();
}

function collect() {
  const cfg = { hotkeys: {} };
  for (const f of FIELDS) cfg[f] = NUMERIC.has(f) ? Number($(f).value) : $(f).value;
  for (const c of CHECKS) cfg[c] = $(c).checked;
  for (const [id] of KEYS) cfg.hotkeys[id] = $('hk_' + id).value;
  return cfg;
}

function previewName() {
  const now = new Date(), p = (n, w = 2) => String(n).padStart(w, '0');
  const date = `${now.getFullYear()}${p(now.getMonth() + 1)}${p(now.getDate())}`;
  const time = `${p(now.getHours())}${p(now.getMinutes())}${p(now.getSeconds())}`;
  const digits = Math.max(1, Math.min(9, Number($('digits').value) || 3));
  const name = $('template').value.replaceAll('{n}', p(1, digits)).replaceAll('{date}', date).replaceAll('{time}', time);
  $('template-ex').textContent = name + '.' + ($('format').value === 'jpg' ? 'jpg' : 'png');
}

async function showDir() {
  $('dir-now').textContent = '保存先: ' + (await invoke('resolved_save_dir'));
}

function showKeyErrors(errors) {
  const box = $('key-errors');
  box.hidden = !errors.length;
  box.textContent = errors.join('\n');
}

async function save() {
  try {
    const errors = await invoke('set_config', { cfg: collect() });
    showKeyErrors(errors);
    $('msg').textContent = errors.length ? '保存しました。ただし、登録できなかったホットキーがあります。' : '保存しました。';
    await showDir();
  } catch (e) {
    $('msg').textContent = '保存できませんでした: ' + e;
  }
}

buildKeyInputs();
for (const id of ['template', 'digits', 'format']) $(id).addEventListener('input', previewName);
$('save').addEventListener('click', save);
$('reset').addEventListener('click', async () => { fill(await invoke('default_config')); $('msg').textContent = '初期値に戻しました。保存すると反映されます。'; });
$('open-dir').addEventListener('click', () => invoke('open_save_dir'));

fill(await invoke('get_config'));
await showDir();
showKeyErrors(await invoke('get_hotkey_errors'));
