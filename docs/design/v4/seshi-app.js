import { Grid, segLen, mix } from './seshi-grid.js';
import { T } from './seshi-tokens.js';

const CL = '\ue0b6', CR = '\ue0b4', COG = '\uf013';
const A = T.a, DESK = T.bg, PB = mix(T.bg, T.surf, 0.7), SKY = T.ws.sky, CONF = mix(T.needs, T.text, 0.45);
const SPIN = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];
const SC = { needs: T.needs, work: T.text, done: T.ok, idle: T.dim, shell: T.dim };
const STN = { needs: 'needs you', work: 'working', done: 'done', idle: 'idle', shell: 'shell' };
const L = s => Array.from(s).length;
const trunc = (s, n) => L(s) <= n ? s : Array.from(s).slice(0, Math.max(0, n - 1)).join('') + '…';
const gl = (st, f) => st === 'work' ? SPIN[(f || 0) % 8] : ({ needs: '●', done: '✓', idle: '○', shell: '›' })[st];
const right = (g, xr, y, segs, b) => g.putSegs(xr - segLen(segs), y, segs, b);
const pill = (segs, bg) => [{ t: CL, fg: bg }, ...segs.map(x => ({ ...x, bg })), { t: CR, fg: bg }];
const rowPill = (g, x, y, w, bg) => g.putSegs(x, y, [{ t: CL, fg: bg }, { t: ' '.repeat(Math.max(0, w - 2)), bg }, { t: CR, fg: bg }]);
const kc = (k, on) => pill([{ t: k, fg: on ? T.accInk : T.acc, b: 1 }], on ? T.acc : T.btn);
const btn = (l, k, kind) => { const [bg, fg, kf] = kind === 'primary' ? [T.acc, T.accInk, T.accInk] : kind === 'danger' ? [T.err, T.bg, T.bg] : [T.btn, T.strong, T.acc]; return pill([{ t: l + ' ', fg, b: 1 }, { t: k, fg: kf, b: 1 }], bg); };
const hints = list => { const o = []; list.forEach(([k, l], i) => o.push({ t: k, fg: T.acc, b: 1 }, { t: ' ' + l + (i < list.length - 1 ? '   ' : ''), fg: T.text })); return o; };
function dimAll(g) { for (const r of g.c) for (const c of r) { const cap = c.ch === CL || c.ch === CR; c.fg = cap ? mix(c.fg, '#000000', 0.45) : mix(mix(c.fg, c.bg, 0.6), '#000000', 0.45); c.bg = mix(c.bg, '#000000', 0.45); } }
function dimRect(g, x, y, w, h, t) { for (let r = y; r < y + h; r++) for (let c = x; c < x + w; c++) { const k = g.c[r] && g.c[r][c]; if (k) k.fg = mix(k.fg, k.bg, t); } }

let SOFT = 0;
const MODAL_KINDS = ['tickets', 'cp'];
function frame(g, x, y, w, h, o = {}) {
  if (SOFT && o.title && !o.ob) { const pb = o.focus ? (o.border || T.acc) : T.btn; o = { ...o, title: null, pt: [{ t: CL, fg: pb }, ...o.title.map(t => ({ ...t, fg: o.focus ? T.accInk : (t.fg === T.line ? T.dim : t.fg), bg: pb })), { t: CR, fg: pb }] }; }
  const f = o.focus, bc = o.border || (f ? T.acc : T.line), bg = o.bg || PB, B = { fg: bc, bg: o.ob || DESK, b: f };
  g.fill(x, y, w, h, bg);
  g.hl(x + 1, y, w - 2, '─', B); g.hl(x + 1, y + h - 1, w - 2, '─', B);
  g.vl(x, y + 1, h - 2, '│', B); g.vl(x + w - 1, y + 1, h - 2, '│', B);
  g.set(x, y, '╭', B); g.set(x + w - 1, y, '╮', B); g.set(x, y + h - 1, '╰', B); g.set(x + w - 1, y + h - 1, '╯', B);
  let rl = 0;
  if (o.right) { const rs = [{ t: ' ' }, ...o.right, { t: ' ' }]; rl = segLen(rs); right(g, x + w - 2, y, rs, { bg: o.ob || DESK }); }
  if (o.title) g.putSegs(x + 2, y, [{ t: ' ' }, ...o.title, { t: ' ' }], { bg: o.ob || DESK }, x + w - 3 - rl);
  if (o.pt) g.putSegs(x + 2, y, o.pt, { bg: o.ob || DESK }, x + w - 3 - rl);
  if (o.foot) g.putSegs(x + 2, y + h - 1, [{ t: ' ' }, ...o.foot, { t: ' ' }], { bg: o.ob || DESK }, x + w - 3);
}
function statusBar(g, x, y, w, segs, rsegs, bg = T.card2) {
  rowPill(g, x + 2, y, w - 4, bg);
  g.putSegs(x + 4, y, segs, { bg }, x + w - 4);
  if (rsegs && segLen(segs) + segLen(rsegs) + 3 <= w - 8) right(g, x + w - 4, y, rsegs, { bg });
}

// ---------- data ----------
const SESS = () => ({
  orders: { sec: 'agents', folder: 'aevox', agent: 'claude', br: 'orders', st: 'needs', age: '38s', q: 'Allow edit to src/checkout.ts?', pr: { n: 412, ok: 0 }, srv: { st: 'ready', port: 3001 }, conf: 'rate-limit', file: 'checkout.ts', chg: '+88 −12 · 4 files', last: 'Coupons now apply after tax; I need to edit checkout.ts.' },
  'rate-limit': { sec: 'agents', folder: 'aevox', agent: 'codex', br: 'rate-limit', st: 'work', age: '4m', srv: { st: 'starting', port: 3003 }, conf: 'orders', file: 'checkout.ts', sub: 11, last: 'Adding a token bucket in front of the checkout endpoint.' },
  search: { sec: 'agents', folder: 'aevox', agent: 'claude', br: 'search', st: 'done', age: '2m', chg: '+42 −7 · 3 files', srv: { st: 'ready', port: 3002 }, last: 'Search now debounces input by 150 ms. All 41 tests pass.' },
  main: { sec: 'agents', folder: 'aevox', agent: 'claude', br: 'main', st: 'idle', age: '1h', srv: { st: 'crashed', port: 3000 }, last: 'Ready when you are.' },
  docs: { sec: 'agents', folder: 'glyph', agent: 'claude', br: 'docs-plan', st: 'done', age: '9m', pr: { n: 194, ok: 1 }, chg: '+118 −4 · 2 files', srv: { st: 'none' }, last: 'Phase 15 is planned and its 9 tickets are filed.' },
  'shell 1': { sec: 'terms', folder: 'aevox', st: 'shell', br: 'main', age: '' },
  'gpu-bench': { sec: 'ssh', folder: 'build-box', agent: 'claude', br: 'gpu-bench', st: 'work', age: '12m', last: 'Profiling the batch loop on the A100.' }
});
const TICKETS = [
  { n: 431, t: 'Checkout total is wrong with coupons', lab: 'bug' },
  { n: 428, t: 'Rate-limit the checkout endpoint', lab: 'infra', in: 'rate-limit' },
  { n: 425, t: 'Search should debounce input', lab: 'ux', in: 'search' },
  { n: 419, t: 'Dark mode for receipts', lab: 'ui', in: 'queue' },
  { n: 417, t: 'Flaky e2e: payment redirect', lab: 'test' },
  { n: 411, t: 'Receipts as PDF', lab: 'feature' }];
const QUEUE = () => [
  { t: '#419 Dark mode for receipts', st: 'running', wt: 'receipts-419', age: '6m' },
  { t: 'Bump stripe to v14', st: 'running', wt: 'stripe-14', age: '2m' },
  { t: '#417 Flaky e2e: payment redirect', st: 'starting', wt: 'e2e-417' },
  { t: 'Write tests for coupon.ts', st: 'waiting' },
  { t: '#411 Receipts as PDF', st: 'waiting' },
  { t: 'Translate checkout copy', st: 'review', chg: '+30 −2 · 2 files' },
  { t: '#405 Upgrade to Node 22', st: 'failed', why: 'npm ci failed: lockfile out of date' }];
const CPS = [['7:42', 'turn 6', 'round coupons half-even', '+12 −3 · 1 file'], ['7:38', 'turn 5', 'move tax into its own module', '+40 −31 · 3 files'], ['7:31', 'turn 4', 'apply coupons after tax', '+22 −9 · 2 files'], ['7:20', 'turn 3', 'add failing test for coupons', '+18 · 1 file'], ['7:12', 'turn 2', 'read checkout flow', 'no changes'], ['7:05', 'start', 'worktree created from main', '']];

export function initialState() {
  return { W: 160, H: 45, f: 0, ov: null, sheet: null, sel: 'orders', focus: 'orders', leader: 0, compose: null, why: null, toast: null,
    S: SESS(), conf: [{ a: 'orders', b: 'rate-limit', file: 'checkout.ts', on: 1 }],
    inbox: { row: 0, q: '', confirm: null }, tk: { tab: 'github', row: 0, q: '', key: '', add: null, gh: 'ok', plane: 'error' }, Q: QUEUE(),
    pr: 'ok', cp: { on: 0, row: 0 }, remote: 0, first: 0, ship: null,
    set: { tab: 'sync', sync: 'off', repo: 'cstin/seshi-config', al: { on: 0, mode: 'name', topic: 'seshi-k7f3q9', test: 'idle' } } };
}
const RANK = { needs: 0, done: 1, work: 2, idle: 3, shell: 4 };
const order = s => Object.keys(s.S).sort((a, b) => RANK[s.S[a].st] - RANK[s.S[b].st]);
const visible = s => { const o = order(s); return ['agents', 'terms', 'ssh'].flatMap(sec => o.filter(id => s.S[id].sec === sec)); };

function layout(s) {
  const { W, H } = s, M = 2, GP = SOFT ? 3 : 2, SW = W < 140 ? 26 : W >= 200 ? 40 : 34, x0 = M + SW + GP, aw = W - x0 - M, py = 4, ph = H - 1 - py;
  let sx = 0, sw = 0, pw = aw;
  if (s.sheet && !(s.modalTools && MODAL_KINDS.includes(s.sheet.k))) { if (W < 140) { sx = x0; sw = aw; pw = 0; } else { sw = W >= 200 ? 96 : 72; sx = x0 + aw - sw; pw = aw - sw - 2; } }
  if (s.sheet && !(s.modalTools && MODAL_KINDS.includes(s.sheet.k)) && W >= 140) pw = aw - sw - GP, sx = x0 + aw - sw;
  return { W, H, M, SW, x0, aw, py, ph, sx, sw, pw, GP, small: W < 140 };
}

