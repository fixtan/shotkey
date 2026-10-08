// 画面側 (設定画面・範囲選択の重ね画面) の試験。Tauri を模して、ブラウザで動かす。
//   使い方:  node test/ui.mjs     (playwright が入っていること)
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'src');
const types = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css' };
const server = http.createServer((req, res) => {
  const f = path.join(root, decodeURIComponent(req.url.split('?')[0]));
  if (!f.startsWith(root) || !fs.existsSync(f)) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'content-type': types[path.extname(f)] || 'application/octet-stream' });
  res.end(fs.readFileSync(f));
});
await new Promise((r) => server.listen(0, r));
const base = `http://localhost:${server.address().port}`;

let ok = 0, fail = 0;
const check = (name, cond, extra = '') => { if (cond) ok++; else { fail++; console.log('NG', name, extra); } };

// Tauri の代わり。呼ばれた invoke を window.__calls に貯める。
const mock = (cfg) => `
  window.__calls = [];
  const cfg = ${JSON.stringify(cfg)};
  const snap = () => { const W = 400, H = 300, b = new ArrayBuffer(8 + W * H * 4), dv = new DataView(b);
    dv.setUint32(0, W, true); dv.setUint32(4, H, true);
    const px = new Uint8Array(b, 8); for (let i = 0; i < W * H; i++) { px[i*4] = 200; px[i*4+1] = 50; px[i*4+2] = 50; px[i*4+3] = 255; }
    return b; };
  window.__TAURI__ = { core: { invoke: async (cmd, args) => {
    window.__calls.push({ cmd, args });
    switch (cmd) {
      case 'get_snap': return snap();
      case 'get_config': return cfg;
      case 'default_config': return { ...cfg, template: 'shot_{date}_{n}' };
      case 'resolved_save_dir': return 'C:\\\\Users\\\\x\\\\Pictures\\\\shotkey';
      case 'get_hotkey_errors': return ['範囲 (Ctrl+Alt+4): 登録できない'];
      case 'set_config': return [];
      default: return null;
    } } } };`;

const baseCfg = { save_dir: '', template: 'shot_{date}_{n}', digits: 3, format: 'png', jpg_quality: 90, clipboard: false, sound: true,
  timer_secs: 5, hotkeys: { all: 'Ctrl+Alt+1', monitor: 'Ctrl+Alt+2', window: 'Ctrl+Alt+3', region: 'Ctrl+Alt+4', timer_window: 'Ctrl+Alt+5', timer_monitor: '', timer_all: '' } };

const browser = await chromium.launch();

// ---- 範囲選択の重ね画面 ----
{
  // 画素 400x300 を、見かけ 200x150 の窓に出す (拡大率 2 のモニタのつもり)
  const ctx = await browser.newContext({ viewport: { width: 200, height: 150 } });
  const page = await ctx.newPage();
  await page.addInitScript(mock(baseCfg));
  await page.goto(base + '/overlay.html?m=1');
  await page.waitForFunction(() => window.__calls.some((c) => c.cmd === 'get_snap'));
  const gs = await page.evaluate(() => window.__calls.find((c) => c.cmd === 'get_snap').args);
  check('get_snap にモニタ番号を渡す', gs.m === 1, JSON.stringify(gs));

  const px = await page.evaluate(() => { const d = document.getElementById('bg').getContext('2d').getImageData(5, 5, 1, 1).data; return [...d]; });
  check('固めた画素が canvas に載る', px[0] === 200 && px[1] === 50 && px[2] === 50, px.join(','));
  const size = await page.evaluate(() => [document.getElementById('bg').width, document.getElementById('bg').height]);
  check('canvas は物理ピクセルの大きさ', size[0] === 400 && size[1] === 300, size.join('x'));

  // 小さすぎるドラッグは、選び直し (finish は呼ばない)
  await page.mouse.move(10, 10); await page.mouse.down(); await page.mouse.move(11, 11); await page.mouse.up();
  check('小さいドラッグでは決定しない', !(await page.evaluate(() => window.__calls.some((c) => c.cmd === 'finish_region'))));

  // 見かけ (10,10)-(60,40) → 物理ピクセル (20,20) 幅100 高60
  await page.mouse.move(10, 10); await page.mouse.down(); await page.mouse.move(60, 40, { steps: 5 });
  const lab = await page.textContent('#label');
  check('寸法の表示は物理ピクセル', lab.trim() === '100 × 60', lab);
  await page.mouse.up();
  const fin = await page.evaluate(() => window.__calls.filter((c) => c.cmd === 'finish_region').map((c) => c.args));
  check('範囲を物理ピクセルで渡す', fin.length === 1 && fin[0].m === 1 && fin[0].x === 20 && fin[0].y === 20 && fin[0].w === 100 && fin[0].h === 60, JSON.stringify(fin));
  await ctx.close();
}
{
  // 逆向きのドラッグ (右下から左上) と、窓の外へのはみ出し
  const ctx = await browser.newContext({ viewport: { width: 200, height: 150 } });
  const page = await ctx.newPage();
  await page.addInitScript(mock(baseCfg));
  await page.goto(base + '/overlay.html?m=0');
  await page.waitForFunction(() => window.__calls.some((c) => c.cmd === 'get_snap'));
  await page.mouse.move(150, 100); await page.mouse.down(); await page.mouse.move(100, 60, { steps: 4 }); await page.mouse.up();
  const fin = await page.evaluate(() => window.__calls.find((c) => c.cmd === 'finish_region')?.args);
  check('逆向きのドラッグも正規化される', fin && fin.x === 200 && fin.y === 120 && fin.w === 100 && fin.h === 80, JSON.stringify(fin));
  await ctx.close();
}
{
  // Esc と右クリックで、やめる
  for (const how of ['esc', 'right']) {
    const ctx = await browser.newContext({ viewport: { width: 200, height: 150 } });
    const page = await ctx.newPage();
    await page.addInitScript(mock(baseCfg));
    await page.goto(base + '/overlay.html?m=0');
    await page.waitForFunction(() => window.__calls.some((c) => c.cmd === 'get_snap'));
    if (how === 'esc') await page.keyboard.press('Escape'); else await page.mouse.click(50, 50, { button: 'right' });
    await page.waitForTimeout(50);
    const calls = await page.evaluate(() => window.__calls.map((c) => c.cmd));
    check(`${how} でやめる`, calls.includes('cancel_region') && !calls.includes('finish_region'), calls.join(','));
    await ctx.close();
  }
}

