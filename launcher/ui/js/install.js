// Install page and the first-run flow: picks the disc image, shows the validation checklist as it fills in, runs the build and shows
// overall and per-stage progress, Cancel / Resume, the log tail and the free-space numbers. All work happens in Rust (src/install.rs, the
// mcla-install crate); this file polls `install_poll` for events and draws them. Every sentence comes from the locales (keys `ins.*`, `dyn.*`).
(function () {
  const $ = (sel, root) => (root || document).querySelector(sel);
  const $$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));
  const t = (k, a) => I18N.t(k, a);

  // The four checklist rows of the design; each row is the worst of its checks (ids come from mcla-install's validation stage).
  const ROWS = [
    { key: 'dyn.chk1', checks: ['readable', 'layout'] },
    { key: 'dyn.chk2', checks: ['required_files', 'executable'] },
    { key: 'dyn.chk3', checks: ['archive_key', 'entries'] },
    { key: 'dyn.chk4', checks: ['size'] },
  ];
  // Build stages as the page shows them. ffmpeg shares the audio row, verify shares the stamp.
  const STAGES = [
    { id: 'extract', names: ['extract'], key: 'dyn.s_extract', weight: 0.09 },
    { id: 'shaders', names: ['shaders'], key: 'dyn.s_shaders', weight: 0.01 },
    { id: 'audio', names: ['ffmpeg', 'audio'], key: 'dyn.s_audio', weight: 0.82 },
    { id: 'video', names: ['video'], key: 'dyn.s_video', weight: 0.06 },
    { id: 'verify', names: ['stamp', 'verify'], key: 'dyn.s_verify', weight: 0.02 },
  ];
  const STAGE_OF = {};
  STAGES.forEach((s) => s.names.forEach((n) => { STAGE_OF[n] = s; }));

  const S = {
    app: null, info: null, bridge: Bridge,
    iso: null, identity: null, warnings: [], checks: {}, valErr: null, validating: false,
    hash: { state: 'idle', done: 0, total: 0, verified: null },
    building: false, built: false, buildErr: null, cancelled: false, resumable: false,
    stage: {}, ffmpegActive: false, t0: 0, since: 0, log: [], polling: false, timer: null,
  };

  // ---------- helpers ----------
  const gb = (n) => (n / 1e9).toFixed(n >= 1e10 ? 0 : 1) + ' GB';
  const mb = (n) => Math.round(n / 1e6) + ' MB';
  const hhmmss = () => new Date().toTimeString().slice(0, 8);
  function errText(e) {
    const code = e && e.code ? e.code : 'io';
    const key = 'dyn.err.' + code;
    const txt = t(key);
    return txt === key ? (e && e.message) || String(e) : txt;
  }
  function logLine(text, cls) {
    S.log.push({ time: hhmmss(), text, cls: cls || '' });
    if (S.log.length > 6) S.log.shift();
    renderLog();
  }

  // ---------- rendering ----------
  function rowStatus(row) {
    const cs = row.checks.map((id) => S.checks[id]).filter(Boolean);
    if (S.valErr && cs.some((c) => c.status === 'fail')) return { status: 'fail', text: cs.find((c) => c.status === 'fail').text };
    if (cs.some((c) => c.status === 'fail')) return { status: 'fail', text: cs.find((c) => c.status === 'fail').text };
    if (cs.length === row.checks.length) {
      const w = cs.find((c) => c.status === 'warn');
      return w ? { status: 'warn', text: w.text } : { status: 'pass', text: '' };
    }
    return { status: S.validating ? 'run' : 'idle', text: '' };
  }

  function icon(status) {
    const map = { pass: ['i-check', ''], warn: ['i-warn', 'warn'], fail: ['i-warn', 'bad'], run: ['i-spin', 'run'], idle: ['i-spin', 'idle'] };
    const [id, cls] = map[status] || map.idle;
    return '<svg class="ic ' + cls + '" style="width:18px;height:18px"><use href="#' + id + '"/></svg>';
  }

  function renderChecks() {
    const host = $('#ins-checks');
    if (!host) return;
    host.textContent = '';
    ROWS.forEach((row) => {
      const st = rowStatus(row);
      // with an error to show, the rows that never ran are noise and would push the box out of the card
      if (S.valErr && st.status === 'idle') return;
      const d = document.createElement('div');
      d.innerHTML = icon(st.status);
      const label = document.createElement('span');
      label.style.flex = '1';
      label.textContent = t(row.key);
      if (st.status === 'idle') label.style.color = 'var(--muted)';
      d.append(label);
      if (st.text && (st.status === 'warn' || st.status === 'fail')) {
        const sm = document.createElement('span');
        sm.className = 'mono';
        sm.style.cssText = 'font-size:11.5px;color:var(--' + (st.status === 'warn' ? 'warn' : 'bad') + ');max-width:210px;text-align:right';
        sm.textContent = st.text;
        d.append(sm);
      }
      host.append(d);
    });
    // the background checksum
    const h = S.hash;
    const d = document.createElement('div');
    const pct = h.total ? Math.floor((h.done * 100) / h.total) : 0;
    let status = 'idle', label = t('dyn.chk_hash'), right = '';
    if (h.state === 'run') { status = 'run'; right = pct + '%'; }
    else if (h.state === 'done') { status = h.verified ? 'pass' : 'warn'; label = t(h.verified ? 'dyn.hash_ok' : 'dyn.hash_other'); }
    else if (h.state === 'error') { status = 'warn'; label = t('dyn.hash_error'); }
    d.innerHTML = icon(status);
    const l = document.createElement('span');
    l.style.flex = '1';
    l.textContent = label;
    if (status === 'idle') l.style.color = 'var(--muted)';
    d.append(l);
    if (right) { const r = document.createElement('span'); r.className = 'mono'; r.style.cssText = 'font-size:13px;color:var(--warn)'; r.textContent = right; d.append(r); }
    if (!(S.valErr && status === 'idle')) host.append(d);
    if (h.state === 'run') {
      const bar = document.createElement('div');
      bar.className = 'bar thin warn';
      bar.style.cssText = '--p:' + pct + ';margin-left:28px;display:block';
      bar.innerHTML = '<i></i>';
      host.append(bar);
    }
    // the explanation box or the error box
    const err = $('#ins-err');
    if (S.valErr) {
      $('#ins-err-title').textContent = errText(S.valErr);
      $('#ins-err-detail').textContent = S.valErr.detail || '';
      $('#ins-err-detail').title = S.valErr.detail || '';
      err.hidden = false;
      $('#ins-note').hidden = true;
    } else {
      err.hidden = true;
      $('#ins-note').hidden = false;
    }
  }

  function renderIso() {
    const f = $('#ins-iso');
    f.textContent = S.iso || t('dyn.path_none');
    f.style.color = S.iso ? '' : 'var(--muted)';
    f.title = S.iso || '';
  }

  function renderSteps() {
    const valid = !!S.identity && !S.valErr;
    const st = {
      1: window.Update ? window.Update.step1() : { s: 'todo', d: 'ins.st1_d' },
      2: S.validating ? { s: 'now', d: 'dyn.st2_checking' } : valid ? { s: 'done', d: 'ins.st2_d' } : S.valErr ? { s: 'now', d: 'dyn.st2_error' } : { s: 'now', d: 'dyn.st2_none' },
      3: S.built ? { s: 'done', d: 'dyn.st3_done' } : S.building ? { s: 'now', d: 'ins.st3_d' } : valid ? { s: 'now', d: 'dyn.st3_ready' } : { s: 'todo', d: 'dyn.st3_wait' },
      4: S.built ? { s: 'done', d: 'dyn.st4_done' } : { s: 'todo', d: 'ins.st4_d' },
    };
    $$('.stepper .st').forEach((el) => {
      const n = el.dataset.step;
      const v = st[n];
      el.className = 'st ' + v.s;
      const dot = $('.dot', el);
      dot.innerHTML = v.s === 'done' ? '<svg class="ic"><use href="#i-check"/></svg>' : n;
      $('[data-step-d]', el).textContent = v.text != null ? v.text : t(v.d);
    });
    $$('.stepper .join').forEach((j) => { j.classList.toggle('done', st[Number(j.dataset.join)].s === 'done'); });
  }

  function stageFraction(st) {
    const rec = S.stage[st.id];
    if (!rec) return 0;
    if (rec.finished) return 1;
    return rec.total ? Math.min(1, rec.done / rec.total) : 0;
  }

  function overall() {
    if (S.built) return 1;
    return STAGES.reduce((a, st) => a + st.weight * stageFraction(st), 0);
  }

  function renderStages() {
    const host = $('#ins-stages');
    if (!host.children.length) {
      STAGES.forEach((st) => {
        const wrap = document.createElement('div');
        wrap.dataset.stage = st.id;
        wrap.innerHTML = '<div class="stage"><span class="n"><span class="mark"></span><span class="nm"></span></span><span class="st-r"></span></div><div class="bar thin" style="--p:0;margin:7px 0 0 26px" hidden><i></i></div>';
        host.append(wrap);
      });
    }
    STAGES.forEach((st) => {
      const wrap = $('[data-stage="' + st.id + '"]', host);
      const rec0 = S.stage[st.id];
      const useFf = st.id === 'audio' && S.ffmpegActive;
      const rec = useFf ? (S.ff ? { done: S.ff.done, total: S.ff.total, finished: false } : { done: 0, total: 0, finished: false }) : rec0;
      const frac = useFf ? (rec.total ? Math.min(1, rec.done / rec.total) : 0) : stageFraction(st);
      const done = S.built || !!(!useFf && rec && rec.finished);
      const running = S.building && rec && !rec.finished;
      const row = $('.stage', wrap);
      row.classList.toggle('waiting', !done && !running);
      $('.mark', wrap).innerHTML = done ? '<svg class="ic"><use href="#i-check"/></svg>' : '<span class="ring' + (running ? ' spin' : '') + '"></span>';
      $('.nm', wrap).textContent = t(st.id === 'audio' && S.ffmpegActive ? 'dyn.s_ffmpeg' : st.key);
      const r = $('.st-r', wrap);
      r.className = 'st-r' + (done ? ' ok' : running && rec.total ? ' pct' : '');
      r.textContent = done ? t('dyn.done') : running && rec.total ? Math.floor(frac * 100) + '%' : S.building || S.resumable ? t('dyn.waiting') : '';
      const bar = $('.bar', wrap);
      bar.hidden = !(running && rec.total);
      bar.style.setProperty('--p', Math.floor(frac * 100));
    });
  }

  function renderBuild() {
    const f = overall();
    $('#ins-pct').textContent = Math.floor(f * 100) + '%';
    $('#ins-overall').style.setProperty('--p', Math.floor(f * 100));
    const left = $('#ins-left');
    // time left from the speed since the first sample at 15% (extraction, and stages a resumed run skips, would skew a plain average)
    if (S.building && f >= 0.15 && !S.rate) S.rate = { t: Date.now(), f };
    if (S.building && S.rate && f - S.rate.f >= 0.04) {
      const secs = (((Date.now() - S.rate.t) / 1000) * (1 - f)) / (f - S.rate.f);
      left.textContent = secs < 60 ? t('dyn.left_lt1') : t('dyn.left_min', [Math.ceil(secs / 60)]);
    } else {
      left.textContent = S.built ? t('dyn.finished') : '';
    }
    renderStages();
    const start = $('#ins-start'), cancel = $('#ins-cancel'), note = $('#ins-btn-note');
    const valid = !!S.identity && !S.valErr;
    start.hidden = S.building;
    cancel.hidden = !S.building;
    start.disabled = !valid || S.validating;
    start.textContent = t(S.built ? 'dyn.rebuild' : S.resumable || (S.info && S.info.has_partial) ? 'dyn.resume' : 'ins.start');
    note.className = 'note s';
    note.style.color = '';
    if (S.building) { note.textContent = t('dyn.cancel_note'); }
    else if (S.buildErr) { note.textContent = S.cancelled ? t('dyn.cancelled') : errText(S.buildErr); note.style.color = S.cancelled ? '' : 'var(--bad)'; }
    else if (S.built) { note.textContent = t('dyn.built_note'); }
    else if (!valid) { note.textContent = t('dyn.need_disc'); }
    else if (S.info && S.info.ffmpeg_found === null && S.info.ffmpeg_download_bytes) { note.textContent = t('dyn.ffmpeg_note', [mb(S.info.ffmpeg_download_bytes), S.info.ffmpeg_version]); }
    else { note.textContent = t('dyn.ready_note'); }
    $('#ins-browse').disabled = S.building || S.validating;
    const ip = $('#ins-play');
    if (ip) ip.hidden = !(S.built && !S.building);
    const verify = $('#ins-verify');
    verify.hidden = !(S.info && S.info.installed) || S.building;
  }

  function renderSpace() {
    const i = S.info;
    $('#ins-folder').textContent = i ? i.root : t('f.unknown');
    if (!i) return;
    $('#ins-need').textContent = t('dyn.need', [gb(i.required_bytes)]);
    $('#ins-free').textContent = t('dyn.free', [gb(i.free_bytes)]);
    $('#ins-space-bar').style.setProperty('--p', i.free_bytes ? Math.min(100, Math.round((i.required_bytes * 100) / i.free_bytes)) : 0);
  }

  function renderLog() {
    const host = $('#ins-log');
    if (!host) return;
    host.textContent = '';
    S.log.forEach((l) => {
      const d = document.createElement('div');
      if (l.cls) d.className = l.cls;
      d.textContent = '[' + l.time + '] ' + l.text;
      host.append(d);
    });
  }

  function renderFirstRun() {
    const i = S.info;
    const fr = $('[data-i18n="fr.need2"]');
    if (fr && i) fr.textContent = t('dyn.fr_need2', [gb(i.data_bytes + 1.5e9)]);
    const folder = $('[data-i18n="fr.folder"]');
    if (folder && i) folder.textContent = i.root;
    const takes = $('[data-i18n="fr.takes"]');
    if (takes) takes.textContent = t('dyn.fr_takes');
  }

  function renderHome() {
    const i = S.info;
    if (!i || !i.installed) return;
    const p = $('[data-i18n="home.install_path"]');
    if (p) p.textContent = i.root;
    const sz = $('[data-i18n="home.install_size"]');
    if (sz) sz.textContent = t('dyn.home_size', [gb(i.data_bytes)]);
  }

  function render() {
    renderIso(); renderChecks(); renderSteps(); renderBuild(); renderSpace(); renderLog(); renderFirstRun(); renderHome();
  }

  // ---------- events from Rust ----------
  function onEvent(e) {
    if (e.check) {
      S.checks[e.check.id] = e.check;
      return;
    }
    if (e.stage === 'checksum') {
      S.hash.state = 'run'; S.hash.done = e.done; S.hash.total = e.total;
      return;
    }
    if (e.stage === 'preflight' || e.stage === 'validate') return;
    const st = STAGE_OF[e.stage];
    if (!st) return;
    if (e.stage === 'ffmpeg') {
      // the download shows on the audio row under its own name and does not count in the overall percentage
      S.ffmpegActive = !e.finished;
      if (e.total > 1) S.ff = { done: e.done, total: e.total };
      if (e.finished) S.ff = null;
      if (e.finished && e.total === 1 && e.done === 1) logLine(e.stage + ': ' + e.message);
      else if (!e.finished && e.total === 0 && e.message) logLine(e.stage + ': ' + e.message);
      return;
    }
    if (e.stage === 'audio' && e.finished) S.ffmpegActive = false;
    const rec = S.stage[st.id] || (S.stage[st.id] = { done: 0, total: 0, finished: false });
    if (e.total > 0 && e.stage !== 'stamp') { rec.done = e.done; rec.total = e.total; }
    if (e.finished && e.stage === st.names[st.names.length - 1]) rec.finished = true;
    if (e.finished && e.stage === 'ffmpeg') { rec.done = 0; rec.total = 0; }
    if ((e.finished && e.total === 1 && e.done === 1) || (!e.finished && e.total === 0 && e.message)) logLine(e.stage + ': ' + e.message, /problem|fail|error/i.test(e.message) ? 'w' : '');
  }

  function onDone(d) {
    if (d.kind === 'validated') {
      S.validating = false;
      if (d.ok) {
        S.identity = d.identity; S.warnings = d.warnings || []; S.valErr = null;
        S.hash = { state: 'run', done: 0, total: 0, verified: null };
        logLine('validate: ' + t('dyn.log_disc_ok'));
      } else {
        S.identity = null; S.valErr = d.error;
        S.hash = { state: 'idle', done: 0, total: 0, verified: null };
        logLine('validate: ' + errText(d.error), 'w');
      }
    } else if (d.kind === 'hashed') {
      if (d.hash) { S.hash = { state: 'done', done: 1, total: 1, verified: d.hash.verified }; logLine('checksum: ' + t(d.hash.verified ? 'dyn.hash_ok' : 'dyn.hash_other'), d.hash.verified ? '' : 'w'); }
      else S.hash.state = 'error';
    } else if (d.kind === 'built') {
      S.building = false;
      if (d.ok) {
        S.built = true; S.buildErr = null; S.resumable = false;
        STAGES.forEach((st) => { const r = S.stage[st.id] || (S.stage[st.id] = {}); r.finished = true; r.done = r.done || 1; });
        if (d.dump) S.hash = Object.assign({}, S.hash, { state: 'done', verified: d.dump === 'verified' });
        logLine('install: ' + t('dyn.log_built', [d.files, gb(d.bytes), d.seconds]));
        if (d.audio_problems) logLine('audio: ' + t('dyn.log_problems', [d.audio_problems]), 'w');
        S.app.setInstalled(true);
      } else {
        S.buildErr = d.error; S.cancelled = d.cancelled; S.resumable = true;
        logLine('install: ' + (d.cancelled ? t('dyn.cancelled') : errText(d.error) + ' (' + (d.error.detail || '') + ')'), 'w');
        // a cancelled stage is not finished
      }
      refreshInfo();
    } else if (d.kind === 'verified') {
      const bad = d.missing + d.wrong_size;
      logLine('verify: ' + (bad ? t('dyn.verify_bad', [bad]) : t('dyn.verify_ok', [d.checked])), bad ? 'w' : '');
      S.app.toast(bad ? t('dyn.verify_bad', [bad]) : t('dyn.verify_ok', [d.checked]), !!bad);
      if (bad) { S.resumable = true; S.built = false; }
      refreshInfo();
    }
  }

  async function poll() {
    if (S.polling) return;
    S.polling = true;
    try {
      const p = await Bridge.installPoll(S.since);
      if (p) {
        S.since = p.next;
        p.events.forEach(onEvent);
        p.done.forEach(onDone);
        if (p.events.length || p.done.length) render();
      }
    } catch (e) {
      Bridge.log('error', 'install poll failed: ' + e);
    }
    S.polling = false;
  }

  async function pollNow() {
    while (S.polling) await new Promise((r) => setTimeout(r, 20));
    await poll();
  }

  function startPolling() {
    if (S.timer || !Bridge.available) return;
    S.timer = setInterval(poll, 200);
  }

  async function refreshInfo() {
    try { S.info = await Bridge.installInfo(); } catch (e) { Bridge.log('error', 'install_info failed: ' + e); }
    if (S.info && S.info.installed && !S.building) S.built = true;
    render();
  }

  // ---------- actions ----------
  async function chooseIso() {
    let path = null;
    try { path = await Bridge.pickIso(); } catch (e) { Bridge.log('error', 'file dialog failed: ' + e); }
    if (path) await validate(path);
  }

  async function validate(path) {
    S.iso = path; S.identity = null; S.valErr = null; S.checks = {}; S.validating = true; S.warnings = [];
    S.hash = { state: 'idle', done: 0, total: 0, verified: null };
    S.buildErr = null; S.cancelled = false;
    render();
    try {
      await Bridge.validateIso(path);
    } catch (e) {
      S.validating = false;
      S.valErr = e && e.code ? e : { code: 'io', detail: String(e) };
    }
    // the validation's events and results arrive through poll; one poll now so the checklist is complete at once
    await pollNow();
    S.validating = false;
    render();
  }

  async function start() {
    if (!S.iso || !S.identity) return;
    S.building = true; S.buildErr = null; S.cancelled = false; S.built = false; S.t0 = Date.now();
    S.stage = {}; S.ffmpegActive = false; S.ff = null; S.rate = null; S.log = [];
    logLine('install: ' + t('dyn.log_started'));
    render();
    try {
      await Bridge.startInstall(S.iso, null);
    } catch (e) {
      S.building = false; S.buildErr = e && e.code ? e : { code: 'io', detail: String(e) };
      logLine('install: ' + errText(S.buildErr), 'w');
      render();
    }
  }

  async function verify() {
    logLine('verify: ' + t('dyn.log_verifying'));
    try { await Bridge.verifyInstall(); } catch (e) { S.app.toast(errText(e), true); logLine('verify: ' + errText(e), 'w'); }
    await pollNow();
    render();
  }

  function init(app) {
    S.app = app;
    $('#ins-browse').addEventListener('click', chooseIso);
    $('#ins-start').addEventListener('click', start);
    $('#ins-cancel').addEventListener('click', () => { Bridge.cancelInstall(); $('#ins-btn-note').textContent = t('dyn.cancelling'); });
    $('#ins-verify').addEventListener('click', verify);
    render();
    refreshInfo().then(() => {
      // a build or validation that was running when the page loaded (reload) is picked up again
      if (S.info && S.info.building) { S.building = true; S.t0 = Date.now(); }
      render();
    });
    startPolling();
  }

  // Test hook (RUSTY_MCLA_INSTALL_TEST): drives the page the way a user would - validate, press Start, optionally Cancel and Resume -
  // and returns what the page showed. The launcher writes it to the self-test report and exits.
  async function test(iso, withCancel) {
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    const rep = { iso, samples: [], phases: [] };
    const snap = (tag) => ({
      tag, at: Math.round((Date.now() - t00) / 1000), pct: $('#ins-pct').textContent, left: $('#ins-left').textContent,
      stages: $$('#ins-stages > div').map((w) => $('.nm', w).textContent + ': ' + ($('.st-r', w).textContent || '-')),
      steps: $$('.stepper .st').map((e) => e.className.replace('st ', '') + ' ' + $('[data-step-d]', e).textContent),
      btn: $('#ins-start').hidden ? 'cancel' : $('#ins-start').textContent + (($('#ins-start').disabled) ? ' (disabled)' : ''),
      note: $('#ins-btn-note').textContent, log: $$('#ins-log > div').map((d) => d.textContent),
    });
    const t00 = Date.now();
    S.app.setInstalled(false);
    await validate(iso);
    rep.validate = { identity: S.identity, error: S.valErr, rows: ROWS.map((r) => rowStatus(r).status + (rowStatus(r).text ? ' [' + rowStatus(r).text + ']' : '')), checks: Object.keys(S.checks).map((k) => k + ':' + S.checks[k].status), snap: snap('validated'),
      // does the content of the disc card stay inside the card? (slack = free pixels above the card's bottom padding)
      fit: (() => {
        // the page is not the one on screen during the test: show it for the measurement, then put it back
        const pg = $('[data-page="install"]'); const was = pg.hidden; pg.hidden = false;
        const card = $('#ins-err').closest('.card'); const cb = card.getBoundingClientRect();
        const last = [...card.children].filter((e) => e.offsetParent !== null).pop(); const lb = last.getBoundingClientRect();
        const out = { cardBottom: Math.round(cb.bottom), contentBottom: Math.round(lb.bottom), slack: Math.round(cb.bottom - 20 - lb.bottom), errVisible: !$('#ins-err').hidden, noteVisible: $('#ins-note').offsetParent !== null, rows: $$('#ins-checks > div').length };
        pg.hidden = was;
        return out;
      })() };
    await sleep(1500);
    rep.hashPartial = { state: S.hash.state, done: S.hash.done, total: S.hash.total };
    if (!S.identity) return rep;
    $('#ins-start').click();
    let cancelled = false, last = 0;
    for (let i = 0; i < 900; i++) {
      await sleep(1000);
      if (i % 5 === 0) rep.samples.push(snap('run'));
      const a = S.stage.audio;
      if (withCancel && !cancelled && !S.ffmpegActive && a && a.total && a.done > a.total * 0.3) {
        cancelled = true;
        rep.cancelAt = snap('before cancel');
        $('#ins-cancel').click();
        for (let j = 0; j < 120 && S.building; j++) await sleep(500);
        rep.afterCancel = snap('after cancel');
        rep.afterCancel.buildErr = S.buildErr; rep.afterCancel.cancelled = S.cancelled;
        // Resume
        $('#ins-start').click();
        rep.resumeAt = Date.now() - t00;
      }
      if (!S.building && (S.built || (S.buildErr && !S.cancelled))) break;
      if (!S.building && i > 3 && !S.built && !S.buildErr) last++;
      if (last > 5) break;
    }
    rep.final = snap('final');
    rep.final.built = S.built; rep.final.buildErr = S.buildErr; rep.final.hash = S.hash;
    await refreshInfo();
    rep.info = S.info;
    rep.seconds = Math.round((Date.now() - t00) / 1000);
    if (S.built) { await verify(); rep.verifyLog = S.log.slice(-2); }
    return rep;
  }

  window.Install = { init, render, refreshInfo, test, state: S };
})();