// ---------- sidebar ----------
function tagsOf(s, id, small) {
  const x = s.S[id], t = [];
  if (x.pr) t.push({ t: '#' + x.pr.n + (x.pr.ok ? ' ✓' : ' ✕'), fg: x.pr.ok ? T.ok : T.err });
  if (x.srv && x.srv.st !== 'none' && !small) {
    const v = x.srv.st === 'ready' ? { t: '▶ :' + x.srv.port, fg: T.ok } : x.srv.st === 'starting' ? { t: SPIN[s.f % 8] + ' :' + x.srv.port, fg: T.dim } : x.srv.st === 'crashed' ? { t: '✕ :' + x.srv.port + ' crashed', fg: T.err } : { t: '■ :' + x.srv.port, fg: T.dim };
    t.push(v);
  }
  const c = s.conf.find(c => c.on && (c.a === id || c.b === id));
  if (c && !small) t.push({ t: '⇆ ' + c.file, fg: CONF });
  return t;
}
function sidebar(g, s, Ly, reg) {
  const { M, SW, H, small } = Ly, x = M, y = 1, w = SW, h = H - 2;
  frame(g, x, y, w, h, s.remote ? { title: [{ t: '⇄ build-box', fg: T.ws.teal, b: 1 }, { t: ' remote', fg: T.dim }] } : {});
  const rowY = {}; let yy = y + 2;
  const secs = s.remote ? [['agents', 'AGENTS']] : [['agents', 'AGENTS'], ['terms', 'TERMINALS'], ['ssh', 'SSH']];
  const ids = s.first ? [] : order(s);
  for (const [sec, name] of secs) {
    const list = ids.filter(id => (s.remote ? s.S[id].sec === 'ssh' : s.S[id].sec === sec));
    const hd = [{ t: name, fg: T.dim, b: 1 }, { t: ' ' + list.length + ' ', fg: T.text }];
    g.putSegs(x + 3, yy, hd); g.hl(x + 3 + segLen(hd), yy, w - 6 - segLen(hd), '─', { fg: T.line }); yy++;
    if (!list.length) {
      if (sec === 'agents') { ['Run claude or codex in any', 'repo. It moves into its own', 'worktree and shows up here.'].forEach(l => { g.put(x + 3, yy++, trunc(l, w - 6), { fg: T.dim, i: 1 }); }); }
      else g.put(x + 3, yy++, 'none', { fg: T.dim, i: 1 });
      yy++; continue;
    }
    const folders = [...new Set(list.map(id => s.S[id].folder))];
    for (const fo of folders) {
      g.putSegs(x + 3, yy, [{ t: '▾ ', fg: T.dim }, { t: (sec === 'ssh' ? '⇄ ' : '') + fo, fg: sec === 'ssh' ? T.ws.teal : T.strong, b: 1 }]); yy++;
      for (const id of list.filter(i => s.S[i].folder === fo)) {
        if (yy > y + h - 6) break;
        const it = s.S[id], sel = s.sel === id, bg = sel ? T.hov : PB;
        rowY[id] = yy;
        if (sel) rowPill(g, x + 2, yy, w - 4, T.hov);
        g.put(x + 5, yy, gl(it.st, s.f), { fg: SC[it.st], b: it.st === 'needs', bg });
        g.put(x + 7, yy, trunc(id, w - 18), { fg: sel ? T.strong : T.text, b: sel, bg });
        const meta = it.agent ? (small ? it.age : it.agent + ' · ' + it.age) : '';
        if (meta) right(g, x + w - 4, yy, [{ t: meta, fg: it.st === 'needs' ? T.needs : T.dim }], { bg });
        reg(x + 2, yy, w - 4, 1, { t: 'sel', id }, 'row');
        yy++;
        const tg = tagsOf(s, id, small);
        if (tg.length && !small) { let tx = x + 7; tg.forEach(t => { if (tx + L(t.t) <= x + w - 3) tx = g.putSegs(tx, yy, [t]) + 2; else if (t.t[0] === '⇆' && tx + 1 <= x + w - 3) tx = g.putSegs(tx, yy, [{ ...t, t: '⇆' }]) + 2; }); yy++; }
        if (it.q && it.st === 'needs' && !small) { g.put(x + 7, yy, trunc(it.q, w - 10), { fg: mix(T.needs, PB, 0.2), i: 1 }); yy++; }
        if (it.sub && !small) { g.put(x + 7, yy, '↳ workflow-subagent ×' + it.sub, { fg: T.dim }); yy++; }
      }
    }
    yy++;
  }
  if (s.remote) { g.put(x + 3, yy, trunc('Over SSH: no checkpoints and', w - 6), { fg: T.dim, i: 1 }); g.put(x + 3, yy + 1, trunc('no open-in-browser yet.', w - 6), { fg: T.dim, i: 1 }); }
  g.hl(x + 3, y + h - 4, w - 6, '─', { fg: T.line });
  g.putSegs(x + 3, y + h - 3, hints([['a', 'actions']]));
  right(g, x + w - 3, y + h - 3, [{ t: COG, fg: T.text }, { t: ' ,', fg: T.acc, b: 1 }]);
  reg(x + 3, y + h - 3, 9, 1, { t: 'toast', m: 'Actions list: built in 0.15' });
  reg(x + w - 7, y + h - 3, 4, 1, { t: 'open', ov: 'settings' });
  return rowY;
}

// ---------- tabs + panes ----------
function tabs(g, s, Ly, reg) {
  let tx = Ly.x0;
  const tp = (segs, bg, act) => { const x0 = tx; tx = g.putSegs(tx, 2, pill([{ t: ' ' }, ...segs, { t: ' ' }], bg)) + 1; if (act) reg(x0, 2, tx - x0 - 1, 1, act); };
  tp([{ t: '1 agents', fg: T.accInk, b: 1 }, { t: ' ●', fg: T.needs, b: 1 }], T.acc);
  tp([{ t: '2 ', fg: T.text, b: 1 }, { t: 'servers', fg: T.text }, { t: ' ▶', fg: T.ok }], PB, { t: 'toast', m: 'Server output lives in each worktree’s sheet now (l)' });
  tp([{ t: '+', fg: T.acc, b: 1 }], PB);
  if (s.leader) right(g, Ly.x0 + Ly.aw, 2, pill([{ t: ' ⌨ CTRL+SPACE ', fg: T.bg, b: 1 }, ...(Ly.small ? [] : [{ t: ' press a key · ? keys · Esc ', fg: T.bg }])], SKY));
}
function outLines(s, id) {
  const x = s.S[id];
  if (x.st === 'shell') return [[{ t: '@cstin ', fg: A.blue }, { t: '→ ', fg: A.green }, { t: 'aevox ', fg: T.fg }, { t: 'git(', fg: A.yellow }, { t: 'main', fg: A.red }, { t: ')', fg: A.yellow }], [{ t: '$ ', fg: T.dim }, { t: '█', fg: T.acc }]];
  const o = [[{ t: '● ', fg: A.green }, { t: 'Read(src/' + (x.file || 'index.ts') + ')', fg: T.fg }], [{ t: '  └ 214 lines', fg: T.dim }], [], [{ t: '● ', fg: A.green }, { t: 'Bash(npm test -- ' + x.br + ')', fg: T.fg }], [{ t: '  └ ', fg: T.dim }, { t: '41 passed', fg: A.green }, { t: ', 0 failed', fg: T.dim }], [], [{ t: '● ', fg: T.fg }, { t: x.last || '', fg: T.fg }]];
  if (x.sent) o.push([], [{ t: '> ', fg: T.dim }, { t: x.sent, fg: T.strong }]);
  if (x.st === 'needs') o.push([], [{ t: x.q, fg: T.strong, b: 1 }], [{ t: '❯ 1. Yes   2. Yes, and don’t ask again   3. No', fg: T.text }]);
  if (x.st === 'done') o.push([], [{ t: '✻ done · ' + x.chg, fg: T.dim }]);
  if (x.st === 'work') o.push([], [{ t: SPIN[s.f % 8] + ' Working… (esc to interrupt)', fg: A.yellow }]);
  return o;
}
function pane(g, s, x, y, w, h, id, focus, reg) {
  const it = s.S[id], bc = focus && s.leader ? SKY : null;
  frame(g, x, y, w, h, { focus, border: bc, title: [{ t: id, fg: focus ? (s.leader ? SKY : T.acc) : T.text, b: 1 }, ...(it.agent && w > 50 ? [{ t: ' ─ ', fg: T.line }, { t: it.agent, fg: T.dim }] : [])], right: [{ t: gl(it.st, s.f) + ' ' + STN[it.st], fg: SC[it.st], b: it.st === 'needs' }, { t: '  ✕', fg: T.dim }] });
  const lines = outLines(s, id), top = y + 2, bot = y + h - 3, n = bot - top + 1;
  const PX = SOFT ? 4 : 3;
  lines.slice(Math.max(0, lines.length - n)).forEach((l, i) => g.putSegs(x + PX, top + Math.max(0, n - lines.length) + i, l, {}, x + w - PX));
  g.putSegs(x + PX, y + h - 2, [{ t: '~\\code\\' + it.folder, fg: T.dim }, { t: '  ⎇ ' + it.br, fg: A.green }], {}, x + w - 20);
  const tg = tagsOf(s, id, w < 60); if (tg.length) { let segs = []; tg.forEach(t => segs.push(t, { t: '  ' })); segs.pop(); right(g, x + w - 3, y + h - 2, segs); }
  if (!focus) dimRect(g, x + 1, y + 1, w - 2, h - 2, 0.42);
  reg(x, y, w, h, { t: 'focus', id }, 'pane');
}
function panes(g, s, Ly, reg) {
  const { x0, py, ph, pw } = Ly; if (pw <= 0) return;
  if (s.first) {
    frame(g, x0, py, pw, ph, { focus: 1, title: [{ t: 'shell 1', fg: T.acc, b: 1 }] });
    const cx = x0 + Math.floor(pw / 2), cy = py + Math.floor(ph / 2) - 4, c = (yy, segs) => g.putSegs(cx - Math.floor(segLen(segs) / 2), yy, segs);
    c(cy, [{ t: 'Welcome to Seshi', fg: T.strong, b: 1 }]);
    c(cy + 2, [{ t: 'cd', fg: T.acc, b: 1 }, { t: ' into a repo, then run ', fg: T.text }, { t: 'claude', fg: T.acc, b: 1 }, { t: ' or ', fg: T.text }, { t: 'codex', fg: T.acc, b: 1 }, { t: '.', fg: T.text }]);
    c(cy + 3, [{ t: 'It gets its own worktree and appears in the sidebar.', fg: T.dim }]);
    c(cy + 5, [{ t: 'Ctrl+Space', fg: SKY, b: 1 }, { t: ' then ', fg: T.dim }, { t: '?', fg: T.acc, b: 1 }, { t: ' shows every key.', fg: T.dim }]);
    g.putSegs(x0 + 3, py + ph - 3, [{ t: '@cstin ', fg: A.blue }, { t: '→ ', fg: A.green }, { t: '~ ', fg: T.fg }, { t: '█', fg: T.acc }]);
    return;
  }
  const f = s.S[s.focus] ? s.focus : 'orders';
  if (pw >= 110) { const lw = Math.floor(pw * 0.62), gp = Ly.GP; pane(g, s, x0, py, lw, ph, f, 1, reg); pane(g, s, x0 + lw + gp, py, pw - lw - gp, ph, 'shell 1', 0, reg); }
  else pane(g, s, x0, py, pw, ph, f, 1, reg);
}

