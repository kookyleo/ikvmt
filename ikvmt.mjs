#!/usr/bin/env node
// ikvmt — run commands and read the host console through a Supermicro BMC's
// HTML5 iKVM, fully headless. No SSH, no serial cable: just the BMC's IPMI
// network path plus the browser-based KVM viewer.
//
// Usage:
//   node ikvmt.mjs <bmc-ip> shot                 # screenshot current console + OCR
//   node ikvmt.mjs <bmc-ip> run "<shell cmd>"    # run cmd in host root shell, print output (OCR)
//   node ikvmt.mjs <bmc-ip> repl                 # interactive: one command per stdin line
//
// Requirements:
//   - a Chromium-family browser (system Chrome by default; override with IKVM_CHROME)
//   - python3 + PIL + tesseract on PATH (for screen OCR)
//   - playwright-core (npm install)
//
// How it works: drives the BMC's HTML5 KVM in a headless browser, types the
// command through the Insyde keymap (with explicit shift handling), and reads
// the result by reconnecting for a fresh video frame and OCR'ing the canvas.
// See README.md for details and the reasoning behind each design decision.
//   node ikvmt.mjs <bmc-ip> shot --json          # {state, text} as JSON
//
// Env (all optional):
//   IKVM_USER=<bmc user>  IKVM_PASS=<bmc password>   BMC credentials (required)
//   HOST_USER=<user>       HOST_PASS=<password>      host OS login (auto-used at login prompt)
//
// Built-in discipline (learned the hard way, see SKILL.md):
//   - connect via 2nd tab + referer; poll bootstrap button; wait rfb_state=normal
//   - typing via Insyde Keymap + explicit shift (keyboard.type garbles specials)
//   - Enter belt-and-braces (keyboard.press + sendKey 65293)
//   - two-phase read: phase A types (video may be frozen), close, wait, phase B reconnects
//     and screenshots fresh within 30s; terminal content persists on VGA console
//   - OCR: autocontrast -> threshold 60 -> 2x LANCZOS -> tesseract --psm 6

import { chromium } from 'playwright-core';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const CHROME = process.env.IKVM_CHROME || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const IKVM_USER = process.env.IKVM_USER;
const IKVM_PASS = process.env.IKVM_PASS;
const HOST_USER = process.env.HOST_USER;
const HOST_PASS = process.env.HOST_PASS;
const log = (...a) => console.error(new Date().toTimeString().slice(0, 8), ...a);

// ---------------------------------------------------------------- OCR
function ocr(png) {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'ikvmt-'));
  const bin = path.join(work, 'bin.png');
  const py = `
from PIL import Image, ImageOps
im = Image.open(${JSON.stringify(png)}).convert('L')
im = ImageOps.autocontrast(im)
im = im.point(lambda p: 255 if p > 60 else 0)
im = im.resize((im.width*2, im.height*2), Image.LANCZOS)
im.save(${JSON.stringify(bin)})`;
  spawnSync('python3', ['-c', py], { stdio: 'inherit' });
  const r = spawnSync('tesseract', [bin, 'stdout', '--psm', '6'], { encoding: 'utf8' });
  return (r.stdout || '').trim();
}

// ---------------------------------------------------------------- keyboard
const KEYMAP = [[65,4],[97,4],[66,5],[98,5],[67,6],[99,6],[68,7],[100,7],[69,8],[101,8],[70,9],[102,9],[71,10],[103,10],[72,11],[104,11],[73,12],[105,12],[74,13],[106,13],[75,14],[107,14],[76,15],[108,15],[77,16],[109,16],[78,17],[110,17],[79,18],[111,18],[80,19],[112,19],[81,20],[113,20],[82,21],[114,21],[83,22],[115,22],[84,23],[116,23],[85,24],[117,24],[86,25],[118,25],[87,26],[119,26],[88,27],[120,27],[89,28],[121,28],[90,29],[122,29],[33,30],[49,30],[64,31],[50,31],[35,32],[51,32],[36,33],[52,33],[37,34],[53,34],[94,35],[54,35],[38,36],[55,36],[42,37],[56,37],[40,38],[57,38],[41,39],[48,39],[65293,40],[65307,41],[65288,42],[65289,43],[32,44],[95,45],[45,45],[43,46],[61,46],[91,47],[123,47],[93,48],[125,48],[124,49],[92,49],[59,51],[58,51],[39,52],[34,52],[126,53],[96,53],[44,54],[60,54],[46,55],[62,55],[47,56],[63,56]];
const SHIFT_KS = 65505;

