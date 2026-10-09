import { Grid, termLines, wrapSegs, trunc, segLen, L, mix } from './seshi-grid.js';

export const T = { name: 'dark', bg: '#070b10', fg: '#c9d1d9', surf: '#0c131b', card: '#0f1821', card2: '#18242f', line: '#1f2c3a', dim: '#71808f', text: '#a7b4c2', strong: '#f2f6f8',
  acc: '#c3f53c', accInk: '#0a1204', needs: '#ffb547', work: '#a7b4c2', ready: '#4fd8ec', shipped: '#5b6876', ok: '#7fd962', err: '#ff6b6b', scrim: '#000000', btn: '#1d2a37', hov: '#2a3a4c',
  ws: { violet: '#a593ff', sky: '#5aa9ff', pink: '#ff7ab6', teal: '#3dd6c0' },
  a: { red: '#ff6b6b', green: '#7fd962', yellow: '#e8c565', blue: '#5aa9ff', magenta: '#a593ff', cyan: '#3dd6c0', gray: '#71808f' } };
const G = { needs: '●', work: ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'], ready: '◆', shipped: '✓' };
const tint = (c, k) => mix(T.bg, c, k);
const BTN = (label, key, st) => { const P = st === 'primary' ? [T.acc, T.accInk, T.accInk] : st === 'ghost' ? [T.card, T.text, T.acc] : [T.btn, T.strong, T.acc]; return [{ t: ' ' + label + ' ', bg: P[0], fg: P[1], b: st !== 'ghost' }, ...(key ? [{ t: key + ' ', bg: P[0], fg: P[2], b: 1 }] : [])]; };
const BTNS = (list, st) => list.flatMap((b, i) => i ? [{ t: ' ' }, ...BTN(b[0], b[1], b[2] || st)] : BTN(b[0], b[1], b[2] || st));

const PROJ = () => [
  { name: 'All projects', sub: '9 tasks', needs: 2 },
  { name: 'shop-api', c: T.ws.violet, sub: [['ready', 1], ['work', 2], ['shipped', 2]], needs: 1, cur: 1 },
  { name: 'web-shop', c: T.ws.sky, sub: [['work', 1], ['shipped', 1]], needs: 1 },
  { name: 'docs', c: T.ws.pink, sub: [['ready', 1]] },
  { name: 'drover-cli', c: T.ws.teal, sub: 'no tasks yet' }
];
const TASKS = () => [
  { st: 'needs', title: 'fix the flaky checkout test', agent: 'claude', age: '3m', q: 'Allow running "npm test -- checkout"?' },
  { st: 'ready', title: 'add rate limiting to /login', agent: 'claude', age: '12m', plus: '+42', minus: '−7', files: '3 files' },
  { st: 'work', title: 'fix the failing checks on #412', agent: 'codex', age: '2m', det: 'running npm test', p: 6, f: 2 },
  { st: 'work', title: 'migration for orders.status', agent: 'gemini', age: '1m', det: 'reading db/schema.sql', p: 2, f: 5 },
  { st: 'shipped', title: 'bump express to 4.19.2', agent: 'codex', age: '1h', det: 'merged into main' },
  { st: 'shipped', title: 'tidy logger output', agent: 'claude', age: '3h', det: 'committed' }
];
const STAGES = [['needs', 'NEEDS YOU'], ['ready', 'READY'], ['work', 'WORKING'], ['shipped', 'SHIPPED']];
const col = st => T[st];
const glyph = t => t.st === 'work' ? G.work[(t.f || 0) % G.work.length] : G[t.st];

function rbox(g, x, y, w, h, fg, bg, dashed) {
  if (bg) g.fill(x + 1, y + 1, w - 2, h - 2, bg);
  const hz = dashed ? '╌' : '─', vt = dashed ? '╎' : '│', s = { fg };
  g.set(x, y, '╭', s); g.set(x + w - 1, y, '╮', s); g.set(x, y + h - 1, '╰', s); g.set(x + w - 1, y + h - 1, '╯', s);
  for (let i = 1; i < w - 1; i++) { g.set(x + i, y, hz, s); g.set(x + i, y + h - 1, hz, s); }
  for (let j = 1; j < h - 1; j++) { g.set(x, y + j, vt, s); g.set(x + w - 1, y + j, vt, s); }
}

function topBar(g, W) {
  g.fill(0, 0, W, 1, T.bg);
  let x = g.put(1, 0, '>_ drover', { fg: T.acc, b: 1 }) + 3;
  [['Tasks', 't', 1], ['Inbox 5', 'i'], ['Files', 'f'], ['Toolbox', 'b']].forEach(([n, k, on]) => {
    x = g.putSegs(x, 0, [{ t: ' ' + n + ' ', bg: on ? T.acc : T.surf, fg: on ? T.accInk : T.text, b: !!on }, { t: k + ' ', bg: on ? T.acc : T.surf, fg: on ? T.accInk : T.dim, b: 1 }]) + 1;
  });
  const s = [{ t: ' Settings ', bg: T.surf, fg: T.text }, { t: ', ', bg: T.surf, fg: T.dim, b: 1 }];
  g.putSegs(W - segLen(s), 0, s);
}

function sidebar(g, SW, H, compact) {
  g.fill(0, 1, SW, H - 2, T.surf);
  g.put(2, 2, 'PROJECTS', { fg: T.dim, b: 1 });
  let y = 4;
  PROJ().forEach((p, i) => {
    const rows = compact ? 1 : 2, sel = !!p.cur, ink = sel ? T.accInk : null;
    if (sel) g.fill(0, y, SW, rows, T.acc);
    if (p.c) g.put(1, y, '▌', { fg: ink || p.c, bg: sel ? T.acc : undefined });
    g.put(2, y, trunc(p.name, SW - 7), { fg: ink || (i ? T.strong : T.text), b: sel || !i, bg: sel ? T.acc : undefined });
    if (p.needs) { const n = G.needs + p.needs; g.put(SW - 1 - L(n), y, n, { fg: ink || T.needs, b: 1, bg: sel ? T.acc : undefined }); }
    if (!compact) {
      const segs = typeof p.sub === 'string' ? [{ t: p.sub, fg: ink || T.dim }] : p.sub.flatMap(([st, n]) => [{ t: (st === 'work' ? G.work[2] : G[st]) + n + '  ', fg: ink || (st === 'shipped' ? T.dim : col(st)) }]);
      g.putSegs(2, y + 1, segs, { bg: sel ? T.acc : undefined });
    }
    y += rows + (compact ? 0 : 1);
    if (!i) { g.hl(1, y, SW - 2, '─', { fg: T.line }); y += compact ? 1 : 2; }
  });
  g.putSegs(2, H - 3, BTN('+ Project', 'p', 'ghost'));
}

function card(g, x, y, w, t, o = {}) {
  const sel = !!o.sel, small = !!o.small;
  const h = small ? 4 : t.st === 'needs' && sel ? 8 : 5;
  if (o.lifted) { rbox(g, x, y, w, h, T.accInk, T.acc); } else rbox(g, x, y, w, h, sel ? T.acc : T.line, T.card);
  const bg = o.lifted ? T.acc : T.card, ink = o.lifted ? T.accInk : null, ix = x + 2, iw = w - 4;
  g.put(x + w - 3, y, '⠿', { fg: ink || (sel ? T.acc : T.dim) });
  g.putSegs(ix, y + 1, [{ t: glyph(t) + ' ', fg: ink || col(t.st), b: t.st === 'needs' }, { t: trunc(t.title, iw - 2), fg: ink || (t.st === 'shipped' ? T.text : T.strong), b: t.st !== 'shipped' }], { bg });
  if (small) { g.putSegs(ix + 2, y + 2, [{ t: trunc(t.st === 'needs' ? 'answer below' : t.st === 'ready' ? t.plus + ' ' + t.minus + ' · checks ✓' : t.st === 'work' ? t.det : t.det, iw - 2), fg: ink || (t.st === 'needs' ? T.needs : T.dim) }], { bg }); return h; }
  g.putSegs(ix + 2, y + 2, [{ t: t.agent, fg: ink || T.text }, { t: ' · ' + t.age, fg: ink || T.dim }], { bg });
  if (t.st === 'needs') g.put(ix + 2, y + 3, trunc(t.q, iw - 2), { fg: ink || T.needs, bg });
  if (t.st === 'ready') { g.putSegs(ix + 2, y + 3, [{ t: t.plus, fg: ink || T.a.green }, { t: ' ' + t.minus, fg: ink || T.a.red }, ...(w >= 40 ? [{ t: ' · ' + t.files, fg: ink || T.dim }] : [])], { bg }); const c = G.shipped + ' checks'; g.put(x + w - 2 - L(c), y + 3, c, { fg: ink || T.ok, bg }); }
  if (t.st === 'work') g.putSegs(ix + 2, y + 3, [{ t: '█'.repeat(t.p), fg: ink || T.acc }, { t: '░'.repeat(10 - t.p), fg: ink || T.line }, { t: '  ' + trunc(t.det, iw - 14), fg: ink || T.dim }], { bg });
  if (t.st === 'shipped') g.put(ix + 2, y + 3, trunc(t.det, iw - 2), { fg: ink || T.dim, bg });
  if (t.st === 'needs' && sel) {
    g.putSegs(ix, y + 5, BTNS([['Yes', '1', 'primary'], ['Always', '2'], ['No', '3']]), { }, x + w - 1);
    g.putSegs(ix, y + 6, [{ t: 'r', fg: T.acc, b: 1, bg }, { t: ' reply   ', fg: T.dim, bg }, { t: 'z', fg: T.acc, b: 1, bg }, { t: ' full screen', fg: T.dim, bg }], {}, x + w - 1);
  }
  return h;
}

function home(W, H, o = {}) {
  const g = new Grid(W, H, T), tasks = TASKS(), compact = W < 140;
  topBar(g, W);
  const SW = W >= 200 ? 34 : compact ? 20 : 28;
  sidebar(g, SW, H, compact);
  const mx = SW + 2, mw = W - mx - 1;
  g.putSegs(mx, 2, [{ t: '▌', fg: T.ws.violet }, { t: 'shop-api', fg: T.strong, b: 1 }, { t: compact ? '' : '   ~\\code\\shop-api · main', fg: T.dim }]);
  const nb = BTN('+ New task', 'n', 'primary'); g.putSegs(W - 1 - segLen(nb), 2, nb);
  const gap = compact ? 1 : 2, cw = Math.floor((mw - gap * 3) / 4), hy = compact ? 3 : 4, cy0 = hy + 2;
  let bottom = cy0;
  STAGES.forEach(([st, label], ci) => {
    const cx = mx + ci * (cw + gap), list = tasks.filter(t => t.st === st);
    g.fill(cx, hy, cw, 1, tint(col(st), 0.2));
    g.putSegs(cx + 1, hy, [{ t: label, fg: st === 'shipped' ? T.text : col(st), b: 1 }]);
    g.put(cx + cw - 1 - String(list.length).length, hy, String(list.length), { fg: T.text, bg: tint(col(st), 0.2) });
    let y = cy0;
    list.forEach((t, i) => {
      const sel = st === 'needs' && i === 0 && !o.drag;
      if (o.drag && t.st === 'ready') { rbox(g, cx, y, cw, compact ? 4 : 5, T.line, null, true); y += (compact ? 4 : 5) + 1; return; }
      y += card(g, cx, y, cw, t, { sel, small: compact }) + 1;
    });
    if (o.drag && st === 'shipped') { rbox(g, cx, y, cw, compact ? 4 : 5, T.acc, null, true); g.put(cx + 2, y + 2, 'drop to ship', { fg: T.acc, b: 1 }); o.dropY = y; o.dropX = cx; y += 6; }
    bottom = Math.max(bottom, y);
  });
  const ty = Math.max(bottom, compact ? 15 : 18), th = H - 1 - ty;
  rbox(g, mx, ty, mw, th, T.acc, T.bg);
  const tab = [{ t: ' claude ', bg: T.acc, fg: T.accInk, b: 1 }, { t: ' fix the flaky checkout test ', fg: T.strong, b: 1 }, { t: 'live ', fg: T.dim }];
  g.putSegs(mx + 2, ty, tab);
  if (!compact) { const r = ' drag the top edge to resize '; g.put(mx + mw - 2 - L(r), ty, r, { fg: T.dim }); }
  const ask = compact && !o.drag, bot = ty + th - 1 - (ask ? 1 : 0);
  const lines = wrapSegs(termLines(T), mw - 5), room = bot - ty - 1, shown = lines.slice(Math.max(0, lines.length - room));
  shown.forEach((l, i) => g.putSegs(mx + 2, bot - shown.length + i, l, {}, mx + mw - 2));
  if (ask) { g.fill(mx + 1, bot, mw - 2, 1, T.card2); g.putSegs(mx + 2, bot, [{ t: G.needs + ' waiting  ', fg: T.needs, b: 1, bg: T.card2 }, ...BTNS([['Yes', '1', 'primary'], ['Always', '2'], ['No', '3'], ['Reply', 'r']])], {}, mx + mw - 2); }
  g.fill(0, H - 1, W, 1, T.surf);
  if (o.hint) g.putSegs(1, H - 1, o.hint, { bg: T.surf });
  else g.putSegs(1, H - 1, [{ t: G.needs + ' 2 need you', fg: T.needs, b: 1 }, { t: '   ' }, { t: G.ready + ' 2 ready', fg: T.ready }, { t: '   ' }, { t: G.work[2] + ' 4 working', fg: T.work }, { t: '   across 3 projects', fg: T.dim }], { bg: T.surf });
  g.put(W - 12, H - 1, ' Ctrl+Space ', { fg: T.accInk, bg: T.acc, b: 1 });
  if (o.drag && o.dropX != null) {
    const t = tasks[1], cx = o.dropX + 1, cy = o.dropY + 1;
    g.fill(o.dropX + 1, o.dropY + 1, cw - 2, 3, T.bg);
    card(g, cx, cy, cw, t, { lifted: 1 });
    g.set(cx + 6, cy + 2, '↖', { fg: T.bg, bg: T.strong, b: 1 });
  }
  return g;
}

export function build() {
  const R = g => g.rows();
  return [
    { id: '1a', title: 'Home · 160×45', note: 'Pick a project on the left; its tasks sit in stage columns; the selected card\u2019s terminal is always below, framed in the same lime.', rows: R(home(160, 45)) },
    { id: '1b', title: 'Home · 100×30', note: 'Sidebar shrinks to names + needs count. Cards drop to two lines; answer buttons move into the terminal frame.', rows: R(home(100, 30)) },
    { id: '1c', title: 'Home · 240×65', note: 'Wide: roomier cards and a bigger terminal. Same structure.', rows: R(home(240, 65)) },
    { id: '1d', title: 'Drag a card to ship it · 160×45', note: 'Dragging a ready card into Shipped opens the ship choice (commit, merge, open PR). Every drag has a key too.', rows: R(home(160, 45, { drag: 1, hint: [{ t: 'dragging ', fg: T.text }, { t: 'add rate limiting to /login', fg: T.strong, b: 1 }, { t: ' · drop in Shipped to commit, merge or open a PR · Esc cancels', fg: T.text }] })) }
  ];
}