// ---------- the sheet ----------
const SHEETS = { inbox: 'Inbox', tickets: 'Tickets', pr: 'Pull request', changes: 'Changes', server: 'Dev server', cp: 'Checkpoints' };
function sheet(g, s, Ly, reg, asModal) {
  const sh = s.sheet;
  let x = Ly.sx, y = Ly.py, w = Ly.sw, h = Ly.ph;
  if (asModal) {
    const qn = s.Q.length + s.Q.filter(q => q.st !== 'waiting').length + (s.tk.add != null ? 2 : 0);
    const body = sh.k === 'cp' ? (s.cp.on ? CPS.length + 3 : 7) : s.tk.tab === 'queue' ? qn + 4 : s.tk.tab === 'github' ? TICKETS.length + 4 : 7;
    w = Math.min(sh.k === 'cp' ? 92 : 112, s.W - 6); h = Math.min(body + 6, s.H - 4); x = Math.floor((s.W - w) / 2); y = Math.floor((s.H - h) / 2);
  }
  const tgt = sh.id || s.sel;
  const title = [{ t: SHEETS[sh.k], fg: T.acc, b: 1 }];
  if (['pr', 'changes', 'server', 'cp'].includes(sh.k) && sh.k !== 'changes' || (sh.k === 'changes' && !sh.conf)) title.push({ t: ' ─ ', fg: T.line }, { t: tgt, fg: T.text });
  frame(g, x, y, w, h, { focus: 1, bg: T.card, title, right: [{ t: 'Esc ✕', fg: T.dim }] });
  reg(x + w - 9, y, 7, 1, { t: 'close' });
  const C = { bg: T.card }, X = x + (SOFT ? 4 : 3), W2 = w - (SOFT ? 8 : 6), Y = y + 2;
  ({ inbox: inboxBody, tickets: ticketsBody, pr: prBody, changes: changesBody, server: serverBody, cp: cpBody })[sh.k](g, s, { x, y, w, h, X, W2, Y, C, tgt }, reg);
}
function sectionHd(g, X, yy, W2, name, n, C) { const hd = [{ t: name, fg: T.dim, b: 1 }, ...(n != null ? [{ t: ' ' + n + ' ', fg: T.text }] : [{ t: ' ' }])]; g.putSegs(X, yy, hd, C); g.hl(X + segLen(hd), yy, W2 - segLen(hd), '─', { fg: T.line, ...C }); }
function inboxItems(s) {
  const o = order(s).filter(id => s.S[id].sec !== 'terms');
  const q = s.inbox.q.toLowerCase();
  if (q) return o.filter(id => id.includes(q) || (s.S[id].agent || '').includes(q)).map(id => ({ k: 'find', id }));
  return [...o.filter(id => s.S[id].st === 'needs').map(id => ({ k: 'needs', id })), ...s.conf.filter(c => c.on).map((c, i) => ({ k: 'conf', i })), ...o.filter(id => s.S[id].st === 'done').map(id => ({ k: 'done', id }))];
}
function inboxBody(g, s, b, reg) {
  const { X, W2, Y, C, x, w, y, h } = b; let yy = Y;
  rowPill(g, X - 1, yy, W2 + 2, T.card2);
  g.putSegs(X + 1, yy, [{ t: '› ', fg: T.acc, b: 1 }, ...(s.inbox.q ? [{ t: s.inbox.q, fg: T.strong }, { t: '█', fg: T.acc }] : [{ t: 'type to find any session', fg: T.dim, i: 1 }])], { bg: T.card2 });
  yy += 2;
  const items = inboxItems(s), cur = items[s.inbox.row];
  const isSel = it => cur && it.k === cur.k && it.id === cur.id && it.i === cur.i;
  const head = (it, segs, rsegs) => { const sel = isSel(it), bg = sel ? T.hov : T.card; if (sel) rowPill(g, X - 1, yy, W2 + 2, T.hov); g.putSegs(X + 1, yy, segs, { bg }, X + W2 - (rsegs ? segLen(rsegs) + 1 : 0)); if (rsegs) right(g, X + W2 - 1, yy, rsegs, { bg }); reg(X - 1, yy, W2 + 2, 1, { t: 'irow', n: items.indexOf(it) }); yy++; };
  if (s.inbox.q) {
    sectionHd(g, X, yy++, W2, 'FOUND', items.length, C);
    items.forEach(it => { const z = s.S[it.id]; head(it, [{ t: gl(z.st, s.f) + ' ', fg: SC[z.st] }, { t: it.id, fg: T.strong, b: 1 }, { t: '  ' + z.agent + ' · ' + z.folder, fg: T.dim }], [{ t: 'Enter go', fg: T.dim }]); });
    if (!items.length) g.put(X, yy++, 'No session matches. Esc clears.', { fg: T.dim, i: 1, ...C });
    statusBar(g, x, y + h - 2, w, hints([['↑↓', 'move'], ['Enter', 'go to it'], ['Esc', 'clear']]));
    return;
  }
  const needs = items.filter(i => i.k === 'needs'), conf = items.filter(i => i.k === 'conf'), done = items.filter(i => i.k === 'done');
  sectionHd(g, X, yy++, W2, 'NEEDS YOU', needs.length, C);
  if (!needs.length) g.putSegs(X, yy++, [{ t: '✓ ', fg: T.ok }, { t: 'Nothing is waiting on you.', fg: T.text }], C);
  needs.forEach(it => {
    const z = s.S[it.id];
    head(it, [{ t: '● ', fg: T.needs, b: 1 }, { t: it.id, fg: T.strong, b: 1 }, { t: '  ' + z.agent + ' · ' + z.folder, fg: T.dim }], [{ t: z.age, fg: T.needs }]);
    g.put(X + 3, yy++, trunc(z.q, W2 - 4), { fg: T.needs, i: 1, ...C });
    if (s.compose && s.compose.from === 'inbox' && s.compose.id === it.id) { yy = composeInline(g, s, X + 1, yy, W2 - 1, T.card, 6); }
    else { let ax = X + 3; [['Yes', '1', 'primary'], ['Always', '2'], ['No', '3']].forEach(([l, k, st]) => { const x0 = ax; ax = g.putSegs(ax, yy, btn(l, k, st)) + 1; reg(x0, yy, ax - x0 - 1, 1, { t: 'answer', id: it.id, n: +k }); }); g.putSegs(ax + 2, yy, hints([['m', 'follow-up']]), C); reg(ax + 2, yy, 11, 1, { t: 'compose', id: it.id, from: 'inbox' }); yy++; }
    yy++;
  });
  if (conf.length) {
    sectionHd(g, X, yy++, W2, 'HEADS UP', null, C);
    conf.forEach(it => { const c = s.conf[it.i]; head(it, [{ t: '⇆ ', fg: CONF, b: 1 }, { t: c.a, fg: T.strong, b: 1 }, { t: ' and ', fg: T.text }, { t: c.b, fg: T.strong, b: 1 }, { t: ' both changed ', fg: T.text }, { t: c.file, fg: A.blue }]);
      g.put(X + 3, yy++, trunc('Different branches, same lines 80–96. It will conflict on merge.', W2 - 4), { fg: T.dim, i: 1, ...C });
      g.putSegs(X + 3, yy, hints([['d', 'both diffs'], ['m', 'tell orders'], ['k', 'dismiss']]), C); reg(X + 3, yy, 12, 1, { t: 'confDiff', i: it.i }); yy += 2; });
  }
  sectionHd(g, X, yy++, W2, 'JUST FINISHED', done.length, C);
  if (!done.length) g.put(X, yy++, 'Nothing finished since you last looked.', { fg: T.dim, i: 1, ...C });
  done.forEach(it => {
    const z = s.S[it.id];
    head(it, [{ t: '✓ ', fg: T.ok, b: 1 }, { t: it.id, fg: T.strong, b: 1 }, { t: '  ' + z.agent + ' · ' + z.folder, fg: T.dim }], [{ t: z.chg, fg: T.text }]);
    g.put(X + 3, yy++, trunc('“' + z.last + '”', W2 - 4), { fg: T.text, i: 1, ...C });
    const cf = s.inbox.confirm;
    if (cf && cf.id === it.id) {
      if (cf.busy) g.putSegs(X + 3, yy, [{ t: SPIN[s.f % 8] + ' ', fg: T.acc }, { t: cf.k === 'merge' ? 'Merging ' + it.id + ' into main…' : 'Removing worktree…', fg: T.text }], C);
      else { let ax = g.putSegs(X + 3, yy, [{ t: cf.k === 'merge' ? 'Merge into main, then remove the worktree and branch?  ' : 'Throw away ' + z.chg.split(' · ')[0] + ' and remove the worktree?  ', fg: T.strong }], C); ax = g.putSegs(ax, yy, btn(cf.k === 'merge' ? 'Merge' : 'Throw away', 'Enter', cf.k === 'merge' ? 'primary' : 'danger')) + 1; g.putSegs(ax, yy, btn('Cancel', 'Esc')); }
    } else {
      const ks = [['d', 'diff'], ['M', 'merge'], ['x', 'throw away']]; if (z.srv && z.srv.st === 'ready') ks.push(['O', ':' + z.srv.port]);
      g.putSegs(X + 3, yy, hints(ks), C); reg(X + 3, yy, 6, 1, { t: 'diff', id: it.id }); reg(X + 12, yy, 7, 1, { t: 'merge', id: it.id });
    }
    yy += 2;
  });
  const q = s.Q, cnt = st => q.filter(i => i.st === st).length;
  if (yy < y + h - 4) {
    sectionHd(g, X, yy++, W2, 'QUEUE', null, C);
    g.putSegs(X, yy, [{ t: SPIN[s.f % 8] + ' ' + cnt('running') + ' running', fg: T.text }, { t: ' · ' + cnt('starting') + ' starting · ' + cnt('waiting') + ' waiting', fg: T.dim }, ...(cnt('failed') ? [{ t: ' · ' + cnt('failed') + ' failed', fg: T.err }] : [])], C);
    right(g, X + W2, yy, hints([['Q', 'open']]), C); reg(X, yy, W2, 1, { t: 'sheet', k: 'tickets', tab: 'queue' });
  }
  statusBar(g, x, y + h - 2, w, hints([['↑↓', 'move'], ['1 2 3', 'answer'], ['m', 'follow-up'], ['Enter', 'go']]), [{ t: 'j', fg: T.acc, b: 1 }, { t: ' closes', fg: T.dim }]);
}
function wrapText(t, n) { const out = []; for (const para of t.split('\n')) { if (!para) { out.push(''); continue; } let cur = ''; for (const w of para.split(' ')) { if (!cur) cur = w; else if (L(cur) + 1 + L(w) <= n) cur += ' ' + w; else { out.push(cur); cur = w; } while (L(cur) > n) { out.push(cur.slice(0, n)); cur = cur.slice(n); } } out.push(cur); } return out; }
function composeInline(g, s, x, yy, w, ob = T.card, maxRows = 8) {
  const text = s.compose.text || '', lines = wrapText(text, w - 6), rows = Math.max(3, Math.min(maxRows, lines.length + (text ? 0 : 0)));
  const hidden = Math.max(0, lines.length - rows), shown = lines.slice(hidden), h = rows + 2;
  const words = text.trim() ? text.trim().split(/\s+/).length : 0;
  frame(g, x, yy, w, h, { focus: 1, bg: T.card2, ob, title: hidden ? [{ t: '↑ ' + hidden + ' more', fg: T.dim }] : null, right: [{ t: lines.length > 1 || words ? words + ' words' : '', fg: T.dim }] });
  if (!text) g.put(x + 3, yy + 1, 'Next prompt for ' + s.compose.id + '…', { fg: T.dim, i: 1, bg: T.card2 });
  shown.forEach((l, i) => { const last = i === shown.length - 1; g.putSegs(x + 3, yy + 1 + i, [{ t: l, fg: T.strong }, ...(last ? [{ t: '█', fg: T.acc }] : [])], { bg: T.card2 }, x + w - 2); });
  if (!text) g.put(x + 3 + L('Next prompt for ' + s.compose.id + '…') + 1, yy + 1, '█', { fg: T.acc, bg: T.card2 });
  yy += h;
  g.putSegs(x + 1, yy, hints([['Enter', 'send'], ['Shift+Enter', 'new line'], ['Esc', 'cancel']]), { bg: ob });
  return yy + 1;
}
function ticketsBody(g, s, b, reg) {
  const { X, W2, Y, C, x, w, y, h } = b, tk = s.tk; let yy = Y;
  let tx = X - 1;
  [['github', 'GitHub', TICKETS.length], ['linear', 'Linear', null], ['plane', 'Plane', null], ['queue', 'Queue', s.Q.length]].forEach(([k, l, n]) => {
    const on = tk.tab === k, x0 = tx;
    tx = g.putSegs(tx, yy, on ? pill([{ t: ' ' + l + (n != null ? ' ' + n : '') + ' ', fg: T.accInk, b: 1 }], T.acc) : [{ t: '  ' + l + (n != null ? ' ' + n : '') + '  ', fg: T.text, ...C }]) + 1;
    reg(x0, yy, tx - x0 - 1, 1, { t: 'tkTab', k });
  });
  right(g, X + W2, yy, [{ t: 'Tab', fg: T.acc, b: 1 }, { t: ' next', fg: T.dim }], C);
  yy += 2;
  if (tk.tab === 'queue') return queueBody(g, s, b, yy, reg);
  if (tk.tab === 'linear') {
    g.put(X, yy++, 'Linear isn’t set up yet.', { fg: T.strong, b: 1, ...C });
    g.put(X, yy++, 'Paste a personal API key. It stays on this machine.', { fg: T.dim, ...C }); yy++;
    rowPill(g, X - 1, yy, W2 + 2, T.card2); g.putSegs(X + 1, yy, [{ t: 'key ', fg: T.dim }, ...(tk.key ? [{ t: '•'.repeat(tk.key.length), fg: T.strong }] : [{ t: 'lin_api_…', fg: T.dim, i: 1 }]), { t: '█', fg: T.acc }], { bg: T.card2 }); yy += 2;
    g.putSegs(X, yy, btn('Save key', 'Enter', 'primary'), C); g.putSegs(X + 18, yy, [{ t: 'linear.app › Settings › API', fg: A.blue, u: 1 }], C);
    statusBar(g, x, y + h - 2, w, hints([['Enter', 'save'], ['Tab', 'next source']]));
    return;
  }
  if (tk.tab === 'plane') {
    g.putSegs(X, yy++, [{ t: '✕ ', fg: T.err, b: 1 }, { t: 'Plane rejected the key (401).', fg: T.strong, b: 1 }], C);
    g.put(X, yy++, 'It may have expired. Paste a new one, or retry.', { fg: T.dim, ...C }); yy++;
    let ax = g.putSegs(X, yy, btn('Retry', 'r', 'primary'), C) + 1; g.putSegs(ax, yy, btn('New key', 'k'), C);
    statusBar(g, x, y + h - 2, w, hints([['r', 'retry'], ['k', 'new key']]));
    return;
  }
  rowPill(g, X - 1, yy, W2 + 2, T.card2); g.putSegs(X + 1, yy, [{ t: '› ', fg: T.acc, b: 1 }, ...(tk.q ? [{ t: tk.q, fg: T.strong }, { t: '█', fg: T.acc }] : [{ t: 'search aevox issues assigned to you', fg: T.dim, i: 1 }])], { bg: T.card2 }); yy += 2;
  if (tk.gh === 'loading') { g.putSegs(X, yy, [{ t: SPIN[s.f % 8] + ' ', fg: T.acc }, { t: 'Fetching issues from GitHub…', fg: T.text }], C); return; }
  const list = TICKETS.filter(t => !tk.q || t.t.toLowerCase().includes(tk.q.toLowerCase()));
  list.forEach((t, i) => {
    const sel = i === tk.row, bg = sel ? T.hov : T.card; if (sel) rowPill(g, X - 1, yy, W2 + 2, T.hov);
    const tag = t.in ? (t.in === 'queue' ? { t: 'queued', fg: T.dim } : { t: '⎇ ' + t.in, fg: A.green }) : { t: t.lab, fg: T.dim };
    g.putSegs(X + 1, yy, [{ t: '#' + t.n + '  ', fg: T.dim }, { t: t.t, fg: sel ? T.strong : T.text, b: sel }], { bg }, X + W2 - L(tag.t) - 2);
    right(g, X + W2 - 1, yy, [tag], { bg }); reg(X - 1, yy, W2 + 2, 1, { t: 'tkRow', n: i }); yy++;
  });
  if (!list.length) g.put(X, yy, 'No issue matches. Esc clears the search.', { fg: T.dim, i: 1, ...C });
  statusBar(g, x, y + h - 2, w, hints([['Enter', 'start an agent'], ['q', 'add to queue'], ['o', 'open']]), [{ t: 'own worktree each', fg: T.dim }]);
}
function queueBody(g, s, b, yy, reg) {
  const { X, W2, C, x, w, y, h } = b;
  g.putSegs(X, yy++, [{ t: 'Runs ', fg: T.dim }, { t: '3 at a time', fg: T.strong, b: 1 }, { t: W2 < 70 ? ', each in its own worktree.' : ', each in its own worktree, also with the window closed.', fg: T.dim }], C, X + W2);
  yy++;
  if (s.tk.add != null) { rowPill(g, X - 1, yy, W2 + 2, T.card2); g.putSegs(X + 1, yy, [{ t: '+ ', fg: T.acc, b: 1 }, ...(s.tk.add ? [{ t: s.tk.add, fg: T.strong }] : [{ t: 'describe the task…', fg: T.dim, i: 1 }]), { t: '█', fg: T.acc }], { bg: T.card2 }); yy += 2; }
  const G = { running: [SPIN[s.f % 8], T.text, 'running'], starting: ['◌', T.dim, 'starting'], waiting: ['·', T.dim, 'waiting'], review: ['✓', T.ok, 'ready for review'], failed: ['✕', T.err, 'failed'] };
  s.Q.forEach((q, i) => {
    const [gg, c, lab] = G[q.st], sel = i === s.tk.row && s.tk.add == null, bg = sel ? T.hov : T.card;
    if (sel) rowPill(g, X - 1, yy, W2 + 2, T.hov);
    const wait = q.st === 'waiting' ? '#' + (s.Q.filter(z => z.st === 'waiting').indexOf(q) + 1) + ' next' : '';
    g.putSegs(X + 1, yy, [{ t: gg + ' ', fg: c, b: 1 }, { t: q.t, fg: sel ? T.strong : T.text, b: sel }], { bg }, X + W2 - 22);
    right(g, X + W2 - 1, yy, [{ t: wait || lab, fg: c }, ...(q.age ? [{ t: ' · ' + q.age, fg: T.dim }] : [])], { bg });
    reg(X - 1, yy, W2 + 2, 1, { t: 'qRow', n: i }); yy++;
    if (q.st === 'failed') g.put(X + 3, yy++, q.why, { fg: T.err, i: 1, ...C });
    else if (q.wt && (q.st === 'running' || q.st === 'starting')) g.put(X + 3, yy++, '⎇ ' + q.wt, { fg: T.dim, ...C });
    else if (q.st === 'review') g.put(X + 3, yy++, q.chg + ' · in the Inbox', { fg: T.dim, ...C });
  });
  statusBar(g, x, y + h - 2, w, hints([['a', 'add task'], ['r', 'retry'], ['x', 'remove'], ['Enter', 'open']]), [{ t: 'limit in Settings', fg: T.dim }]);
}
function prBody(g, s, b, reg) {
  const { X, W2, Y, C, x, w, y, h, tgt } = b, z = s.S[tgt]; let yy = Y;
  if (s.pr === 'loading') { g.putSegs(X, yy, [{ t: SPIN[s.f % 8] + ' ', fg: T.acc }, { t: 'Fetching #412 from GitHub…', fg: T.text }], C); return; }
  if (s.pr === 'error') { g.putSegs(X, yy++, [{ t: '✕ ', fg: T.err, b: 1 }, { t: 'GitHub CLI isn’t signed in.', fg: T.strong, b: 1 }], C); g.putSegs(X, yy + 1, [{ t: 'Run ', fg: T.dim }, { t: 'gh auth login', fg: T.acc, b: 1 }, { t: ' in any shell, then ', fg: T.dim }, { t: 'r', fg: T.acc, b: 1 }, { t: ' to retry.', fg: T.dim }], C); return; }
  if (!z.pr) { g.put(X, yy++, 'No pull request for ' + tgt + ' yet.', { fg: T.strong, b: 1, ...C }); g.put(X, yy++, 'Ship commits, pushes and opens one into main.', { fg: T.dim, ...C }); yy++; g.putSegs(X, yy, btn('Ship', 'S', 'primary'), C); reg(X, yy, 10, 1, { t: 'open', ov: 'ship' }); return; }
  g.putSegs(X, yy++, [{ t: '#412 ', fg: T.dim }, { t: 'Coupons apply after tax', fg: T.strong, b: 1 }], C);
  g.putSegs(X, yy++, [{ t: '⎇ orders → main', fg: A.green }, { t: '  ·  ' + z.chg + '  ·  open, 2 reviews', fg: T.dim }], C, X + W2); yy++;
  sectionHd(g, X, yy++, W2, 'CHECKS', '2 of 4 failing', C);
  [['✓', T.ok, 'lint', '12s'], ['✓', T.ok, 'typecheck', '31s'], ['✕', T.err, 'test', '2 failed · checkout.spec.ts'], ['✕', T.err, 'e2e', 'payment redirect timed out']].forEach(([gg, c, n, d]) => { g.putSegs(X + 1, yy, [{ t: gg + ' ', fg: c, b: 1 }, { t: n.padEnd(11), fg: T.text }, { t: d, fg: T.dim }], C, X + W2); yy++; });
  yy++; sectionHd(g, X, yy++, W2, 'REVIEW COMMENTS', 2, C);
  [['maya', 'checkout.ts:88', 'Round half-even here, not half-up.'], ['jon', 'coupon.ts:14', 'Can this be a pure function?']].forEach(([who, at, t]) => { g.putSegs(X + 1, yy++, [{ t: '@' + who, fg: T.ws.violet, b: 1 }, { t: '  ' + at, fg: A.blue }], C); g.put(X + 3, yy++, trunc(t, W2 - 4), { fg: T.text, i: 1, ...C }); });
  yy++;
  let ax = g.putSegs(X, yy, btn('Hand to ' + tgt, 'F', 'primary'), C) + 1; reg(X, yy, ax - X - 1, 1, { t: 'handoff', id: tgt });
  g.putSegs(ax + 1, yy, [{ t: 'sends the 2 failures and 2 comments as its next prompt', fg: T.dim, i: 1 }], C, X + W2);
  statusBar(g, x, y + h - 2, w, hints([['F', 'hand to agent'], ['d', 'diff'], ['S', 'ship'], ['o', 'on GitHub']]));
}
function changesBody(g, s, b, reg) {
  const { X, W2, Y, C, x, w, y, h, tgt } = b; let yy = Y;
  const hunk = (who, rows) => { g.putSegs(X, yy++, [{ t: '⎇ ' + who, fg: A.green, b: 1 }, { t: '  src/checkout.ts', fg: A.blue }], C); rows.forEach(([n, sg, t]) => { const c = sg === '+' ? A.green : sg === '−' ? A.red : T.dim; g.putSegs(X + 1, yy++, [{ t: String(n).padStart(3) + ' ', fg: T.dim }, { t: sg + ' ', fg: c }, { t: t, fg: sg === ' ' ? T.text : c }], C, X + W2); }); yy++; };
  if (s.sheet.conf) {
    g.putSegs(X, yy++, [{ t: '⇆ ', fg: CONF, b: 1 }, { t: 'orders and rate-limit both changed checkout.ts, lines 80–96', fg: T.strong }], C, X + W2); yy++;
    hunk('orders', [[86, ' ', 'const sub = subtotal(cart);'], [87, '−', 'const total = applyCoupon(sub) * (1 + tax);'], [87, '+', 'const taxed = sub * (1 + tax);'], [88, '+', 'const total = roundHalfEven(applyCoupon(taxed));']]);
    hunk('rate-limit', [[86, ' ', 'const sub = subtotal(cart);'], [87, '+', 'await bucket.take(req.ip);'], [88, ' ', 'const total = applyCoupon(sub) * (1 + tax);']]);
    g.put(X, yy, trunc('Merging both will conflict at line 87. Tell one agent to rebase on the other, or keep going.', W2), { fg: T.dim, i: 1, ...C });
    statusBar(g, x, y + h - 2, w, hints([['m', 'tell orders'], ['M', 'tell rate-limit'], ['k', 'dismiss']]));
    return;
  }
  const z = s.S[tgt];
  g.putSegs(X, yy++, [{ t: z.chg || 'no changes', fg: T.strong, b: 1 }, { t: '  on ⎇ ' + z.br, fg: T.dim }], C); yy++;
  [['M', 'src/search/input.ts', '+18 −4'], ['M', 'src/search/useSearch.ts', '+20 −3'], ['A', 'src/search/debounce.test.ts', '+4']].forEach(([m, f, n], i) => { const sel = i === 0, bg = sel ? T.hov : T.card; if (sel) rowPill(g, X - 1, yy, W2 + 2, T.hov); g.putSegs(X + 1, yy, [{ t: m + ' ', fg: m === 'A' ? A.green : A.yellow, b: 1 }, { t: f, fg: sel ? T.strong : T.text }], { bg }); right(g, X + W2 - 1, yy, [{ t: n, fg: T.dim }], { bg }); yy++; });
  yy++;
  [['@@ -12,6 +12,9 @@', 'h'], ['  export function SearchInput() {', ' '], ['−   onChange={e => search(e.target.value)}', '−'], ['+   const run = useDebounce(search, 150);', '+'], ['+   onChange={e => run(e.target.value)}', '+'], ['  }', ' ']].forEach(([t, k]) => { g.put(X + 1, yy++, trunc(t, W2 - 2), { fg: k === 'h' ? A.cyan : k === '+' ? A.green : k === '−' ? A.red : T.text, ...C }); });
  statusBar(g, x, y + h - 2, w, hints([['M', 'merge'], ['x', 'throw away'], ['O', 'open :3002'], ['r', 'review mark']]));
}
function serverBody(g, s, b, reg) {
  const { X, W2, Y, C, x, w, y, h, tgt } = b, z = s.S[tgt], sv = z.srv || { st: 'none' }; let yy = Y;
  if (sv.st === 'none') {
    g.put(X, yy++, 'No run command for ' + z.folder + ' yet.', { fg: T.strong, b: 1, ...C });
    g.put(X, yy++, 'Saved to .seshi.toml; every worktree gets its own port.', { fg: T.dim, ...C }); yy++;
    rowPill(g, X - 1, yy, W2 + 2, T.card2); g.putSegs(X + 1, yy, [{ t: 'run ', fg: T.dim }, { t: 'npm run dev -- --port $PORT', fg: T.strong }, { t: '█', fg: T.acc }], { bg: T.card2 }); yy += 2;
    g.putSegs(X, yy, btn('Save and run', 'Enter', 'primary'), C);
    statusBar(g, x, y + h - 2, w, hints([['Enter', 'save'], ['Esc', 'cancel']]));
    return;
  }
  const stl = sv.st === 'ready' ? [{ t: '▶ ready', fg: T.ok, b: 1 }, { t: '  localhost:' + sv.port, fg: A.blue, u: 1 }] : sv.st === 'starting' ? [{ t: SPIN[s.f % 8] + ' starting', fg: T.text, b: 1 }, { t: '  :' + sv.port, fg: T.dim }] : sv.st === 'crashed' ? [{ t: '✕ crashed', fg: T.err, b: 1 }, { t: '  exit 1 · 40s ago', fg: T.dim }] : [{ t: '■ stopped', fg: T.dim, b: 1 }];
  g.putSegs(X, yy++, [{ t: 'npm run dev', fg: T.strong }, { t: '  ·  ', fg: T.dim }, ...stl], C, X + W2); yy++;
  const log = sv.st === 'crashed' ? [['> vite --port 3000', T.dim], ['', T.dim], ['✕ Error: Cannot find module \'./tax\'', A.red], ['  at src/checkout.ts:3:1', T.dim], ['', T.dim], ['Process exited with code 1', A.red]] : [['> vite --port ' + sv.port, T.dim], ['', T.dim], ['  VITE v5.4  ready in 412 ms', A.green], ['  ➜  Local:   http://localhost:' + sv.port + '/', A.cyan], ['', T.dim], ['  10:42:07 [vite] hmr update /src/checkout.ts', T.dim], ['  10:42:31 [vite] page reload src/coupon.ts', T.dim]];
  log.forEach(([t, c]) => g.put(X + 1, yy++, trunc(t, W2 - 2), { fg: c, ...C }));
  statusBar(g, x, y + h - 2, w, sv.st === 'crashed' ? hints([['U', 'restart'], ['m', 'ask ' + tgt + ' to fix']]) : hints([['u', sv.st === 'stopped' ? 'run' : 'stop'], ['U', 'restart'], ['O', 'open in browser']]), [{ t: 'port per worktree', fg: T.dim }]);
}
function cpBody(g, s, b, reg) {
  const { X, W2, Y, C, x, w, y, h, tgt } = b; let yy = Y;
  if (s.remote) { g.put(X, yy++, 'Checkpoints don’t work over SSH yet.', { fg: T.strong, b: 1, ...C }); g.put(X, yy, 'They need the folder on this machine. Coming in 0.17.', { fg: T.dim, ...C }); return; }
  if (!s.cp.on) {
    g.put(X, yy++, 'Checkpoints are off.', { fg: T.strong, b: 1, ...C });
    ['When on, Seshi saves this folder after every agent turn,', 'so you can roll back to any of them. It uses git refs;', 'nothing leaves your machine and the branch isn’t touched.'].forEach(l => g.put(X, yy++, l, { fg: T.dim, ...C }));
    yy++; g.putSegs(X, yy, btn('Turn on for aevox', 'Enter', 'primary'), C); reg(X, yy, 26, 1, { t: 'cpOn' });
    statusBar(g, x, y + h - 2, w, hints([['Enter', 'turn on'], ['Esc', 'not now']]));
    return;
  }
  g.putSegs(X, yy++, [{ t: '⎇ ' + tgt, fg: A.green, b: 1 }, { t: '  · one checkpoint per turn', fg: T.dim }], C); yy++;
  CPS.forEach(([t, turn, what, chg], i) => {
    const sel = i === s.cp.row, bg = sel ? T.hov : T.card; if (sel) rowPill(g, X - 1, yy, W2 + 2, T.hov);
    g.putSegs(X + 1, yy, [{ t: (i === 0 ? '● ' : '○ '), fg: i === 0 ? T.acc : T.dim }, { t: t + '  ', fg: T.dim }, { t: turn.padEnd(8), fg: T.text }, { t: what, fg: sel ? T.strong : T.text, b: sel }], { bg }, X + W2 - L(chg) - 2);
    if (chg) right(g, X + W2 - 1, yy, [{ t: chg, fg: T.dim }], { bg });
    reg(X - 1, yy, W2 + 2, 1, { t: 'cpRow', n: i }); yy++;
  });
  statusBar(g, x, y + h - 2, w, hints([['↑↓', 'pick'], ['Enter', 'roll back to it'], ['d', 'diff from now']]));
}

