const L = s => Array.from(s).length;
const segLen = segs => segs.reduce((n, s) => n + L(s.t), 0);
const trunc = (s, n) => { const a = Array.from(s); return a.length <= n ? s : a.slice(0, Math.max(0, n - 1)).join('') + '…'; };
const wrap = (s, n, max = 99) => { const out = []; let cur = ''; for (const w of s.split(' ')) { if (!cur) cur = w; else if (L(cur) + 1 + L(w) <= n) cur += ' ' + w; else { out.push(cur); cur = w; } } if (cur) out.push(cur); if (out.length > max) { out.length = max; out[max - 1] = trunc(out[max - 1] + '……', n); } return out.map(l => trunc(l, n)); };
const hex = h => [1, 3, 5].map(i => parseInt(h.slice(i, i + 2), 16));
const mix = (a, b, t) => { const A = hex(a), B = hex(b); return '#' + A.map((v, i) => Math.round(v + (B[i] - v) * t).toString(16).padStart(2, '0')).join(''); };

export const THEMES = {
  dark: { name: 'dark', bg: '#0a0e13', fg: '#c9d1d9', surf: '#10161d', surf2: '#1a232e', btn: '#263241', hov: '#34465c', selTxt: '#2f4a66', pop: '#0e141b', sel: '#17212c', line: '#223040', dim: '#738394', text: '#a7b4c2', strong: '#eef3f7',
    acc: '#c3f53c', accFill: '#c3f53c', accInk: '#0a0e13', scrim: '#000000',
    needs: '#ffb547', work: '#a7b4c2', ready: '#5fd4e8', shipped: '#738394', ok: '#7fd962', err: '#ff6b6b',
    ws: { violet: '#a593ff', sky: '#5aa9ff', pink: '#ff7ab6', teal: '#3dd6c0', orange: '#ff9e64', slate: '#9fb3c8' },
    a: { red: '#ff6b6b', green: '#7fd962', yellow: '#e8c565', blue: '#5aa9ff', magenta: '#a593ff', cyan: '#3dd6c0', gray: '#738394' } },
  light: { name: 'light', bg: '#f6f8f3', fg: '#1d242c', surf: '#eaeee5', surf2: '#dbe2d4', btn: '#d3dac8', hov: '#bfc9b2', selTxt: '#b9d3f0', pop: '#ffffff', sel: '#e3eadb', line: '#c6cfbf', dim: '#5c6875', text: '#3c4652', strong: '#0c1116',
    acc: '#4a7a00', accFill: '#c3f53c', accInk: '#0a0e13', scrim: '#5a6050',
    needs: '#9a5800', work: '#3c4652', ready: '#00788a', shipped: '#5c6875', ok: '#3d7d1c', err: '#c0392b',
    ws: { violet: '#6a4fd6', sky: '#1f6fd1', pink: '#c2387a', teal: '#0f8a7a', orange: '#c25a18', slate: '#4c5f78' },
    a: { red: '#c0392b', green: '#3d7d1c', yellow: '#8a6d00', blue: '#1f6fd1', magenta: '#6a4fd6', cyan: '#0f8a7a', gray: '#5c6875' } }
};