async function typeChar(ikvm, ch) {
  const code = ch.codePointAt(0);
  const entry = KEYMAP.find((e) => e[0] === code);
  if (!entry) { log('WARN unknown char', JSON.stringify(ch)); return; }
  const needsShift = ch !== ch.toLowerCase() || '!@#$%^&*()_+{}|:"<>?~'.includes(ch);
  if (needsShift) await ikvm.evaluate((k) => UI.rfb.sendKey(k, 1), SHIFT_KS);
  await ikvm.evaluate(([k, d]) => UI.rfb.sendKey(k, d), [entry[0], 1]);
  await ikvm.waitForTimeout(35);
  await ikvm.evaluate(([k, d]) => UI.rfb.sendKey(k, d), [entry[0], 0]);
  if (needsShift) await ikvm.evaluate((k) => UI.rfb.sendKey(k, 0), SHIFT_KS);
  await ikvm.waitForTimeout(25);
}
async function typeText(ikvm, t) { for (const ch of t) await typeChar(ikvm, ch); }
async function pressEnter(ikvm) {
  await ikvm.keyboard.press('Enter');
  await ikvm.waitForTimeout(300);
  await ikvm.evaluate(() => UI.rfb.sendKey(65293, 1));
  await ikvm.waitForTimeout(150);
  await ikvm.evaluate(() => UI.rfb.sendKey(65293, 0));
}
async function typeCmd(ikvm, cmd) { await typeText(ikvm, cmd); await pressEnter(ikvm); }

// ---------------------------------------------------------------- connect
async function connect(ip) {
  const browser = await chromium.launch({
    headless: true, executablePath: CHROME, args: ['--ignore-certificate-errors'],
  });
  const ctx = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  await page.goto(`https://${ip}/`, { waitUntil: 'domcontentloaded', timeout: 30000 });
  await page.fill('input[name=name]', IKVM_USER);
  await page.fill('input[name=pwd]', IKVM_PASS);
  await page.click('input[name=Login]');
  await page.waitForTimeout(3000);
  log('bmc logged in');
  const p2 = await ctx.newPage();
  await p2.goto(
    `https://${ip}/cgi/url_redirect.cgi?url_name=man_ikvm_html5`,
    { waitUntil: 'domcontentloaded', timeout: 30000, referer: `https://${ip}/cgi/url_redirect.cgi?url_name=mainmenu` },
  );
  let enabled = false;
  for (let i = 0; i < 15; i++) {
    enabled = await p2.evaluate(() => {
      const b = document.getElementById('btnikvmhtml5_bootstrap');
      return !!b && !b.disabled;
    }).catch(() => false);
    if (enabled) break;
    await p2.waitForTimeout(2000);
  }
  if (!enabled) { await browser.close(); throw new Error('KVM button never enabled (BMC busy? KVM slot occupied by another session?)'); }
  const before = new Set(ctx.pages());
  await p2.evaluate(() => document.getElementById('btnikvmhtml5_bootstrap').click());
  let ikvm = null;
  for (let i = 0; i < 20; i++) {
    await p2.waitForTimeout(1000);
    for (const p of ctx.pages()) if (!before.has(p) && p.url().includes('bootstrap')) { ikvm = p; break; }
    if (ikvm) break;
  }
  if (!ikvm) { await browser.close(); throw new Error('KVM popup did not appear'); }
  await ikvm.waitForLoadState('domcontentloaded');
  for (let i = 0; i < 30; i++) {
    const st = await ikvm.evaluate(() => (window.UI && UI.rfb && UI.rfb._rfb_state) || 'no-rfb').catch(() => 'gone');
    if (st === 'normal') break;
    if (st === 'failed') { await browser.close(); throw new Error('RFB failed'); }
    await ikvm.waitForTimeout(2000);
  }
  return { browser, ikvm };
}

async function screenshot(ikvm, file) {
  const c = await ikvm.$('#noVNC_canvas');
  if (!c) throw new Error('no canvas');
  await c.screenshot({ path: file });
  return file;
}