// ---------- popovers ----------
function popAt(Ly, rowY, id, w, h) { const x = Ly.M + Ly.SW + 1, yr = rowY[id] != null ? rowY[id] : 6; return { x, y: Math.max(1, Math.min(Ly.H - 1 - h, yr - 1)), w: Math.min(w, Ly.W - x - 2), h }; }
function composePop(g, s, Ly, rowY, reg) {
  const id = s.compose.id, z = s.S[id], ww = Math.min(76, Ly.W - Ly.M - Ly.SW - 3), rows = Math.max(3, Math.min(10, wrapText(s.compose.text || '', ww - 10).length)), { x, y, w, h } = popAt(Ly, rowY, id, 76, rows + 8);
  frame(g, x, y, w, h, { focus: 1, bg: T.card, title: [{ t: 'message ', fg: T.text }, { t: id, fg: T.acc, b: 1 }], right: [{ t: z.agent, fg: T.dim }] });
  g.put(x + 3, y + 2, trunc((z.st === 'needs' ? z.q : '“' + z.last + '”'), w - 6), { fg: z.st === 'needs' ? T.needs : T.dim, i: 1, bg: T.card });
  composeInline(g, s, x + 2, y + 4, w - 4, T.card, 10);
  g.set(x, y + 2, '◂', { fg: T.acc, bg: DESK, b: 1 });
}
function whyPop(g, s, Ly, rowY, reg) {
  const id = s.why, z = s.S[id], { x, y, w, h } = popAt(Ly, rowY, id, 64, 9);
  frame(g, x, y, w, h, { focus: 1, bg: T.card, title: [{ t: 'why ', fg: T.text }, { t: gl(z.st, s.f) + ' ' + STN[z.st], fg: SC[z.st], b: 1 }] });
  const rows = z.st === 'needs' ? [['hook', 'Notification: permission prompt', '38s ago', 1], ['screen', 'matched “Do you want to”', '2s ago'], ['process', 'claude running · pid 4412', 'now']] : z.st === 'done' ? [['hook', 'Stop: turn finished', '2m ago', 1], ['screen', 'prompt is idle', '2m ago'], ['process', 'claude running · pid 3980', 'now']] : [['screen', 'matched “esc to interrupt”', '1s ago', 1], ['hook', 'none since the last prompt', ''], ['process', (z.agent || 'shell') + ' running', 'now']];
  rows.forEach(([src, d, t, win], i) => { const yy = y + 2 + i; g.putSegs(x + 3, yy, [{ t: src.padEnd(9), fg: win ? T.strong : T.dim, b: win }, { t: d, fg: win ? T.text : T.dim }], { bg: T.card }, x + w - 12); right(g, x + w - 4, yy, [{ t: t, fg: T.dim }, { t: win ? '  ◂' : '   ', fg: T.acc, b: 1 }], { bg: T.card }); });
  g.put(x + 3, y + 6, 'The most specific source wins: hook, then screen, then process.', { fg: T.dim, i: 1, bg: T.card });
  g.set(x, y + 2, '◂', { fg: T.acc, bg: DESK, b: 1 });
}
function toast(g, s, Ly) {
  const segs = pill([{ t: ' ' + s.toast + ' ', fg: T.strong }], T.card2);
  right(g, Ly.x0 + Ly.aw - 3, Ly.H - 2, segs);
}