// ---- 設定画面 ----
{
  const ctx = await browser.newContext({ viewport: { width: 620, height: 800 } });
  const page = await ctx.newPage();
  await page.addInitScript(mock(baseCfg));
  await page.goto(base + '/index.html');
  await page.waitForFunction(() => document.getElementById('hk_all')?.value);
  check('ホットキーが入る', (await page.inputValue('#hk_all')) === 'Ctrl+Alt+1' && (await page.inputValue('#hk_timer_monitor')) === '' && (await page.inputValue('#hk_timer_window')) === 'Ctrl+Alt+5');
  check('登録できなかった説明を出す', (await page.textContent('#key-errors')).includes('登録できない'));
  check('保存される場所を出す', (await page.textContent('#dir-now')).includes('Pictures'));
  check('名前の例が出る', /^shot_\d{8}_001\.png$/.test(await page.textContent('#template-ex')), await page.textContent('#template-ex'));

  // ホットキーの入力
  await page.click('#hk_window');
  await page.keyboard.press('Control+Alt+KeyK');
  check('Ctrl+Alt+K を取り込む', (await page.inputValue('#hk_window')) === 'Ctrl+Alt+K', await page.inputValue('#hk_window'));
  await page.keyboard.press('KeyA');
  check('修飾なしの文字キーは断る', (await page.inputValue('#hk_window')) === 'Ctrl+Alt+K' && (await page.textContent('#msg')).includes('Ctrl'));
  await page.keyboard.press('F9');
  check('F キーは単独で使える', (await page.inputValue('#hk_window')) === 'F9');
  await page.keyboard.press('Delete');
  check('Delete で空にする', (await page.inputValue('#hk_window')) === '');

  // 保存で、画面の値がそのまま渡る
  await page.fill('#template', 'cap_{n}');
  await page.selectOption('#format', 'jpg');
  await page.check('#clipboard');
  await page.click('#hk_timer_monitor'); await page.keyboard.press('Control+Shift+F5');
  await page.click('#save');
  await page.waitForFunction(() => window.__calls.some((c) => c.cmd === 'set_config'));
  const sent = await page.evaluate(() => window.__calls.find((c) => c.cmd === 'set_config').args.cfg);
  check('保存に画面の値が渡る', sent.template === 'cap_{n}' && sent.format === 'jpg' && sent.clipboard === true && sent.digits === 3 && sent.timer_secs === 5, JSON.stringify(sent));
  check('ホットキーも渡る', sent.hotkeys.all === 'Ctrl+Alt+1' && sent.hotkeys.window === '' && sent.hotkeys.timer_monitor === 'Ctrl+Shift+F5' && sent.hotkeys.timer_window === 'Ctrl+Alt+5' && sent.hotkeys.timer_all === '', JSON.stringify(sent.hotkeys));
  await ctx.close();
}

await browser.close();
server.close();
console.log(`${ok} OK, ${fail} FAIL`);
process.exit(fail ? 1 : 0);