// ---------------------------------------------------------------- ops
function detectState(text) {
  if (/login:\s*$/im.test(text) || /login:.*$/m.test(text.split('\n').filter(Boolean).pop() || '')) return 'login';
  return 'shell';
}

// Phase A: make sure we're in a root shell, then run `cmd` (video may be frozen — keys still land)
async function phaseA(ip, cmd) {
  const { browser, ikvm } = await connect(ip);
  await ikvm.waitForTimeout(2500);
  await ikvm.bringToFront();
  const cv = await ikvm.$('#noVNC_canvas');
  await cv.click({ force: true });
  await ikvm.waitForTimeout(400);
  const shot = await screenshot(ikvm, path.join(os.tmpdir(), `ikvmt_state_${Date.now()}.png`));
  const text = ocr(shot);
  const state = detectState(text);
  if (state === 'login') {
    if (HOST_USER && HOST_PASS) {
      log('at login prompt -> logging in as', HOST_USER);
      await typeCmd(ikvm, HOST_USER);
      await ikvm.waitForTimeout(2000);
      await typeCmd(ikvm, HOST_PASS);
      await ikvm.waitForTimeout(3000);
    } else {
      log('WARN at login prompt but HOST_USER/HOST_PASS not set; skipping login');
    }
  } else {
    log('root shell present');
  }
  await ikvm.keyboard.press('Control+c');
  await ikvm.waitForTimeout(300);
  log('typing:', cmd);
  await typeCmd(ikvm, `clear;${cmd}`);
  await ikvm.waitForTimeout(3000);
  await browser.close();
  log('phase A done');
}

// Phase B: fresh connection -> fresh video -> screenshot within 30s -> OCR
async function phaseB(ip) {
  const { browser, ikvm } = await connect(ip);
  await ikvm.waitForTimeout(6000);
  const shot = await screenshot(ikvm, path.join(os.tmpdir(), `ikvmt_fresh_${Date.now()}.png`));
  const text = ocr(shot);
  await browser.close();
  return { text, shot };
}

async function cmdShot(ip, json) {
  const { text, shot } = await phaseB(ip);
  if (json) console.log(JSON.stringify({ state: detectState(text), shot, text }, null, 2));
  else console.log(text);
  process.exit(0);
}

async function cmdRun(ip, cmd, json) {
  await phaseA(ip, cmd);
  log('waiting 30s for KVM slot to free...');
  await new Promise((r) => setTimeout(r, 30000));
  const { text, shot } = await phaseB(ip);
  if (json) console.log(JSON.stringify({ cmd, shot, text }, null, 2));
  else console.log(text);
  process.exit(0);
}

async function cmdRepl(ip, json) {
  log('REPL: type a shell command per line (empty line = screenshot only). Ctrl-D to quit.');
  const lines = fs.readFileSync(0, 'utf8').split('\n');
  for (const raw of lines) {
    const line = raw.trim();
    if (!line) continue;
    log('---', line);
    if (line === 'shot' || line === 'screen') {
      const { text } = await phaseB(ip);
      console.log(text);
    } else {
      await phaseA(ip, line);
      await new Promise((r) => setTimeout(r, 30000));
      const { text } = await phaseB(ip);
      console.log(text);
    }
    await new Promise((r) => setTimeout(r, 3000));
  }
  process.exit(0);
}

// ---------------------------------------------------------------- main
const [bmcIp, verb, ...rest] = process.argv.slice(2);
const json = rest.includes('--json');
const args = rest.filter((a) => a !== '--json');
if (!bmcIp || !verb || !/^192\.|^\d+\.\d+/.test(bmcIp)) {
  console.error('usage: ikvmt.mjs <bmc-ip> shot | run "<cmd>" | repl [--json]');
  process.exit(2);
}
const main = async () => {
  if (!IKVM_USER || !IKVM_PASS) {
    console.error('set IKVM_USER and IKVM_PASS (BMC credentials) in the environment');
    process.exit(2);
  }
  if (verb === 'shot') await cmdShot(bmcIp, json);
  else if (verb === 'run') {
    if (!args[0]) { console.error('run: missing command'); process.exit(2); }
    await cmdRun(bmcIp, args[0], json);
  } else if (verb === 'repl') await cmdRepl(bmcIp, json);
  else { console.error('unknown verb:', verb); process.exit(2); }
};
main().catch((e) => { console.error('FATAL:', e.message); process.exit(1); });