// ---------- modals ----------
function center(s, w, h) { return { x: Math.floor((s.W - w) / 2), y: Math.floor((s.H - h) / 2) }; }
function ship(g, s, reg) {
  const id = s.ship.id, z = s.S[id], w = Math.min(76, s.W - 6), h = 15, { x, y } = center(s, w, h), C = { bg: T.card };
  frame(g, x, y, w, h, { focus: 1, bg: T.card, title: [{ t: 'Ship ', fg: T.text }, { t: id, fg: T.acc, b: 1 }] });
  g.put(x + 3, y + 2, 'This will, in order:', { fg: T.dim, ...C });
  const steps = [['Commit 4 changed files as', '“orders: coupons apply after tax”'], ['Push ⎇ orders to origin', '3 new commits'], [z.pr ? 'Update pull request #412' : 'Open a pull request into main', z.pr ? 'checks run again' : 'as a draft']];
  steps.forEach(([a, b2], i) => { const yy = y + 4 + i * 2, done = s.ship.step > i, run = s.ship.step === i && s.ship.busy; g.putSegs(x + 3, yy, [{ t: done ? '✓ ' : run ? SPIN[s.f % 8] + ' ' : (i + 1) + ' ', fg: done ? T.ok : T.acc, b: 1 }, { t: a + ' ', fg: T.strong }, { t: b2, fg: T.dim }], C, x + w - 3); });
  g.put(x + 3, y + 10, 'Nothing is merged. You can still change the PR on GitHub.', { fg: T.dim, i: 1, ...C });
  if (!s.ship.busy) { let ax = g.putSegs(x + 3, y + 12, btn('Ship', 'Enter', 'primary')) + 2; reg(x + 3, y + 12, 12, 1, { t: 'shipGo' }); ax = g.putSegs(ax, y + 12, btn('Edit message', 'e')) + 2; g.putSegs(ax, y + 12, btn('Cancel', 'Esc')); }
}
function rollback(g, s, reg) {
  const cp = CPS[s.cp.row], w = Math.min(74, s.W - 6), h = 14, { x, y } = center(s, w, h), C = { bg: T.card };
  frame(g, x, y, w, h, { focus: 1, bg: T.card, title: [{ t: 'Roll back ', fg: T.text }, { t: 'orders', fg: T.acc, b: 1 }] });
  g.putSegs(x + 3, y + 2, [{ t: 'Back to ', fg: T.text }, { t: cp[0] + ' · ' + cp[1], fg: T.strong, b: 1 }, { t: '  “' + cp[2] + '”', fg: T.dim }], C, x + w - 3);
  g.put(x + 3, y + 4, '3 files go back:  checkout.ts  coupon.ts  tax.ts', { fg: T.text, ...C });
  g.put(x + 3, y + 5, '1 file is removed:  tax.test.ts', { fg: T.text, ...C });
  g.putSegs(x + 3, y + 7, [{ t: '✓ ', fg: T.ok, b: 1 }, { t: 'What’s there now is saved as a checkpoint first, so this can be undone.', fg: T.dim }], C, x + w - 3);
  g.put(x + 3, y + 8, 'The agent keeps running; tell it what changed.', { fg: T.dim, ...C });
  let ax = g.putSegs(x + 3, y + 11, btn('Roll back', 'Enter', 'danger')) + 2; reg(x + 3, y + 11, 17, 1, { t: 'rbGo' }); g.putSegs(ax, y + 11, btn('Cancel', 'Esc'));
}
function settings(g, s, reg) {
  const w = Math.min(90, s.W - 6), h = Math.min(28, s.H - 4), { x, y } = center(s, w, h), C = { bg: T.card }, st = s.set;
  frame(g, x, y, w, h, { focus: 1, bg: T.card, title: [{ t: 'Settings', fg: T.acc, b: 1 }], right: [{ t: 'Esc ✕', fg: T.dim }] });
  let tx = x + 3;
  [['general', 'General'], ['appearance', 'Appearance'], ['sync', 'Sync'], ['alerts', 'Alerts'], ['queue', 'Queue']].forEach(([k, l]) => { const on = st.tab === k, x0 = tx; tx = g.putSegs(tx, y + 2, on ? pill([{ t: ' ' + l + ' ', fg: T.accInk, b: 1 }], T.acc) : [{ t: '  ' + l + '  ', fg: T.text, ...C }]) + 1; reg(x0, y + 2, tx - x0 - 1, 1, { t: 'setTab', k }); });
  g.hl(x + 3, y + 3, w - 6, '─', { fg: T.line, ...C });
  const X = x + 4, W2 = w - 8; let yy = y + 5;
  const opt = (label, opts, yy0, sel) => { if (sel) rowPill(g, x + 2, yy0, w - 4, T.hov); const bg = sel ? T.hov : T.card; g.put(X, yy0, label, { fg: sel ? T.strong : T.text, b: sel, bg }); let vx = X + 26; opts.forEach(([o, on]) => { vx = g.putSegs(vx, yy0, on ? pill([{ t: o, fg: T.accInk, b: 1 }], T.acc) : [{ t: ' ' + o + ' ', fg: T.dim, bg }]) + 1; }); };
  if (st.tab === 'sync') {
    g.put(X, yy++, 'Keep the same settings on every machine, through a private GitHub repo.', { fg: T.text, ...C }); yy++;
    if (st.sync === 'off' || st.sync === 'connecting') {
      g.putSegs(X, yy, [{ t: '1 ', fg: T.acc, b: 1 }, { t: 'Repo', fg: T.strong }], C); rowPill(g, X + 12, yy, 40, T.card2); g.putSegs(X + 14, yy, [{ t: st.repo, fg: T.strong }, { t: '█', fg: T.acc }], { bg: T.card2 }); g.put(X + 54, yy++, 'created if missing, private', { fg: T.dim, ...C }); yy++;
      g.putSegs(X, yy, [{ t: '2 ', fg: T.acc, b: 1 }, { t: 'Uses your ', fg: T.strong }, { t: 'gh', fg: T.acc, b: 1 }, { t: ' login (signed in as cstin)', fg: T.strong }], C); yy += 2;
      if (st.sync === 'connecting') g.putSegs(X, yy, [{ t: SPIN[s.f % 8] + ' ', fg: T.acc }, { t: 'Creating repo and pushing settings…', fg: T.text }], C);
      else { g.putSegs(X, yy, btn('Connect', 'Enter', 'primary'), C); reg(X, yy, 15, 1, { t: 'syncGo' }); }
    } else if (st.sync === 'error') {
      g.putSegs(X, yy++, [{ t: '✕ ', fg: T.err, b: 1 }, { t: 'cstin/seshi-config is public. Seshi only syncs to private repos.', fg: T.strong }], C); yy++;
      g.putSegs(X, yy, btn('Use another name', 'Enter', 'primary'), C);
    } else {
      g.putSegs(X, yy++, [{ t: '✓ ', fg: T.ok, b: 1 }, { t: 'Synced 2 minutes ago', fg: T.strong, b: 1 }, { t: ' · cstin/seshi-config · 3 machines', fg: T.dim }], C); yy++;
      g.putSegs(X, yy++, [{ t: 'Synced     ', fg: T.dim }, { t: 'theme, keys, agents, presets, recipes, queue limit', fg: T.text }], C);
      g.putSegs(X, yy++, [{ t: 'This only  ', fg: T.dim }, { t: 'window size, SSH hosts, API keys, ntfy topic', fg: T.text }], C); yy++;
      let ax = g.putSegs(X, yy, btn('Sync now', 's')) + 2; g.putSegs(ax, yy, btn('Disconnect', 'x'));
    }
  } else if (st.tab === 'alerts') {
    const al = st.al;
    g.put(X, yy++, 'Get a push on your phone when an agent needs you. Off by default.', { fg: T.text, ...C }); yy++;
    opt('Phone alerts', [['on', al.on], ['off', !al.on]], yy++, 1); yy++;
    g.putSegs(X, yy, [{ t: 'Topic', fg: T.text }], C); rowPill(g, X + 25, yy, 30, T.card2); g.putSegs(X + 27, yy, [{ t: al.topic, fg: T.strong }], { bg: T.card2 }); g.putSegs(X + 57, yy++, [{ t: 'n', fg: T.acc, b: 1 }, { t: ' new random', fg: T.dim }], C);
    g.put(X + 26, yy++, trunc('Subscribe to it in the ntfy app (iOS, Android). No account.', W2 - 26), { fg: T.dim, i: 1, ...C });
    g.put(X + 26, yy++, trunc('The topic name is the only secret on the public server.', W2 - 26), { fg: T.needs, i: 1, ...C }); yy++;
    g.put(X, yy, 'What it says', { fg: T.text, ...C });
    [['name', '“claude in AeVox needs you”'], ['text', 'also the question: “Allow edit to src/checkout.ts?”']].forEach(([k, l], i) => { const on = al.mode === k; g.putSegs(X + 26, yy + i, [{ t: on ? '(•) ' : '( ) ', fg: on ? T.acc : T.dim, b: 1 }, { t: l, fg: on ? T.strong : T.text }], C, x + w - 3); reg(X + 26, yy + i, 50, 1, { t: 'alMode', k }); });
    yy += 3;
    g.put(X + 26, yy++, trunc('Question text may include code or paths; anyone with the topic can read it.', W2 - 26), { fg: T.dim, i: 1, ...C }); yy++;
    const tst = al.test === 'sending' ? [{ t: SPIN[s.f % 8] + ' sending…', fg: T.text }] : al.test === 'sent' ? [{ t: '✓ Sent. Check your phone.', fg: T.ok, b: 1 }] : al.test === 'fail' ? [{ t: '✕ ntfy.sh didn’t answer. Check your connection.', fg: T.err }] : [];
    const ax = g.putSegs(X + 26, yy, btn('Send a test', 't', 'primary'), C); reg(X + 26, yy, ax - X - 26, 1, { t: 'alTest' }); g.putSegs(ax + 2, yy, tst, C);
  } else if (st.tab === 'queue') {
    opt('Agents at a time', [['1'], ['2'], ['3', 1], ['5'], ['8']], yy++, 1); yy++;
    opt('Keep running when closed', [['yes', 1], ['no']], yy++); yy++;
    opt('Agent for queued work', [['claude', 1], ['codex']], yy++);
  } else { g.put(X, yy, 'Built in 0.15. See the floating handoff.', { fg: T.dim, i: 1, ...C }); }
  statusBar(g, x, y + h - 2, w, hints([['Tab', 'section'], ['↑↓', 'move'], ['←→', 'change']]), st.sync === 'synced' ? [{ t: '✓ synced', fg: T.ok }] : [{ t: 'not synced', fg: T.dim }], T.card2);
}
function keymap(g, s, reg) {
  const w = Math.min(118, s.W - 4), h = 22, { x, y } = center(s, w, h), C = { bg: T.card };
  frame(g, x, y, w, h, { focus: 1, border: SKY, bg: T.card, title: [{ t: 'Ctrl+Space', fg: SKY, b: 1 }], right: [{ t: 'Esc ✕', fg: T.dim }] });
  const groups = [['AGENTS', [['j', 'inbox'], ['m', 'follow-up'], ['1-3', 'answer'], ['i', 'why this status']]], ['WORK', [['T', 'tickets'], ['Q', 'queue'], ['P', 'pull request'], ['S', 'ship'], ['C', 'checkpoints']]], ['SERVER', [['u', 'run / stop'], ['U', 'restart'], ['O', 'open in browser'], ['l', 'output']]], ['PANES', [['p', 'new pane'], ['z', 'zoom'], ['x', 'close'], ['Alt+←→', 'move']]], ['PROJECT', [['w', 'worktrees ›'], ['f', 'files'], ['d', 'changes'], ['g', 'branches']]], ['SESHI', [[',', 'settings'], ['a', 'actions'], ['?', 'this map'], ['q', 'quit']]]];
  const cw = Math.floor((w - 6) / 3);
  groups.forEach(([n, ks], gi) => { const gx = x + 3 + (gi % 3) * cw, gy = y + 2 + Math.floor(gi / 3) * 9; g.put(gx, gy, n, { fg: T.dim, b: 1, ...C }); ks.forEach(([k, l], i) => { const e = g.putSegs(gx, gy + 2 + i, kc(k, gi === 1 && i === 0)); g.put(Math.max(e + 1, gx + 9), gy + 2 + i, trunc(l, cw - 10), { fg: T.text, ...C }); }); });
  g.put(x + 3, y + h - 2, 'New in v4: m T Q P S C u U O l i. Bare keys work when the sidebar has focus.', { fg: T.dim, i: 1, ...C });
}
function splash(g, s, reg) {
  const { W, H } = s, BIG = { S: ['█████', '█    ', '█████', '    █', '█████'], E: ['█████', '█    ', '████ ', '█    ', '█████'], H: ['█   █', '█   █', '█████', '█   █', '█   █'], I: ['█████', '  █  ', '  █  ', '  █  ', '█████'] };
  const word = 'SESHI', lw = word.length * 7 - 2, lx = Math.floor((W - lw) / 2), ly = Math.max(1, Math.floor(H / 2) - 15);
  [...word].forEach((ch, k) => BIG[ch].forEach((row, r) => [...row].forEach((c, ci) => { if (c !== ' ') g.set(lx + k * 7 + ci, ly + r, '█', { fg: mix(T.acc, T.ws.teal, (k * 7 + ci) / lw) }); })));
  const c = (yy, segs) => g.putSegs(Math.floor((W - segLen(segs)) / 2), yy, segs);
  c(ly + 7, [{ t: 'while you were away   ', fg: T.dim }, { t: '● 1 needs you', fg: T.needs, b: 1 }, { t: '   ' + SPIN[s.f % 8] + ' 2 working', fg: T.text }, { t: '   ✓ 2 finished', fg: T.ok }]);
  const tw = Math.min(78, W - 8), tx = Math.floor((W - tw) / 2), ty = ly + 10, rows = [['orders', 'claude', '3 tasks', '1h 52m', '$2.40'], ['search', 'claude', '2 tasks', '1h 10m', '$1.75'], ['rate-limit', 'codex', '1 task', '48m', '$0.90'], ['docs', 'claude', '1 task', '22m', '$0.61']];
  const th = rows.length + 6;
  frame(g, tx, ty, tw, th, { bg: PB, title: [{ t: 'TODAY', fg: T.text, b: 1 }, { t: ' since 8:02', fg: T.dim }] });
  const cols = [3, 18, 28, 40, 54];
  rows.forEach((r, i) => { const yy = ty + 2 + i; r.forEach((v, j) => g.put(tx + cols[j] + (j === 4 ? tw - 64 : 0), yy, v, { fg: j === 0 ? T.strong : j === 4 ? T.text : T.dim, b: j === 0 })); const bw = Math.round(({ '1h 52m': 112, '1h 10m': 70, '48m': 48, '22m': 22 })[r[3]] / 112 * 10); g.put(tx + 48, yy, '━'.repeat(bw), { fg: mix(T.acc, PB, 0.35) }); });
  g.hl(tx + 3, ty + th - 3, tw - 6, '─', { fg: T.line });
  g.putSegs(tx + 3, ty + th - 2, [{ t: '7 tasks done', fg: T.strong, b: 1 }, { t: ' · 4h 12m of agent time · ', fg: T.dim }, { t: '$5.66', fg: T.strong, b: 1 }], {}, tx + tw - 3);
  const by = ty + th + 2; let segs = [...btn('Resume', 'Enter', 'primary'), { t: '   ' }, ...btn('New shell here', 'n')]; const bx = Math.floor((W - segLen(segs)) / 2); g.putSegs(bx, by, segs); reg(bx, by, 14, 1, { t: 'enter' });
}