export const GLYPHS = {
  unicode: { needs: '●', work: ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'], ready: '◆', shipped: '✓', ok: '✓', fail: '✕', back: '‹', more: '▸', on: '(•)', off: '( )' },
  ascii: { needs: '!', work: ['|', '/', '-', '\\'], ready: '*', shipped: '+', ok: '+', fail: 'x', back: '<', more: '>', on: '(*)', off: '( )' }
};

class Grid {
  constructor(w, h, T) { this.w = w; this.h = h; this.T = T; this.c = []; for (let y = 0; y < h; y++) { const r = []; for (let x = 0; x < w; x++) r.push({ ch: ' ', fg: T.fg, bg: T.bg, b: 0, i: 0, u: 0 }); this.c.push(r); } }
  set(x, y, ch, s = {}) { if (x < 0 || y < 0 || x >= this.w || y >= this.h) return; const c = this.c[y][x]; c.ch = ch; c.fg = s.fg || this.T.fg; if (s.bg) c.bg = s.bg; c.b = s.b ? 1 : 0; c.i = s.i ? 1 : 0; c.u = s.u ? 1 : 0; }
  put(x, y, str, s = {}) { for (const ch of Array.from(str)) this.set(x++, y, ch, s); return x; }
  putSegs(x, y, segs, base = {}, maxX) { for (const sg of segs) for (const ch of Array.from(sg.t)) { if (maxX != null && x >= maxX) return x; this.set(x++, y, ch, { ...base, ...sg }); } return x; }
  fill(x, y, w, h, bg) { for (let j = y; j < y + h; j++) for (let i = x; i < x + w; i++) this.set(i, j, ' ', { bg }); }
  hl(x, y, w, ch, s) { for (let i = 0; i < w; i++) this.set(x + i, y, ch, s); }
  vl(x, y, h, ch, s) { for (let j = 0; j < h; j++) this.set(x, y + j, ch, s); }
  box(x, y, w, h, o = {}) {
    const S = ['┌', '┐', '└', '┘', '─', '│'], s = { fg: o.fg }; if (o.bg) this.fill(x, y, w, h, o.bg);
    this.set(x, y, S[0], s); this.set(x + w - 1, y, S[1], s); this.set(x, y + h - 1, S[2], s); this.set(x + w - 1, y + h - 1, S[3], s);
    for (let i = 1; i < w - 1; i++) { this.set(x + i, y, S[4], s); this.set(x + i, y + h - 1, S[4], s); }
    for (let j = 1; j < h - 1; j++) { this.set(x, y + j, S[5], s); this.set(x + w - 1, y + j, S[5], s); }
    if (o.title) this.putSegs(x + 1, y, o.title, {}, x + w - 2);
    if (o.right) this.putSegs(x + w - 2 - segLen(o.right), y, o.right);
  }
  dimAll() { const T = this.T, k = T.name === 'dark' ? 0.45 : 0.14; for (const r of this.c) for (const c of r) { c.fg = mix(mix(c.fg, c.bg, 0.6), T.scrim, k); c.bg = mix(c.bg, T.scrim, k); } }
  rows() {
    const out = [];
    for (let y = 0; y < this.h; y++) {
      const runs = []; let cur = null;
      for (let x = 0; x < this.w; x++) {
        const c = this.c[y][x], cp = c.ch.codePointAt(0);
        const fx = cp > 126 ? (cp >= 0x2500 && cp <= 0x259f ? 1 : 2) : 0;
        const k = c.fg + c.bg + c.b + c.i + c.u + fx;
        if (fx !== 2 && cur && cur.k === k) { cur.t += c.ch; cur.n++; continue; }
        cur = { k, n: 1, fx, t: c.ch, fg: c.fg, bg: c.bg, fw: c.b ? '700' : '400', fs: c.i ? 'italic' : 'normal', td: c.u ? 'underline' : 'none' };
        runs.push(cur);
      }
      out.push(runs.map(({ k, n, fx, ...r }) => { const pua = fx === 2 && r.t.codePointAt(0) >= 0xe000 && r.t.codePointAt(0) <= 0xf8ff; return { ...r, wd: pua ? '7.2px' : fx === 2 ? '1ch' : 'auto', fz: pua ? '16.5px' : '12px', ov: pua ? 'visible' : 'hidden', ta: pua ? (r.t === '\ue0b6' ? 'right' : r.t === '\ue0b4' ? 'left' : 'center') : 'center', lh: pua ? '16px' : '16px' }; }));
    }
    return out;
  }
}

const KY = (T, k) => ({ t: k, fg: T.acc, b: 1 });
const HK = (T, k, l, fg) => [KY(T, k), { t: ' ' + l, fg: fg || T.text }];
const gap = (n = 3) => ({ t: ' '.repeat(n) });
const joinH = (...groups) => groups.flatMap((g, i) => i ? [gap(), ...g] : g);
const BTN = (T, label, key, st = 'normal') => {
  const P = { normal: [T.btn, T.strong, T.acc], hover: [T.hov, T.strong, T.acc], pressed: [T.strong, T.bg, T.bg], primary: [T.accFill, T.accInk, T.accInk], danger: [T.btn, T.err, T.acc], ghost: [T.bg, T.text, T.acc], off: [T.surf, T.dim, T.dim] }[st];
  return [{ t: ' ' + label + ' ', bg: P[0], fg: P[1], b: st !== 'off' && st !== 'ghost' }, ...(key ? [{ t: key + ' ', bg: P[0], fg: P[2], b: 1 }] : [])];
};
const BTNS = (T, list, sep = 1, st) => list.flatMap((b, i) => i ? [gap(sep), ...BTN(T, b[0], b[1], b[2] || st)] : BTN(T, b[0], b[1], b[2] || st));
const pointer = (g, x, y) => g.set(x, y, '↖', { fg: g.T.bg, bg: g.T.strong, b: 1 });
const ST = (T, G) => ({ needs: { g: G.needs, l: 'needs you', c: T.needs }, work: { g: G.work[2], l: 'working', c: T.work }, ready: { g: G.ready, l: 'ready', c: T.ready }, shipped: { g: G.shipped, l: 'shipped', c: T.shipped } });
const stG = (G, t) => t.st === 'work' ? G.work[(t.f || 0) % G.work.length] : ({ needs: G.needs, ready: G.ready, shipped: G.shipped })[t.st];

const tasksData = (T) => { const a = T.a; return [
  { st: 'needs', title: 'fix the flaky checkout test', proj: 'shop-api', pc: T.ws.violet, agent: 'claude', age: '3m', branch: 'task/fix-flaky-checkout', q: 'Allow running "npm test -- checkout"?', opts: ['yes', 'yes, always for npm test', 'no'],
    tail: [[{ t: '• ', fg: a.green }, { t: 'Read ' }, { t: 'test/checkout.spec.ts', fg: a.cyan }], [{ t: '• ', fg: a.green }, { t: 'The test sleeps 500 ms for the cart and races the API mock.' }], [{ t: '• ', fg: a.green }, { t: "Plan: await the mock's ready promise instead of sleeping." }], [], [{ t: 'Edit ', b: 1 }, { t: 'test/checkout.spec.ts', fg: a.cyan }], [{ t: '  - ', fg: a.red }, { t: 'await page.waitForTimeout(500);', fg: a.red }], [{ t: '  + ', fg: a.green }, { t: 'await mocks.cart.ready;', fg: a.green }], [], [{ t: 'Now I want to run the checkout tests to confirm.' }]] },
  { st: 'needs', title: 'ABC-123 checkout fails on Safari', proj: 'web-shop', pc: T.ws.sky, agent: 'codex', age: '40s', branch: 'task/abc-123', q: 'Use the Playwright WebKit build, or the real Safari driver?', opts: ['WebKit build', 'Safari driver'] },
  { st: 'ready', title: 'add rate limiting to /login', proj: 'shop-api', pc: T.ws.violet, agent: 'claude', age: '12m', branch: 'task/rate-limit-login', det: '+42 −7 · 3 files · checks pass' },
  { st: 'ready', title: 'update README examples', proj: 'docs', pc: T.ws.pink, agent: 'claude', age: '25m', branch: 'task/readme-examples', det: '+18 −30 · 2 files · checks pass' },
  { st: 'work', title: 'fix the failing checks on #412', proj: 'shop-api', pc: T.ws.violet, agent: 'codex', age: '2m', f: 2, det: 'running npm test' },
  { st: 'work', title: 'refactor nav component', proj: 'web-shop', pc: T.ws.sky, agent: 'claude', age: '6m', f: 5, det: 'editing src/nav/Nav.tsx' },
  { st: 'work', title: 'migration for orders.status', proj: 'shop-api', pc: T.ws.violet, agent: 'gemini', age: '1m', f: 8, det: 'reading db/schema.sql' },
  { st: 'shipped', title: 'bump express to 4.19.2', proj: 'shop-api', pc: T.ws.violet, agent: 'codex', age: '1h', det: 'merged into main' },
  { st: 'shipped', title: 'nav a11y fixes', proj: 'web-shop', pc: T.ws.sky, agent: 'claude', age: '2h', det: 'PR #409 opened' },
  { st: 'shipped', title: 'tidy logger output', proj: 'shop-api', pc: T.ws.violet, agent: 'claude', age: '3h', det: 'committed' }
]; };

function topBar(g, c, cur, right, hov) {
  const { T, W } = c;
  let x = g.put(0, 0, ' >_ drover ', { fg: T.accInk, bg: T.accFill, b: 1 }) + 2;
  const tab = (id, label, key, badge) => { const on = id === cur, h = hov === id, bg = on ? T.btn : h ? T.hov : T.surf; return [{ t: ' ' + label + (badge ? ' ' + badge : '') + ' ', bg, fg: on || h ? T.strong : T.text, b: on }, { t: key + ' ', bg, fg: on || h ? T.acc : T.dim, b: 1 }]; };
  for (const [id, label, key, badge] of [['home', 'Tasks', 't'], ['inbox', 'Inbox', 'i', '5'], ['files', 'Files', 'f'], ['toolbox', 'Toolbox', 'b']]) x = g.putSegs(x, 0, tab(id, label, key, badge)) + 1;
  const set = tab('settings', 'Settings', ','), sx = W - segLen(set);
  g.putSegs(sx, 0, set);
  if (right) g.putSegs(sx - 2 - segLen(right), 0, right);
}

function crumbBar(g, c, t, right) {
  const { T, Gl, W } = c, S = ST(T, Gl)[t.st];
  g.putSegs(0, 0, [{ t: ' ' + Gl.back + ' Tasks ', fg: T.accInk, bg: T.accFill, b: 1 }, gap(2), { t: t.title, fg: T.strong, b: 1 }, gap(), { t: S.g + ' ' + S.l, fg: S.c, b: t.st === 'needs' }, gap(), { t: t.proj, fg: t.pc }, { t: '  ' + t.branch, fg: T.dim }], {}, W - 2 - (right ? segLen(right) : 0));
  if (right) g.putSegs(W - 1 - segLen(right), 0, right);
}

function counts(c, tasks) {
  const { T, Gl } = c, S = ST(T, Gl), n = st => tasks.filter(t => t.st === st).length;
  return [{ t: ' ' + S.needs.g + ' ' + n('needs') + ' need you', fg: T.needs, b: 1 }, gap(), { t: S.ready.g + ' ' + n('ready') + ' ready', fg: T.ready }, gap(), { t: S.work.g + ' ' + n('work') + ' working', fg: T.work }];
}

function statusBar(g, c, o = {}) {
  const { T, W } = c, y = g.h - 1; g.fill(0, y, W, 1, T.surf);
  const right = [...(o.hint ? [] : (o.right || [])), gap(), { t: ' Ctrl+Space ', fg: T.accInk, bg: T.accFill, b: 1 }];
  if (o.hint) g.putSegs(0, y, [{ t: ' ' }, ...o.hint], {}, W - segLen(right) - 2); else if (o.left) g.putSegs(0, y, o.left);
  g.putSegs(W - segLen(right), y, right);
}

function inlineLines(c, t) {
  const { T } = c;
  if (t.st === 'needs') return [
    [{ t: t.q, fg: T.strong, b: 1 }],
    [...t.opts.flatMap((o, i) => [KY(T, String(i + 1)), { t: ' ' + o + '   ', fg: T.strong }]), ...HK(T, 'r', 'reply…', T.dim), gap(), ...HK(T, 'Enter', 'open', T.dim)]];
  if (t.st === 'ready') return [[{ t: t.det, fg: T.dim }, gap(), ...HK(T, 'Enter', 'review', T.dim)]];
  if (t.st === 'work') return [[{ t: t.det, fg: T.dim }, gap(), ...HK(T, 'Enter', 'open terminal', T.dim)]];
  return [];
}

function taskList(g, c, x, y, w, h, tasks, o = {}) {
  const { T, Gl } = c, S = ST(T, Gl), sel = o.sel ?? 0, meta = w >= 100;
  let yy = y, idx = 0;
  for (const [st, label] of [['needs', 'NEEDS YOU'], ['ready', 'READY FOR REVIEW'], ['work', 'WORKING'], ['shipped', 'SHIPPED TODAY']]) {
    const list = tasks.filter(t => t.st === st); if (!list.length || yy >= y + h - 1) continue;
    const collapsed = st === 'shipped' && o.collapseShipped;
    g.putSegs(x + 2, yy, [{ t: label, fg: st === 'needs' ? T.needs : T.dim, b: 1 }, { t: '  ' + list.length, fg: T.dim }, ...(collapsed ? [{ t: '  ' + Gl.more, fg: T.dim }] : [])]); yy++;
    if (collapsed) { yy++; continue; }
    for (const t of list) {
      if (yy >= y + h) break;
      const isSel = idx === sel && !o.noSel;
      const right = [{ t: t.proj.padEnd(10), fg: t.pc }, { t: t.agent.padEnd(7), fg: T.dim }, { t: t.age.padStart(4), fg: T.dim }], rw = segLen(right);
      const tw = w - 5 - rw;
      const mid = [{ t: stG(Gl, t) + ' ', fg: S[st].c, b: st === 'needs' }, { t: trunc(t.title, tw - 2), fg: st === 'shipped' ? T.text : T.strong, b: st !== 'shipped' && (isSel || st === 'needs') }];
      const det = st === 'needs' ? t.q : t.det;
      if (meta && det && !(isSel && (o.inline || o.liveSel))) { const room = tw - segLen(mid) - 3; if (room > 10) mid.push({ t: '   ' + trunc(det, room), fg: T.dim }); }
      if (isSel) { g.fill(x, yy, w, 1, T.sel); g.put(x, yy, '>', { fg: T.acc, b: 1 }); } else if (o.hov === idx) g.fill(x, yy, w, 1, T.hov);
      (o.ys || (o.ys = []))[idx] = yy;
      g.putSegs(x + 2, yy, mid, {}, x + w - rw - 2); g.putSegs(x + w - 1 - rw, yy, right); yy++;
      if (isSel && o.inline) for (const l of inlineLines(c, t)) { if (yy >= y + h) break; g.fill(x, yy, w, 1, T.sel); g.putSegs(x + 4, yy, l, {}, x + w - 1); yy++; }
      idx++;
    }
    yy++;
  }
}

function wrapSegs(lines, w) {
  const out = [];
  for (const l of lines) {
    if (segLen(l) <= w) { out.push(l); continue; }
    let cur = [], n = 0;
    for (const sg of l) for (const ch of Array.from(sg.t)) {
      if (n >= w) { out.push(cur); cur = [{ t: '  ' }]; n = 2; }
      const last = cur[cur.length - 1];
      if (last && last.src === sg) last.t += ch; else cur.push({ ...sg, t: ch, src: sg });
      n++;
    }
    out.push(cur);
  }
  return out;
}

function livePanel(g, c, x, y, w, h, t, o = {}) {
  const { T, Gl } = c, S = ST(T, Gl)[t.st];
  if (o.vertical) g.vl(x, y, h, o.divider ? '┃' : '│', { fg: o.divider ? T.acc : T.line });
  const X = o.vertical ? x + 2 : x, R = x + w - 1;
  const hy = o.vertical ? y + 1 : y;
  if (!o.vertical) g.hl(x, hy, w, '─', { fg: T.line });
  const head = [{ t: ' ' + S.g + ' ', fg: S.c, b: 1 }, { t: t.title + ' ', fg: T.strong, b: 1 }];
  const right = [{ t: ' live terminal · ' + t.agent + ' ', fg: T.dim }];
  g.putSegs(X + 1, hy, head, {}, R - segLen(right) - 1);
  g.putSegs(R - segLen(right), hy, right);
  if (o.vertical) g.hl(x + 1, hy + 1, w - 1, '─', { fg: o.drop ? T.acc : T.line });
  const ask = t.st === 'needs' && !o.drop;
  const top = hy + 2, bot = y + h - (ask || o.drop ? 2 : 0), tw = R - (X + 1);
  const lines = wrapSegs(termLines(T), tw), room = bot - top;
  const shown = lines.slice(Math.max(0, lines.length - room)), start = bot - shown.length;
  shown.forEach((l, i) => g.putSegs(X + 1, start + i, l, {}, R));
  for (let i = 0; i < 4; i++) g.set(R, bot - 4 + i, '┃', { fg: T.dim });
  const bx = o.vertical ? x + 1 : x, bw = o.vertical ? w - 1 : w;
  if (ask) {
    g.fill(bx, bot, bw, 2, T.sel);
    g.putSegs(X + 1, bot, [{ t: Gl.needs + ' ' + t.agent + ' is waiting  ', fg: T.needs, b: 1 }, ...BTNS(T, [['Yes', '1'], ['Always', '2'], ['No', '3']])], {}, R);
    g.putSegs(X + 1, bot + 1, [{ t: ' '.repeat(L(Gl.needs + ' ' + t.agent + ' is waiting  ')) }, ...BTNS(T, [['Reply…', 'r'], ['Type here', 'Enter'], ['Full screen', 'z']], 1, 'ghost')], {}, R);
  }
  if (o.drop) {
    g.fill(bx, bot, bw, 2, T.accFill);
    g.putSegs(X + 1, bot, [{ t: 'Drop to insert the path into the ' + t.agent + ' prompt', fg: T.accInk, b: 1 }], {}, R);
    g.putSegs(X + 1, bot + 1, [{ t: '~\\Desktop\\checkout bug.png', fg: T.accInk }], {}, R);
  }
}

function home(T, G, W, H, o = {}) {
  const g = new Grid(W, H, T), c = { T, Gl: G, W }, tasks = tasksData(T);
  topBar(g, c, 'home', [{ t: ' all projects ▾ ', bg: T.btn, fg: T.text }], o.navHov);
  const lo = { collapseShipped: W < 200, liveSel: 1, hov: o.hov };
  let LW = 0;
  if (W < 140) { const LH = 13; lo.collapseShipped = 1; taskList(g, c, 0, 2, W, LH, tasks, lo); livePanel(g, c, 0, 2 + LH, W, H - 3 - LH, tasks[0], { drop: o.drop }); }
  else { LW = o.LW || (W >= 200 ? Math.floor(W * 0.42) : Math.floor(W * 0.48)); if (o.drawer) filesDrawer(g, c, 0, 1, LW, H - 2); else taskList(g, c, 0, 2, LW, H - 3, tasks, lo); livePanel(g, c, LW, 1, W - LW, H - 2, tasks[0], { vertical: 1, drop: o.drop, divider: o.divider }); }
  statusBar(g, c, { left: counts(c, tasks), hint: o.hint, right: BTN(T, '+ New task', 'n') });
  if (o.after) o.after(g, c, lo, LW);
  return g;
}

function filesDrawer(g, c, x, y, w, h) {
  const { T } = c, a = T.a, kind = { png: a.magenta, zip: a.yellow, md: a.cyan, pdf: a.red };
  g.putSegs(x + 2, y + 1, [{ t: 'FILES', fg: T.strong, b: 1 }, { t: '  recent · drag one onto the terminal', fg: T.dim }]);
  [['png', 'checkout bug.png', 'Desktop', '2m'], ['zip', 'shop-api-logs.zip', 'Downloads', '14m'], ['md', 'CHANGELOG.md', 'shop-api', '12m'], ['md', 'checkout-flow-spec.md', 'Downloads', '1h'], ['pdf', 'invoice-template.pdf', 'Downloads', '3h']].forEach(([k, n, wh, age], i) => {
    const yy = y + 3 + i; if (!i) g.fill(x, yy, w, 1, T.sel);
    g.putSegs(x + 2, yy, [{ t: k.padEnd(5), fg: kind[k] }, { t: n, fg: i ? T.fg : T.dim, i: !i }]);
    g.put(x + w - 14, yy, wh.padEnd(10), { fg: T.dim }); g.put(x + w - 1 - L(age), yy, age, { fg: T.dim });
  });
  g.putSegs(x + 2, y + h - 2, BTNS(T, [['Close', 'Esc'], ['All files', 'f']]));
}

function newTask(T, G) {
  const g = home(T, G, 100, 30); g.dimAll();
  const bx = 10, by = 5, bw = 80, X = bx + 3;
  g.box(bx, by, bw, 16, { fg: T.acc, bg: T.pop, title: [{ t: ' new task ', fg: T.accInk, bg: T.accFill, b: 1 }] });
  g.put(X, by + 2, 'What should be done?', { fg: T.dim });
  g.putSegs(X, by + 3, [{ t: '> ', fg: T.acc, b: 1 }, { t: 'fix the flaky checkout', fg: T.strong, b: 1 }, { t: '█', fg: T.acc }]);
  g.putSegs(X, by + 5, [{ t: ' claude ▾ ', bg: T.btn, fg: T.text }, gap(1), { t: ' shop-api ▾ ', bg: T.btn, fg: T.ws.violet }, gap(1), { t: ' own branch ▾ ', bg: T.btn, fg: T.text }, { t: '  task/fix-flaky-checkout', fg: T.dim }]);
  g.hl(bx + 1, by + 7, bw - 2, '─', { fg: T.line });
  g.put(X, by + 8, 'FROM YOUR INBOX', { fg: T.dim, b: 1 });
  g.fill(bx + 1, by + 9, bw - 2, 1, T.sel); g.put(bx + 1, by + 9, '>', { fg: T.acc, b: 1 });
  g.putSegs(X, by + 9, [{ t: 'ABC-140  ', fg: T.dim }, { t: 'Flaky checkout e2e on CI', fg: T.strong }, { t: '   Linear · ticket goes to the agent', fg: T.dim }]);
  g.putSegs(X, by + 10, [{ t: '#412     ', fg: T.dim }, { t: 'add rate limiter to /login', fg: T.text }, { t: '   PR · checks failing', fg: T.dim }]);
  g.putSegs(X, by + 13, [...BTN(T, 'Start', 'Enter', 'primary'), gap(1), ...BTNS(T, [['Attach file', '@'], ['Cancel', 'Esc']]), { t: '   or drop a file here', fg: T.dim }]);
  return g;
}

const termLines = (T) => { const a = T.a; return [
  [{ t: '> ', fg: a.gray }, { t: 'fix the flaky checkout test', fg: T.strong, b: 1 }],
  [],
  [{ t: '• ', fg: a.green }, { t: 'Read ' }, { t: 'test/checkout.spec.ts', fg: a.cyan }],
  [{ t: '• ', fg: a.green }, { t: 'Read ' }, { t: 'test/mocks/cart.ts', fg: a.cyan }],
  [{ t: '• ', fg: a.green }, { t: 'The test sleeps 500 ms for the cart and races the API mock. On a slow CI box the' }],
  [{ t: '  mock answers after the assertion runs.' }],
  [{ t: '• ', fg: a.green }, { t: "Plan: await the mock's ready promise instead of sleeping." }],
  [],
  [{ t: 'Edit ', b: 1 }, { t: 'test/checkout.spec.ts', fg: a.cyan }],
  [{ t: '  18 ', fg: a.gray }, { t: '- await page.waitForTimeout(500);', fg: a.red }],
  [{ t: '  18 ', fg: a.gray }, { t: '+ await mocks.cart.ready;', fg: a.green }],
  [],
  [{ t: 'Now I want to run the checkout tests to confirm.' }],
  [],
  [{ t: 'Run ', fg: a.yellow, b: 1 }, { t: 'npm test -- checkout', fg: T.strong }, { t: '?', fg: a.yellow, b: 1 }],
  [{ t: '❯ 1. Yes', fg: T.strong, b: 1 }],
  [{ t: '  2. Yes, and always allow npm test' }],
  [{ t: '  3. No, tell it what to do instead' }]
]; };

function inside(T, G, W = 160, H = 45) {
  const g = new Grid(W, H, T), c = { T, Gl: G, W }, tasks = tasksData(T), t = tasks[0];
  crumbBar(g, c, t, [{ t: 'claude', fg: T.text }, { t: ' · terminal', fg: T.dim }]);
  g.hl(0, 1, W, '─', { fg: T.line });
  const lines = termLines(T), top = H - 3 - lines.length;
  lines.forEach((l, i) => g.putSegs(2, top + i, l, {}, W - 1));
  g.putSegs(2, H - 3, [{ t: '❯ ', fg: T.a.gray }, { t: '█' }]);
  statusBar(g, c, { left: [{ t: ' ' + G.needs + ' 1 other needs you', fg: T.needs, b: 1 }, gap(), { t: G.ready + ' 2 ready', fg: T.ready }], right: joinH(HK(T, 'Ctrl+Space t', 'tasks', T.dim), HK(T, 'Ctrl+Space n', 'next', T.dim)) });
  return g;
}

const diffLines = (T) => { const a = T.a; return [
  [{ t: 'src/auth.ts', fg: T.strong, b: 1 }, { t: '   +4 −1', fg: T.dim }],
  [],
  [{ t: '@@ -1,4 +1,5 @@', fg: a.cyan }],
  [{ t: '   1    ', fg: a.gray }, { t: "import { Router } from 'express';" }],
  [{ t: '   2    ', fg: a.gray }, { t: "import { verify } from './hash';" }],
  [{ t: '      + ', fg: a.green }, { t: "import { limit } from './rate';", fg: a.green }],
  [{ t: '   3    ', fg: a.gray }],
  [{ t: '   4    ', fg: a.gray }, { t: 'export const router = Router();' }],
  [],
  [{ t: '@@ -38,7 +39,9 @@ ', fg: a.cyan }, { t: 'router setup', fg: a.gray }],
  [{ t: '  38    ', fg: a.gray }, { t: '// POST /login' }],
  [{ t: '  41  - ', fg: a.red }, { t: "router.post('/login', async (req, res) => {", fg: a.red }],
  [{ t: '      + ', fg: a.green }, { t: "router.post('/login', limit({ max: 5 }), async (req, res) => {", fg: a.green }],
  [{ t: '  42    ', fg: a.gray }, { t: '  const { email, password } = req.body;' }],
  [{ t: '      + ', fg: a.green }, { t: '  if (!email) return res.status(400).end();', fg: a.green }],
  [{ t: '  43    ', fg: a.gray }, { t: '  const user = await users.find(email);' }],
  [{ t: '  44    ', fg: a.gray }, { t: '  if (!user) return res.status(401).end();' }]
]; };

function review(T, G, W = 160, H = 45) {
  const g = new Grid(W, H, T), c = { T, Gl: G, W }, tasks = tasksData(T), t = tasks[2], a = T.a;
  crumbBar(g, c, t);
  const small = W < 140;
  g.putSegs(2, 2, [{ t: G.ok + ' tests 14/14', fg: T.ok }, gap(), { t: G.ok + ' lint', fg: T.ok }, gap(), { t: G.ok + ' typecheck', fg: T.ok }, gap(6), { t: '3 files  ', fg: T.text }, { t: '+42', fg: a.green }, { t: ' −7', fg: a.red }, ...(small ? [] : [{ t: '   by claude · 12m', fg: T.dim }])], {}, W - 1);
  g.hl(0, 3, W, '─', { fg: T.line });
  const FW = small ? 30 : 42, top = 4, bot = H - 4;
  g.put(2, top + 1, 'FILES', { fg: T.dim, b: 1 });
  [['M', 'src/auth.ts', '+4', '−1', 1], ['A', 'src/rate.ts', '+24', ''], ['M', 'test/auth.test.ts', '+14', '−6']].forEach(([s, f, p, m, cur], i) => {
    const y = top + 2 + i; if (cur) { g.fill(0, y, FW, 1, T.sel); g.put(0, y, '>', { fg: T.acc, b: 1 }); }
    g.putSegs(2, y, [{ t: s + ' ', fg: s === 'M' ? a.yellow : a.green, b: 1 }, { t: f, fg: cur ? T.strong : T.fg, b: !!cur }]);
    const r = [{ t: p, fg: a.green }, { t: m ? ' ' + m : '', fg: a.red }]; g.putSegs(FW - 2 - segLen(r), y, r);
  });
  g.put(2, top + 7, 'CLAUDE SAYS', { fg: T.dim, b: 1 });
  wrap('Added a token bucket (5 requests a minute per IP) in src/rate.ts and wired it into POST /login. New tests cover the 6th request and the reset after a minute.', FW - 4).forEach((l, i) => g.put(2, top + 8 + i, l, { fg: T.text }));
  g.vl(FW, top, bot - top, '│', { fg: T.line });
  diffLines(T).forEach((l, i) => { if (top + 1 + i < bot) g.putSegs(FW + 3, top + 1 + i, l, {}, W - 1); });
  g.hl(0, H - 3, W, '─', { fg: T.line });
  const acts = small
    ? [...BTN(T, 'Commit', 'c', 'primary'), gap(1), ...BTNS(T, [['Merge', 'm'], ['PR', 'p']]), gap(3), ...BTN(T, 'Reply', 'r'), gap(1), ...BTN(T, 'Discard', 'd', 'danger')]
    : [...BTN(T, 'Commit', 'c', 'primary'), gap(1), ...BTNS(T, [['Merge into main', 'm'], ['Open PR', 'p']]), gap(6), ...BTN(T, 'Reply to claude', 'r'), gap(1), ...BTN(T, 'Discard', 'd', 'danger'), gap(6), { t: 'scroll, or ', fg: T.dim }, KY(T, ']'), { t: ' next file', fg: T.dim }];
  for (let i = 0; i < 6; i++) g.set(W - 1, top + 1 + i, '┃', { fg: T.dim });
  g.putSegs(2, H - 2, acts, {}, W - 1);
  statusBar(g, c, { left: counts(c, tasks) });
  return g;
}

function files(T, G, W = 160, H = 45) {
  const g = new Grid(W, H, T), c = { T, Gl: G, W }, tasks = tasksData(T), a = T.a;
  topBar(g, c, 'files', [{ t: 'inserts into  ', fg: T.dim }, { t: G.needs + ' fix the flaky checkout test', fg: T.needs }]);
  g.putSegs(2, 2, [{ t: '> ', fg: T.acc, b: 1 }, { t: '█', fg: T.acc }, { t: '  type to fuzzy-search shop-api', fg: T.dim, i: 1 }]);
  g.hl(0, 3, W, '─', { fg: T.line });
  const MW = Math.floor(W / 2);
  g.putSegs(2, 5, [{ t: 'RECENT', fg: T.dim, b: 1 }, { t: '   Downloads · Desktop · your projects', fg: T.dim }]);
  const kind = { png: a.magenta, zip: a.yellow, md: a.cyan, pdf: a.red, csv: a.green };
  [['png', 'checkout bug.png', 'Desktop', '2m', 1], ['zip', 'shop-api-logs.zip', 'Downloads', '14m'], ['md', 'CHANGELOG.md', 'shop-api · by claude', '12m'], ['md', 'checkout-flow-spec.md', 'Downloads', '1h'], ['pdf', 'invoice-template.pdf', 'Downloads', '3h'], ['csv', 'orders-export.csv', 'Downloads', 'yesterday']].forEach(([k, n, wh, age, cur], i) => {
    const y = 6 + i; if (cur) { g.fill(0, y, MW, 1, T.sel); g.put(0, y, '>', { fg: T.acc, b: 1 }); }
    g.putSegs(2, y, [{ t: k.padEnd(5), fg: kind[k] }, { t: trunc(n, MW - 32), fg: cur ? T.strong : T.fg, b: !!cur }]);
    g.put(MW - 26, y, trunc(wh, 18), { fg: T.dim }); g.put(MW - 2 - L(age), y, age, { fg: T.dim });
  });
  g.put(2, 13, 'checkout bug.png · PNG 1440×900 · 312 KB · ~\\Desktop', { fg: T.dim, i: 1 });
  g.vl(MW, 4, H - 8, '│', { fg: T.line });
  g.putSegs(MW + 3, 5, [{ t: 'PROJECT', fg: T.dim, b: 1 }, { t: '   shop-api · recently edited', fg: T.dim }]);
  [['test/checkout.spec.ts', 'claude', '3m'], ['src/rate.ts', 'claude', '12m'], ['src/auth.ts', 'claude', '12m'], ['test/mocks/cart.ts', 'you', '1h'], ['db/schema.sql', 'gemini', '1m'], ['package.json', 'codex', '1h']].forEach(([p, who, age], i) => {
    const y = 6 + i; const sl = p.lastIndexOf('/');
    g.putSegs(MW + 3, y, [{ t: p.slice(0, sl + 1), fg: T.dim }, { t: p.slice(sl + 1), fg: T.fg }]);
    g.put(W - 14, y, who.padEnd(7), { fg: T.dim }); g.put(W - 2 - L(age), y, age, { fg: T.dim });
  });
  g.hl(0, H - 3, W, '─', { fg: T.line });
  g.putSegs(2, H - 2, [...BTN(T, 'Insert into prompt', 'Enter', 'primary'), gap(1), ...BTNS(T, [['Open', 'o'], ['Copy path', 'y'], ['Preview', 'Space']]), gap(4), { t: 'or drag a file onto a terminal', fg: T.dim }], {}, W - 1);
  statusBar(g, c, { left: counts(c, tasks) });
  return g;
}

function inbox(T, G, W = 160, H = 45) {
  const g = new Grid(W, H, T), c = { T, Gl: G, W }, tasks = tasksData(T), LW = Math.floor(W * 0.56);
  topBar(g, c, 'inbox', [{ t: 'GitHub', fg: T.text }, { t: ' ' + G.ok, fg: T.ok }, gap(2), { t: 'Linear', fg: T.text }, { t: ' ' + G.ok, fg: T.ok }, gap(2), { t: 'Plane', fg: T.text }, { t: ' ' + G.ok, fg: T.ok }]);
  let y = 2, idx = 0;
  const groups = [
    ['PULL REQUESTS', [['#412', 'add rate limiter to /login', [G.fail + ' checks failing', T.err], 'shop-api', T.ws.violet, '→ task working'], ['#409', 'nav a11y fixes', [G.needs + ' review requested', T.needs], 'web-shop', T.ws.sky], ['#405', 'cache product images', [G.ok + ' approved', T.ok], 'web-shop', T.ws.sky], ['#398', 'split orders service', [G.fail + ' merge conflict', T.err], 'shop-api', T.ws.violet]]],
    ['ISSUES', [['#88', 'Cart total wrong with coupons', ['opened by @lena', T.dim], 'shop-api', T.ws.violet]]],
    ['TICKETS', [['ABC-131', 'Export orders as CSV', ['Linear · High', T.text], 'shop-api', T.ws.violet, '', 1], ['ABC-123', 'Checkout fails on Safari', ['Linear · Urgent', T.text], 'web-shop', T.ws.sky, '→ task needs you'], ['PLN-7', 'Update privacy page', ['Plane · Low', T.text], 'docs', T.ws.pink]]]];
  for (const [label, rows] of groups) {
    g.putSegs(2, y++, [{ t: label, fg: T.dim, b: 1 }, { t: '  ' + rows.length, fg: T.dim }]);
    for (const [id, title, st, proj, pc, linked, cur] of rows) {
      if (cur) { g.fill(0, y, LW, 1, T.sel); g.put(0, y, '>', { fg: T.acc, b: 1 }); }
      const right = [{ t: st[0].padEnd(20), fg: st[1] }, { t: proj.padEnd(9), fg: pc }];
      g.putSegs(2, y, [{ t: id.padEnd(9), fg: T.dim }, { t: title, fg: cur ? T.strong : T.fg, b: !!cur }, { t: linked ? '  ' + linked : '', fg: T.dim, i: 1 }], {}, LW - segLen(right) - 2);
      g.putSegs(LW - 1 - segLen(right), y, right); y++; idx++;
    }
    y++;
  }
  const X = LW + 3, iw = W - LW - 5; g.vl(LW, 1, H - 2, '│', { fg: T.line });
  let yy = 2;
  g.put(X, yy++, 'ABC-131  Export orders as CSV', { fg: T.strong, b: 1 });
  g.putSegs(X, yy++, [{ t: 'Linear · High · assigned to you · ', fg: T.dim }, { t: 'shop-api', fg: T.ws.violet }]);
  yy++;
  wrap('Support wants a CSV export of orders for a date range, so they can reconcile refunds without asking engineering. Include order id, customer email, status, total and refund amount.', iw).forEach(l => g.put(X, yy++, l, { fg: T.text }));
  yy++;
  g.put(X, yy++, 'ACCEPTANCE', { fg: T.dim, b: 1 });
  ['GET /orders/export?from=&to= returns text/csv', 'Only admins can call it', 'Covered by a test'].forEach(l => g.put(X, yy++, '· ' + l, { fg: T.text }));
  yy += 2;
  g.putSegs(X, yy++, [...BTN(T, 'Make it a task', 't', 'primary'), { t: '  "ABC-131 export orders as CSV"', fg: T.text }], {}, W - 1);
  g.put(X, yy++, 'the ticket text goes to the agent as context', { fg: T.dim, i: 1 });
  yy++;
  g.putSegs(X, yy++, BTNS(T, [['Open in Linear', 'o'], ['Snooze', 's']]));
  statusBar(g, c, { left: counts(c, tasks), right: [{ t: 'synced 1m ago', fg: T.dim }] });
  return g;
}

function toolbox(T, G, W = 160, H = 45) {
  const g = new Grid(W, H, T), c = { T, Gl: G, W }, tasks = tasksData(T), LW = Math.floor(W * 0.6);
  topBar(g, c, 'toolbox');
  let x = 2;
  ['Claude Code', 'Codex', 'Cursor', 'Gemini'].forEach((n, i) => { x = g.putSegs(x, 2, [{ t: ' ' + n + ' ', bg: i ? T.surf : T.btn, fg: i ? T.text : T.strong, b: !i }]) + 1; });
  const sc = [{ t: 'show  ', fg: T.dim }, { t: ' shop-api + global ', bg: T.btn, fg: T.strong, b: 1 }, gap(1), { t: ' shop-api only ', bg: T.surf, fg: T.text }, gap(1), { t: ' global only ', bg: T.surf, fg: T.text }];
  g.putSegs(W - 1 - segLen(sc), 2, sc);
  g.hl(0, 3, W, '─', { fg: T.line });
  const C = [2, 30, 52];
  ['NAME', 'FROM', ''].forEach((h, i) => g.put(C[i], 4, h, { fg: T.dim, b: 1 })); g.put(LW - 8, 4, 'STATE', { fg: T.dim, b: 1 });
  const from = s => s === 'project' ? T.ws.violet : s === 'global' ? T.text : T.ws.orange;
  const groups = [
    ['INSTRUCTIONS', [['CLAUDE.md', 'project', 1, '.\\CLAUDE.md'], ['CLAUDE.md', 'global', 1, '~\\.claude\\CLAUDE.md']]],
    ['MCP SERVERS', [['github', 'plugin: github', 1], ['linear', 'global', 1], ['postgres', 'project', 0, '', 1], ['playwright', 'project', 1]]],
    ['SKILLS', [['pdf', 'global', 1], ['frontend-design', 'plugin: design-kit', 1]]],
    ['PLUGINS', [['github 1.4.0', 'global', 1], ['design-kit 0.3', 'global', 1]]],
    ['HOOKS', [['after edit → prettier', 'project', 1], ['on stop → notify', 'global', 1]]],
    ['SUB-AGENTS', [['code-reviewer', 'project', 1], ['test-runner', 'global', 0]]]];
  let y = 6;
  for (const [label, rows] of groups) {
    g.putSegs(2, y++, [{ t: label, fg: T.dim, b: 1 }, { t: '  ' + rows.length, fg: T.dim }]);
    for (const [n, f, on, path, cur] of rows) {
      if (cur) { g.fill(0, y, LW, 1, T.sel); g.put(0, y, '>', { fg: T.acc, b: 1 }); }
      g.put(C[0], y, n, { fg: on ? (cur ? T.strong : T.fg) : T.dim, b: !!cur }); g.put(C[1], y, f, { fg: from(f) }); if (path) g.put(C[2], y, path, { fg: T.dim });
      g.putSegs(LW - 8, y, [on ? { t: ' on  ', bg: T.btn, fg: T.ok, b: 1 } : { t: ' off ', bg: T.surf, fg: T.dim }]); y++;
    }
    y++;
  }
  const X = LW + 3; g.vl(LW, 4, H - 7, '│', { fg: T.line });
  let yy = 5;
  g.put(X, yy++, 'postgres', { fg: T.strong, b: 1 });
  g.putSegs(X, yy++, [{ t: 'MCP server · ', fg: T.dim }, { t: 'project', fg: T.ws.violet }, { t: ' · off', fg: T.dim }]); yy++;
  [['defined in', '.\\.mcp.json  line 12'], ['command', 'npx @mcp/postgres'], ['env', 'DATABASE_URL from .env'], ['used by', 'Claude Code only']].forEach(([k, v]) => { g.put(X, yy, k, { fg: T.dim }); g.put(X + 13, yy++, v, { fg: T.text }); });
  yy++;
  wrap('Off because it was turned off in this project on 28 Sep. Agents in shop-api can not query the database.', W - LW - 5).forEach(l => g.put(X, yy++, l, { fg: T.dim, i: 1 }));
  g.hl(0, H - 3, W, '─', { fg: T.line });
  g.putSegs(2, H - 2, [...BTNS(T, [['Turn on', 'Space'], ['Edit config file', 'e']]), gap(4), { t: 'click a tool or a scope above to switch · click on/off to toggle', fg: T.dim }], {}, W - 1);
  statusBar(g, c, { left: counts(c, tasks) });
  return g;
}

function firstRun(T, G) {
  const W = 100, H = 30, g = new Grid(W, H, T), c = { T, Gl: G, W }, S = ST(T, G);
  topBar(g, c, 'home');
  const X = 8;
  g.put(X, 4, 'Nothing to do yet.', { fg: T.strong, b: 1 });
  wrap('A task is one job for an agent, like "fix the flaky checkout test". drover gives it its own branch, starts the agent, and tells you when it needs you or is ready for review.', 70).forEach((l, i) => g.put(X, 6 + i, l, { fg: T.text }));
  const rows = [['n', 'New task', 'type what should be done'], ['i', 'Start from your inbox', '3 tickets and 1 PR are assigned to you'], ['p', 'Add a project', 'found 4 git repos in ~\\code']];
  rows.forEach(([k, l, d], i) => { const y = 11 + i * 2; g.putSegs(X, y, BTN(T, l, k, i ? 'normal' : 'primary')); g.put(X + 28, y, d, { fg: T.dim }); });
  g.put(X, 19, 'A TASK MOVES THROUGH', { fg: T.dim, b: 1 });
  g.putSegs(X, 20, [{ t: S.work.g + ' working', fg: S.work.c }, { t: '  →  ', fg: T.dim }, { t: S.needs.g + ' needs you', fg: S.needs.c, b: 1 }, { t: '  →  ', fg: T.dim }, { t: S.ready.g + ' ready', fg: S.ready.c }, { t: '  →  ', fg: T.dim }, { t: S.shipped.g + ' shipped', fg: S.shipped.c }]);
  g.put(X, 21, 'you answer at "needs you" and review at "ready"', { fg: T.dim, i: 1 });
  g.putSegs(X, 24, [{ t: 'Every command is ', fg: T.dim }, KY(T, 'Ctrl+Space'), { t: ' then one letter. Try ', fg: T.dim }, KY(T, 'Ctrl+Space ?'), { t: '.', fg: T.dim }]);
  statusBar(g, c, { left: [{ t: ' no projects yet', fg: T.dim }] });
  return g;
}

function inboxEmpty(T, G) {
  const W = 100, H = 30, g = new Grid(W, H, T), c = { T, Gl: G, W };
  topBar(g, c, 'inbox');
  const X = 8;
  g.put(X, 4, 'Your inbox is empty because nothing is connected.', { fg: T.strong, b: 1 });
  wrap('The inbox collects pull requests, issues and tickets that want you, so you never open a browser to check. Any item becomes a task with one key.', 72).forEach((l, i) => g.put(X, 6 + i, l, { fg: T.text }));
  [['g', 'GitHub', 'found a gh login for @lena'], ['l', 'Linear', 'opens a browser once to sign in'], ['p', 'Plane', 'paste an API key']].forEach(([k, n, d], i) => { const y = 10 + i * 2; g.putSegs(X, y, BTN(T, 'Connect ' + n, k, i ? 'normal' : 'primary')); g.put(X + 22, y, d, { fg: T.dim }); });
  g.put(X, 18, 'Nothing is sent to drover servers; tokens stay in your OS keychain.', { fg: T.dim, i: 1 });
  statusBar(g, c, { left: [{ t: ' inbox not connected', fg: T.dim }] });
  return g;
}

function stagesSheet(T, G) {
  const W = 100, H = 20, g = new Grid(W, H, T), c = { T, Gl: G, W }, S = ST(T, G);
  const all = tasksData(T), pick = [all[0], all[2], all[4], all[7]];
  g.put(0, 0, 'IN THE TASK LIST · 100 COLS', { fg: T.dim, b: 1 });
  taskList(g, c, 0, 1, W, 12, pick, { noSel: 1 });
  g.put(0, 13, 'AS A WORD, WHEREVER A TASK IS NAMED', { fg: T.dim, b: 1 });
  let x = 2; ['needs', 'work', 'ready', 'shipped'].forEach(st => { x = g.putSegs(x, 14, [{ t: S[st].g + ' ' + S[st].l, fg: S[st].c, b: st === 'needs' }]) + 5; });
  g.put(0, 16, 'STATUSLINE COUNTS', { fg: T.dim, b: 1 });
  const s = new Grid(W, 1, T); statusBar(s, c, { left: counts(c, all), right: HK(T, 'n', 'new task', T.dim) }); g.c[17] = s.c[0];
  g.put(0, 19, 'Only needs-you is bold and warm. Working moves; ready is cool; shipped fades.', { fg: T.dim, i: 1 });
  return g;
}

function mHover(T, G) {
  const S = ST(T, G);
  return home(T, G, 100, 30, { hov: 2, hint: [{ t: S.ready.g + ' ready', fg: T.ready, b: 1 }, { t: ' · finished, waiting for review · click: terminal · double-click: review', fg: T.text }], after: (g, c, lo) => pointer(g, 2, lo.ys[2]) });
}

function mMenu(T, G) {
  return home(T, G, 100, 30, { hov: 2, hint: [{ t: 'right-click: everything you can do to this task', fg: T.text }], after: (g, c, lo) => {
    const py = lo.ys[2], px = 34; pointer(g, px, py);
    const items = [['Review', 'Enter'], ['Open terminal', 'z'], ['Reply to agent…', 'r'], null, ['Copy branch name', 'y'], ['Open folder in editor', 'e'], ['Rename…', 'F2'], ['Move to project…', ''], null, ['Stop agent', ''], ['Discard task', 'd', 1]];
    const bx = px + 1, by = py + 1, bw = 32;
    g.box(bx, by, bw, items.length + 2, { fg: T.line, bg: T.pop });
    items.forEach((it, i) => { const y = by + 1 + i; if (!it) { g.hl(bx + 1, y, bw - 2, '─', { fg: T.line }); return; } if (i === 0) g.fill(bx + 1, y, bw - 2, 1, T.hov); g.put(bx + 2, y, it[0], { fg: it[2] ? T.err : T.strong, b: i === 0 }); if (it[1]) g.put(bx + bw - 2 - L(it[1]), y, it[1], { fg: T.acc, b: 1 }); });
  } });
}

function mDrag(T, G) {
  return home(T, G, 160, 45, { drawer: 1, drop: 1, hint: [{ t: 'dragging checkout bug.png', fg: T.strong, b: 1 }, { t: ' · release over a terminal to insert its path · Esc cancels', fg: T.text }], after: (g, c, lo, LW) => { const px = LW + 30, py = 28; g.putSegs(px + 1, py, [{ t: ' + checkout bug.png ', bg: T.accFill, fg: T.accInk, b: 1 }]); pointer(g, px, py); } });
}

function mResize(T, G) {
  return home(T, G, 160, 45, { LW: 64, divider: 1, hint: [{ t: 'drag', fg: T.strong, b: 1 }, { t: ' to resize · queue 64 cols, terminal 96 · double-click to reset', fg: T.text }], after: (g, c, lo, LW) => pointer(g, LW, 20) });
}

function mStates(T, G) {
  const W = 100, H = 22, g = new Grid(W, H, T), lab = (y, s) => g.put(0, y, s, { fg: T.dim, b: 1 }), cap = (x, y, s) => g.put(x, y, s, { fg: T.dim, i: 1 });
  const C = 20;
  lab(0, 'BUTTON'); let x = C;
  [['normal', 'normal'], ['hover', 'hover'], ['pressed', 'pressed'], ['primary', 'primary'], ['danger', 'danger'], ['off', 'disabled']].forEach(([st, l]) => { cap(x, 1, l); x = g.putSegs(x, 0, BTN(T, st === 'danger' ? 'Discard' : 'Commit', st === 'danger' ? 'd' : 'c', st)) + 2; });
  lab(3, 'NAV ENTRY'); x = C;
  [[T.btn, T.strong, T.acc, 1, 'current'], [T.surf, T.text, T.dim, 0, 'normal'], [T.hov, T.strong, T.acc, 0, 'hover']].forEach(([bg, fg, k, b, l]) => { cap(x, 4, l); x = g.putSegs(x, 3, [{ t: ' Inbox 5 ', bg, fg, b }, { t: 'i ', bg, fg: k, b: 1 }]) + 3; });
  lab(6, 'LIST ROW');
  g.putSegs(C + 2, 6, [{ t: G.ready + ' ', fg: T.ready }, { t: 'update README examples', fg: T.strong }]); cap(C + 44, 6, 'normal');
  g.fill(C, 7, 40, 1, T.hov); g.putSegs(C + 2, 7, [{ t: G.ready + ' ', fg: T.ready }, { t: 'update README examples', fg: T.strong }]); cap(C + 44, 7, 'hover · whole row is the target');
  g.fill(C, 8, 40, 1, T.sel); g.put(C, 8, '>', { fg: T.acc, b: 1 }); g.putSegs(C + 2, 8, [{ t: G.ready + ' ', fg: T.ready }, { t: 'update README examples', fg: T.strong, b: 1 }]); cap(C + 44, 8, 'selected (click or keyboard)');
  lab(10, 'TOGGLE / MENU'); x = C;
  x = g.putSegs(x, 10, [{ t: ' on  ', bg: T.btn, fg: T.ok, b: 1 }]) + 1; x = g.putSegs(x, 10, [{ t: ' off ', bg: T.surf, fg: T.dim }]) + 3; g.putSegs(x, 10, [{ t: ' claude ▾ ', bg: T.btn, fg: T.text }]); cap(C, 11, 'click flips · dropdowns open a menu under the chip');
  lab(13, 'TEXT SELECTION');
  g.putSegs(C, 13, [{ t: '• The test ' }, { t: 'sleeps 500 ms for the cart', bg: T.selTxt, fg: T.strong }, { t: ' and races the mock.' }]); cap(C, 14, 'drag over text to select · release copies · works in terminals and diffs');
  lab(16, 'SCROLL / DIVIDER');
  g.put(C, 16, 'list', { fg: T.text }); g.set(C + 6, 16, '┃', { fg: T.dim }); cap(C + 8, 16, 'scroll thumb: wheel or drag');
  g.set(C + 40, 16, '│', { fg: T.line }); g.set(C + 42, 16, '┃', { fg: T.acc }); cap(C + 44, 16, 'divider: normal / hover, drag');
  lab(18, 'POINTER'); pointer(g, C, 18); cap(C + 2, 18, 'mouse pointer in these mockups (the terminal draws it)');
  lab(20, 'TARGET SIZE'); cap(C, 20, 'every target is at least 1 row × 3 cols; buttons pad 1 cell each side');
  return g;
}

export function build(theme, gl) {
  const T = THEMES[theme] || THEMES.dark, T2 = THEMES[T.name === 'dark' ? 'light' : 'dark'], G = GLYPHS[gl] || GLYPHS.unicode, R = g => g.rows();
  return {
    home: [
      { id: '1a', title: 'Home · 100×30', note: 'Queue on top, the selected task\'s live terminal below. Answer the agent prompt with 1 / 2 / 3; the bar under it only points there.', rows: R(home(T, G, 100, 30)) },
      { id: '1b', title: 'Home · 160×45', note: 'Queue left, live terminal of the selected task right. Move the cursor and the terminal follows.', rows: R(home(T, G, 160, 45)) },
      { id: '1c', title: 'Home · 240×65', note: 'Wide. More queue columns and a bigger terminal.', rows: R(home(T, G, 240, 65)) },
      { id: '1d', title: 'Home · 160×45 · ' + T2.name, note: 'Same grid, other theme.', rows: R(home(T2, G, 160, 45)) }],
    screens: [
      { id: '2a', title: 'New task · n', note: 'As light as a search box: one line, three quiet options, and inbox matches you can pick instead.', rows: R(newTask(T, G)) },
      { id: '2b', title: 'Review · Enter on a ready task', note: 'Checks on top, files and the agent\'s summary left, diff right, one action row. The most-used screen.', rows: R(review(T, G)) },
      { id: '2c', title: 'Review · 100×30', note: 'Same screen at the minimum.', rows: R(review(T, G, 100, 30)) },
      { id: '2d', title: 'Inside a task', note: 'z on Home: the same terminal, full screen. Task name, stage and branch stay on row 1; ‹ Home (Ctrl+Space h) goes back.', rows: R(inside(T, G)) },
      { id: '2e', title: 'Files', note: 'Recent files from Downloads, Desktop and your projects, plus fuzzy search in the project. Enter drops the path into the agent\'s prompt.', rows: R(files(T, G)) },
      { id: '2f', title: 'Inbox', note: 'PRs, issues and tickets in one list. t turns the selected item into a task, with its text handed to the agent.', rows: R(inbox(T, G)) },
      { id: '2g', title: 'Toolbox', note: 'What each AI tool is set up with, here versus globally, and where each piece comes from.', rows: R(toolbox(T, G)) }],
    empty: [
      { id: '3a', title: 'First run', note: 'Explains a task in two sentences, offers three ways to start, shows the four stages once.', rows: R(firstRun(T, G)) },
      { id: '3b', title: 'Inbox, nothing connected', note: 'Says why it is empty and the one key to fix it.', rows: R(inboxEmpty(T, G)) }],
    mouse: [
      { id: '6a', title: 'Hover · 100×30', note: 'Row under the pointer lights up; hovering a symbol explains it in the statusline.', rows: R(mHover(T, G)) },
      { id: '6b', title: 'Right-click a task', note: 'Context menu at the pointer with everything you can do, keys on the right.', rows: R(mMenu(T, G)) },
      { id: '6c', title: 'Drag a file onto the terminal · 160×45', note: 'Files opens in the queue column; the terminal becomes a lime drop target.', rows: R(mDrag(T, G)) },
      { id: '6d', title: 'Drag to resize · 160×45', note: 'The divider turns lime on hover; drag it, double-click resets.', rows: R(mResize(T, G)) },
      { id: '6e', title: 'States', note: 'Normal, hover, pressed, selected, disabled. Colour, bold and inverse only.', rows: R(mStates(T, G)) }],
    stages: [{ id: '4a', title: 'Stage indicators', note: 'Glyph + word, always. Grouped by stage in the queue.', rows: R(stagesSheet(T, G)) }]
  };
}

export { Grid, tasksData, termLines, wrapSegs, trunc, segLen, L, wrap, mix };
