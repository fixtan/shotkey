// overlay.js — 範囲選択の重ね画面。モニタごとに 1 枚。
// 固めた画面の画素 (物理ピクセル) を canvas にそのまま置き、その上で範囲を選ぶ。
// 座標は、canvas の物理ピクセルで扱う。窓の見かけの大きさ (CSS ピクセル) との比で換算するので、
// モニタの拡大率が何であっても、選んだ範囲は画素の位置と一致する。
const { invoke } = window.__TAURI__.core;

const m = Number(new URLSearchParams(location.search).get('m') || 0);
const bg = document.getElementById('bg');
const ui = document.getElementById('ui');
const label = document.getElementById('label');
const hint = document.getElementById('hint');

const buf = await invoke('get_snap', { m });
const dv = new DataView(buf);
const W = dv.getUint32(0, true), H = dv.getUint32(4, true);
bg.width = ui.width = W;
bg.height = ui.height = H;
bg.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(buf, 8, W * H * 4), W, H), 0, 0);

const g = ui.getContext('2d');
let start = null, cur = null, done = false;

const toPx = (e) => ({
  x: Math.max(0, Math.min(W, Math.round((e.clientX * W) / window.innerWidth))),
  y: Math.max(0, Math.min(H, Math.round((e.clientY * H) / window.innerHeight))),
});
const rectOf = (a, b) => ({ x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) });

function draw() {
  g.clearRect(0, 0, W, H);
  g.fillStyle = 'rgba(0,0,0,0.45)';
  g.fillRect(0, 0, W, H);
  if (!start || !cur) return;
  const r = rectOf(start, cur);
  g.clearRect(r.x, r.y, r.w, r.h); // 選んだところだけ、暗くしない
  const lw = Math.max(1, Math.round(W / window.innerWidth)); // 線の太さは、見かけで 1px
  g.strokeStyle = '#ff3b30';
  g.lineWidth = lw;
  g.strokeRect(r.x + lw / 2, r.y + lw / 2, Math.max(0, r.w - lw), Math.max(0, r.h - lw));
  label.style.display = 'block';
  label.textContent = `${r.w} × ${r.h}`;
  // ラベルは、選んだ範囲の左上の少し上 (上に余裕が無ければ内側)
  const sx = window.innerWidth / W, sy = window.innerHeight / H;
  label.style.left = `${r.x * sx}px`;
  label.style.top = `${Math.max(2, r.y * sy - 26)}px`;
}

function cancel() {
  if (done) return;
  done = true;
  invoke('cancel_region');
}

window.addEventListener('mousedown', (e) => {
  if (e.button === 2) { cancel(); return; }
  if (e.button !== 0) return;
  hint.style.display = 'none';
  start = cur = toPx(e);
  draw();
});
window.addEventListener('mousemove', (e) => {
  if (!start) return;
  cur = toPx(e);
  draw();
});
window.addEventListener('mouseup', (e) => {
  if (e.button !== 0 || !start || done) return;
  cur = toPx(e);
  const r = rectOf(start, cur);
  if (r.w < 4 || r.h < 4) { start = cur = null; label.style.display = 'none'; draw(); return; } // 小さすぎる: 選び直し
  done = true;
  invoke('finish_region', { m, x: r.x, y: r.y, w: r.w, h: r.h });
});
window.addEventListener('keydown', (e) => { if (e.key === 'Escape') cancel(); });
window.addEventListener('contextmenu', (e) => e.preventDefault());
draw();