// ---------- state machine ----------
const ret = (s, fx) => ({ state: s, fx: fx || [] });
const say = (s, m) => { s.toast = m; return [{ after: 2600, act: { t: 'untoast', m } }]; };
function openSheet(s, k, extra) { s.sheet = { k, ...(extra || {}) }; s.compose = null; s.why = null; }
export function act(s0, a) {
  const s = structuredClone(s0);
  if (a.t === 'tick') { s.f++; return ret(s); }
  if (a.t === 'untoast') { if (s.toast === a.m) s.toast = null; return ret(s); }
  if (a.t === 'size') { s.W = a.W; s.H = a.H; return ret(s); }
  if (a.t === 'leader') { s.leader = s.leader ? 0 : 1; return ret(s); }
  if (a.t === 'later') { return a.fn(s); }
  if (a.t === 'key') return key(s, a);
  return click(s, a);
}
function click(s, a) {
  switch (a.t) {
    case 'enter': s.ov = null; return ret(s);
    case 'sel': s.sel = a.id; s.why = null; return ret(s);
    case 'focus': s.focus = a.id; s.sel = a.id; return ret(s);
    case 'close': s.sheet = null; return ret(s);
    case 'open': s.ov = a.ov; if (a.ov === 'ship') s.ship = { id: s.sel, step: 0, busy: 0 }; return ret(s);
    case 'toast': return ret(s, say(s, a.m));
    case 'sheet': openSheet(s, a.k); if (a.tab) s.tk.tab = a.tab; return ret(s);
    case 'irow': s.inbox.row = a.n; return ret(s);
    case 'answer': return answer(s, a.id, a.n);
    case 'compose': s.compose = { id: a.id, text: '', from: a.from }; return ret(s);
    case 'confDiff': openSheet(s, 'changes', { conf: 1 }); return ret(s);
    case 'diff': openSheet(s, 'changes', { id: a.id }); return ret(s);
    case 'merge': s.inbox.confirm = { id: a.id, k: 'merge' }; return ret(s);
    case 'tkTab': s.tk.tab = a.k; s.tk.row = 0; return ret(s);
    case 'tkRow': s.tk.row = a.n; return startTicket(s);
    case 'qRow': s.tk.row = a.n; return ret(s);
    case 'handoff': return handoff(s, a.id);
    case 'cpOn': s.cp.on = 1; return ret(s, say(s, 'Checkpoints on for aevox'));
    case 'cpRow': s.cp.row = a.n; s.ov = 'rollback'; return ret(s);
    case 'rbGo': s.ov = null; return ret(s, say(s, 'Rolled orders back to ' + CPS[s.cp.row][0] + ' · saved the current state first'));
    case 'shipGo': return shipGo(s);
    case 'setTab': s.set.tab = a.k; return ret(s);
    case 'syncGo': return syncGo(s);
    case 'alMode': s.set.al.mode = a.k; return ret(s);
    case 'alTest': return alTest(s);
  }
  return null;
}
function answer(s, id, n) {
  const z = s.S[id]; if (!z || z.st !== 'needs') return null;
  z.st = 'work'; z.q = null; z.age = 'now'; z.sent = ['Yes', 'Yes, always', 'No'][n - 1];
  return ret(s, say(s, 'Answered ' + id + ': ' + z.sent));
}
function handoff(s, id) { const z = s.S[id]; z.st = 'work'; z.sent = 'Fix the 2 failing checks and address 2 review comments on #412'; s.sheet = null; return ret(s, say(s, 'Sent 2 failures and 2 comments to ' + id)); }
function startTicket(s) {
  const t = TICKETS[s.tk.row]; if (!t || t.in) return ret(s, say(s, t && t.in ? '#' + t.n + ' is already ' + (t.in === 'queue' ? 'queued' : 'in ⎇ ' + t.in) : ''));
  const id = 'coupons-' + t.n; s.S[id] = { sec: 'agents', folder: 'aevox', agent: 'claude', br: id, st: 'work', age: 'now', srv: { st: 'starting', port: 3004 }, last: 'Reading issue #' + t.n + ': ' + t.t };
  t.in = id; s.sel = id; return ret(s, say(s, 'claude started on #' + t.n + ' in ⎇ ' + id));
}
function shipGo(s) {
  s.ship.busy = 1; s.ship.step = 0;
  const step = n => ({ after: 700 * (n + 1), act: { t: 'later', fn: st => { if (!st.ship) return ret(st); st.ship.step = n + 1; if (n === 2) { st.ov = null; st.ship = null; return ret(st, say(st, 'Shipped orders · PR #412 updated · checks running')); } return ret(st); } } });
  return ret(s, [step(0), step(1), step(2)]);
}
function syncGo(s) { s.set.sync = 'connecting'; return ret(s, [{ after: 1600, act: { t: 'later', fn: st => { st.set.sync = 'synced'; return ret(st); } } }]); }
function alTest(s) { s.set.al.test = 'sending'; return ret(s, [{ after: 1200, act: { t: 'later', fn: st => { st.set.al.test = 'sent'; return ret(st); } } }]); }
const printable = k => k.length === 1;
function key(s, a) {
  const k = a.key;
  if (s.ov === 'splash') { if (k === 'Enter') { s.ov = null; return ret(s); } if (k === 'n') { s.ov = null; return ret(s, say(s, 'New shell in ~')); } return null; }
  if (s.ov === 'ship') { if (k === 'Escape' && !s.ship.busy) { s.ov = null; s.ship = null; return ret(s); } if (k === 'Enter' && !s.ship.busy) return shipGo(s); return ret(s); }
  if (s.ov === 'rollback') { if (k === 'Escape') { s.ov = null; return ret(s); } if (k === 'Enter') return click(s, { t: 'rbGo' }); return ret(s); }
  if (s.ov === 'keys') { if (k === 'Escape' || k === '?') { s.ov = null; return ret(s); } s.ov = null; return key(s, a); }
  if (s.ov === 'settings') {
    const tabs = ['general', 'appearance', 'sync', 'alerts', 'queue'];
    if (k === 'Escape' || k === ',') { s.ov = null; return ret(s); }
    if (k === 'Tab') { s.set.tab = tabs[(tabs.indexOf(s.set.tab) + 1) % tabs.length]; return ret(s); }
    if (s.set.tab === 'sync' && k === 'Enter' && s.set.sync === 'off') return syncGo(s);
    if (s.set.tab === 'alerts') { if (k === 't') return alTest(s); if (k === 'ArrowLeft' || k === 'ArrowRight') { s.set.al.on = s.set.al.on ? 0 : 1; return ret(s); } if (k === 'ArrowDown' || k === 'ArrowUp') { s.set.al.mode = s.set.al.mode === 'name' ? 'text' : 'name'; return ret(s); } if (k === 'n') { s.set.al.topic = 'seshi-' + Math.random().toString(36).slice(2, 8); return ret(s); } }
    return ret(s);
  }
  if (s.compose) {
    const c = s.compose;
    if (k === 'Escape') { s.compose = null; return ret(s); }
    if (k === 'Enter' && a.shift) { c.text += '\n'; return ret(s); }
    if (k === 'Enter') { const z = s.S[c.id]; const txt = c.text.trim() || 'keep going'; z.sent = txt.split('\n')[0]; z.st = 'work'; z.q = null; z.age = 'now'; s.compose = null; return ret(s, say(s, '✓ Sent to ' + c.id + ' · you stayed here')); }
    if (k === 'Backspace') { c.text = c.text.slice(0, -1); return ret(s); }
    if (printable(k)) { c.text += k; return ret(s); }
    return ret(s);
  }
  if (s.leader) { s.leader = 0; if (k === 'Escape') return ret(s); if (k === '?') { s.ov = 'keys'; return ret(s); } }
  if (s.why && (k === 'Escape' || k === 'i')) { s.why = null; return ret(s); }
  if (s.sheet) { const r = sheetKey(s, a); if (r) return r; }
  const vis = visible(s), i = vis.indexOf(s.sel);
  switch (k) {
    case 'ArrowDown': s.sel = vis[Math.min(vis.length - 1, i + 1)]; s.why = null; return ret(s);
    case 'ArrowUp': s.sel = vis[Math.max(0, i - 1)]; s.why = null; return ret(s);
    case 'Enter': s.focus = s.sel; return ret(s);
    case 'Escape': s.sheet = null; s.why = null; return ret(s);
    case 'm': if (s.S[s.sel].agent) { s.compose = { id: s.sel, text: '', from: 'side' }; return ret(s); } return null;
    case 'i': s.why = s.sel; return ret(s);
    case 'j': if (s.sheet && s.sheet.k === 'inbox') s.sheet = null; else openSheet(s, 'inbox'); return ret(s);
    case 'T': openSheet(s, 'tickets'); s.tk.tab = 'github'; return ret(s);
    case 'Q': openSheet(s, 'tickets'); s.tk.tab = 'queue'; s.tk.row = 0; return ret(s);
    case 'P': openSheet(s, 'pr', { id: s.sel }); return ret(s);
    case 'S': s.ov = 'ship'; s.ship = { id: s.sel, step: 0, busy: 0 }; return ret(s);
    case 'C': openSheet(s, 'cp', { id: s.sel }); return ret(s);
    case 'l': openSheet(s, 'server', { id: s.sel }); return ret(s);
    case 'u': { const v = s.S[s.sel].srv; if (!v || v.st === 'none') { openSheet(s, 'server', { id: s.sel }); return ret(s); } v.st = v.st === 'ready' || v.st === 'starting' ? 'stopped' : 'starting'; const fx = say(s, (v.st === 'stopped' ? 'Stopped' : 'Starting') + ' :' + v.port); if (v.st === 'starting') fx.push({ after: 1500, act: { t: 'later', fn: st => { st.S[s.sel].srv.st = 'ready'; return ret(st); } } }); return ret(s, fx); }
    case 'U': { const v = s.S[s.sel].srv; if (!v || v.st === 'none') return null; v.st = 'starting'; return ret(s, [...say(s, 'Restarting :' + v.port), { after: 1500, act: { t: 'later', fn: st => { st.S[s.sel].srv.st = 'ready'; return ret(st); } } }]); }
    case 'O': { const v = s.S[s.sel].srv; return ret(s, say(s, s.remote ? 'Not over SSH yet: open-in-browser needs a port forward' : v && v.st === 'ready' ? 'Opened localhost:' + v.port + ' in your browser' : 'No server running for ' + s.sel)); }
    case ',': s.ov = 'settings'; return ret(s);
    case '?': s.ov = 'keys'; return ret(s);
    case '1': case '2': case '3': return answer(s, s.sel, +k);
  }
  return null;
}
function sheetKey(s, a) {
  const k = a.key, sh = s.sheet;
  if (sh.k === 'inbox') {
    const items = inboxItems(s), cur = items[s.inbox.row], cf = s.inbox.confirm;
    if (cf) { if (k === 'Escape') { s.inbox.confirm = null; return ret(s); } if (k === 'Enter' && !cf.busy) { cf.busy = 1; return ret(s, [{ after: 1400, act: { t: 'later', fn: st => { const id = st.inbox.confirm.id, kk = st.inbox.confirm.k; delete st.S[id]; st.inbox.confirm = null; st.inbox.row = 0; if (st.sel === id) st.sel = 'orders'; if (st.focus === id) st.focus = 'orders'; return ret(st, say(st, kk === 'merge' ? id + ' merged into main · worktree and branch removed' : id + ' thrown away · worktree removed')); } } }]); } return ret(s); }
    if (s.inbox.q) { if (k === 'Escape') { s.inbox.q = ''; s.inbox.row = 0; return ret(s); } if (k === 'Backspace') { s.inbox.q = s.inbox.q.slice(0, -1); return ret(s); } if (k === 'Enter' && cur) { s.sel = s.focus = cur.id; s.sheet = null; s.inbox.q = ''; return ret(s); } }
    if (k === 'ArrowDown') { s.inbox.row = Math.min(items.length - 1, s.inbox.row + 1); return ret(s); }
    if (k === 'ArrowUp') { s.inbox.row = Math.max(0, s.inbox.row - 1); return ret(s); }
    if (k === 'Escape') { s.sheet = null; return ret(s); }
    if (!cur && !printable(k)) return null;
    if (cur && !s.inbox.q) {
      if (cur.k === 'needs' && '123'.includes(k)) { const r = answer(s, cur.id, +k); return r; }
      if (k === 'm' && cur.k !== 'conf') { s.compose = { id: cur.id, text: '', from: 'inbox' }; return ret(s); }
      if (cur.k === 'conf') { if (k === 'd') { openSheet(s, 'changes', { conf: 1 }); return ret(s); } if (k === 'k') { s.conf[cur.i].on = 0; return ret(s, say(s, 'Dismissed. It comes back if they touch it again.')); } if (k === 'm') { s.compose = { id: 'orders', text: 'rate-limit also changes checkout.ts lines 80–96; rebase on it before you finish', from: 'inbox' }; return ret(s); } }
      if (cur.k === 'done') { if (k === 'd') { openSheet(s, 'changes', { id: cur.id }); return ret(s); } if (k === 'M') { s.inbox.confirm = { id: cur.id, k: 'merge' }; return ret(s); } if (k === 'x') { s.inbox.confirm = { id: cur.id, k: 'discard' }; return ret(s); } if (k === 'O') { const v = s.S[cur.id].srv; return ret(s, say(s, v && v.st === 'ready' ? 'Opened localhost:' + v.port : 'No server running')); } }
      if (k === 'Enter') { const id = cur.id || 'orders'; s.sel = s.focus = id; s.sheet = null; return ret(s); }
      if (k === 'Q') { s.tk.tab = 'queue'; openSheet(s, 'tickets'); return ret(s); }
      if (k === 'j') { s.sheet = null; return ret(s); }
    }
    if (printable(k) && /[a-z]/i.test(k)) { s.inbox.q += k; s.inbox.row = 0; return ret(s); }
    return null;
  }
  if (sh.k === 'tickets') {
    const tk = s.tk, tabs = ['github', 'linear', 'plane', 'queue'];
    if (tk.add != null) { if (k === 'Escape') { tk.add = null; return ret(s); } if (k === 'Enter') { if (tk.add.trim()) s.Q.splice(s.Q.findIndex(q => q.st === 'review'), 0, { t: tk.add.trim(), st: 'waiting' }); tk.add = null; return ret(s, say(s, 'Added to the queue')); } if (k === 'Backspace') { tk.add = tk.add.slice(0, -1); return ret(s); } if (printable(k)) { tk.add += k; return ret(s); } return ret(s); }
    if (k === 'Tab') { tk.tab = tabs[(tabs.indexOf(tk.tab) + 1) % 4]; tk.row = 0; return ret(s); }
    if (k === 'Escape') { if (tk.q) { tk.q = ''; return ret(s); } s.sheet = null; return ret(s); }
    if (tk.tab === 'linear') { if (k === 'Enter' && tk.key) return ret(s, say(s, 'Linear connected · 12 issues')); if (k === 'Backspace') { tk.key = tk.key.slice(0, -1); return ret(s); } if (printable(k)) { tk.key += k; return ret(s); } return ret(s); }
    if (tk.tab === 'plane') { if (k === 'r') return ret(s, say(s, 'Still 401. Paste a new key with k')); return ret(s); }
    const n = tk.tab === 'queue' ? s.Q.length : TICKETS.length;
    if (k === 'ArrowDown') { tk.row = Math.min(n - 1, tk.row + 1); return ret(s); }
    if (k === 'ArrowUp') { tk.row = Math.max(0, tk.row - 1); return ret(s); }
    if (tk.tab === 'queue') {
      const q = s.Q[tk.row];
      if (k === 'a') { tk.add = ''; return ret(s); }
      if (k === 'x' && q) { s.Q.splice(tk.row, 1); tk.row = Math.max(0, tk.row - 1); return ret(s, say(s, 'Removed from the queue')); }
      if (k === 'r' && q && q.st === 'failed') { q.st = 'waiting'; q.why = null; return ret(s, say(s, 'Back in the queue')); }
      if (k === 'Enter' && q && q.st === 'review') { openSheet(s, 'inbox'); return ret(s); }
      return ret(s);
    }
    if (k === 'Enter') return startTicket(s);
    if (k === 'q') { const t = TICKETS[tk.row]; if (t.in) return ret(s, say(s, '#' + t.n + ' is already taken')); t.in = 'queue'; s.Q.splice(3, 0, { t: '#' + t.n + ' ' + t.t, st: 'waiting' }); return ret(s, say(s, 'Queued #' + t.n + ' · starts when a slot frees up')); }
    if (k === 'Backspace') { tk.q = tk.q.slice(0, -1); return ret(s); }
    if (printable(k) && /[a-z ]/i.test(k)) { tk.q += k; tk.row = 0; return ret(s); }
    return ret(s);
  }
  if (sh.k === 'pr') { if (k === 'F') return handoff(s, sh.id || s.sel); if (k === 'S') { s.ov = 'ship'; s.ship = { id: sh.id || s.sel, step: 0, busy: 0 }; return ret(s); } if (k === 'd') { openSheet(s, 'changes', { id: sh.id }); return ret(s); } }
  if (sh.k === 'cp') { if (!s.cp.on && k === 'Enter') return click(s, { t: 'cpOn' }); if (s.cp.on) { if (k === 'ArrowDown') { s.cp.row = Math.min(CPS.length - 1, s.cp.row + 1); return ret(s); } if (k === 'ArrowUp') { s.cp.row = Math.max(0, s.cp.row - 1); return ret(s); } if (k === 'Enter') { s.ov = 'rollback'; return ret(s); } } }
  if (sh.k === 'changes' && sh.conf) { if (k === 'k') { s.conf[0].on = 0; s.sheet = null; return ret(s, say(s, 'Dismissed')); } if (k === 'm') { s.sheet = null; s.compose = { id: 'orders', text: 'rate-limit also changes checkout.ts lines 80–96; rebase on it before you finish', from: 'side' }; return ret(s); } }
  if (sh.k === 'changes' && sh.id && k === 'M') { openSheet(s, 'inbox'); s.inbox.confirm = { id: sh.id, k: 'merge' }; return ret(s); }
  if (k === 'Escape') { s.sheet = null; return ret(s); }
  return null;
}

// ---------- presets ----------
export const PRESETS = [
  ['Splash · Today', s => { s.ov = 'splash'; }],
  ['Main', s => {}],
  ['First run', s => { s.first = 1; s.sel = null; s.focus = null; }],
  ['Inbox', s => { openSheet(s, 'inbox'); }],
  ['Follow-up · sidebar', s => { s.compose = { id: 'orders', text: "yes, but round half-even, not half-up. maya's review on #412 asks for it at checkout.ts:88.\n\nalso keep the tax calculation in its own module so rate-limit can rebase on it without conflicts. when you're done, run the e2e suite and tell me what fails.", from: 'side' }; }],
  ['Follow-up · Inbox', s => { openSheet(s, 'inbox'); s.compose = { id: 'orders', text: "yes, and round half-even. maya asked for it on #412.\nthen run the e2e suite.", from: 'inbox' }; }],
  ['Merge from Inbox', s => { openSheet(s, 'inbox'); s.inbox.row = 2; s.inbox.confirm = { id: 'search', k: 'merge' }; }],
  ['Inbox · find', s => { openSheet(s, 'inbox'); s.inbox.q = 'cl'; }],
  ['Inbox · empty', s => { openSheet(s, 'inbox'); s.S.orders.st = 'work'; s.S.search.st = 'work'; s.S.docs.st = 'idle'; s.conf[0].on = 0; }],
  ['Same-file conflict', s => { openSheet(s, 'changes', { conf: 1 }); }],
  ['Why this status', s => { s.why = 'orders'; }],
  ['Tickets', s => { openSheet(s, 'tickets'); }],
  ['Tickets · loading', s => { openSheet(s, 'tickets'); s.tk.gh = 'loading'; }],
  ['Linear · not set up', s => { openSheet(s, 'tickets'); s.tk.tab = 'linear'; }],
  ['Plane · error', s => { openSheet(s, 'tickets'); s.tk.tab = 'plane'; }],
  ['Queue', s => { openSheet(s, 'tickets'); s.tk.tab = 'queue'; }],
  ['Pull request', s => { openSheet(s, 'pr', { id: 'orders' }); }],
  ['PR · loading', s => { openSheet(s, 'pr', { id: 'orders' }); s.pr = 'loading'; }],
  ['PR · none yet', s => { s.sel = 'search'; openSheet(s, 'pr', { id: 'search' }); }],
  ['Ship confirm', s => { s.ov = 'ship'; s.ship = { id: 'orders', step: 0, busy: 0 }; }],
  ['Dev server', s => { s.sel = 'search'; openSheet(s, 'server', { id: 'search' }); }],
  ['Server crashed', s => { s.sel = 'main'; openSheet(s, 'server', { id: 'main' }); }],
  ['No run command', s => { s.sel = 'docs'; openSheet(s, 'server', { id: 'docs' }); }],
  ['Checkpoints · off', s => { openSheet(s, 'cp', { id: 'orders' }); }],
  ['Checkpoints · list', s => { s.cp.on = 1; openSheet(s, 'cp', { id: 'orders' }); }],
  ['Roll back confirm', s => { s.cp.on = 1; s.cp.row = 2; openSheet(s, 'cp', { id: 'orders' }); s.ov = 'rollback'; }],
  ['Remote window', s => { s.remote = 1; s.sel = 'gpu-bench'; s.focus = 'gpu-bench'; }],
  ['Settings · Sync', s => { s.ov = 'settings'; s.set.tab = 'sync'; }],
  ['Sync · synced', s => { s.ov = 'settings'; s.set.tab = 'sync'; s.set.sync = 'synced'; }],
  ['Sync · error', s => { s.ov = 'settings'; s.set.tab = 'sync'; s.set.sync = 'error'; }],
  ['Settings · Alerts', s => { s.ov = 'settings'; s.set.tab = 'alerts'; s.set.al.on = 1; }],
  ['Leader armed', s => { s.leader = 1; }],
  ['Key map', s => { s.ov = 'keys'; }]];
export function preset(name, W, H, fl) { const s = initialState(); s.W = W || 160; s.H = H || 45; Object.assign(s, fl || {}); const p = PRESETS.find(p => p[0] === name); if (p) p[1](s); return s; }

export function render(s) {
  SOFT = s.soft ? 1 : 0;
  const g = new Grid(s.W, s.H, T); g.fill(0, 0, s.W, s.H, DESK);
  const R = [], reg = (x, y, w, h, act, kind) => R.push({ x, y, w, h, act, kind });
  if (s.ov === 'splash') { splash(g, s, reg); return { rows: g.rows(), regions: R, W: s.W, H: s.H }; }
  const Ly = layout(s);
  const rowY = sidebar(g, s, Ly, reg); tabs(g, s, Ly, reg); panes(g, s, Ly, reg);
  const asModal = s.sheet && s.modalTools && MODAL_KINDS.includes(s.sheet.k);
  if (s.sheet && !asModal) sheet(g, s, Ly, reg);
  if (s.compose && s.compose.from === 'side') composePop(g, s, Ly, rowY, reg);
  if (s.why) whyPop(g, s, Ly, rowY, reg);
  if (asModal && !s.ov) { dimAll(g); R.length = 0; sheet(g, s, Ly, reg, 1); }
  if (s.toast) toast(g, s, Ly);
  if (['ship', 'rollback', 'settings', 'keys'].includes(s.ov)) { dimAll(g); R.length = 0; ({ ship, rollback, settings, keys: keymap })[s.ov](g, s, reg); }
  return { rows: g.rows(), regions: R, W: s.W, H: s.H };
}
