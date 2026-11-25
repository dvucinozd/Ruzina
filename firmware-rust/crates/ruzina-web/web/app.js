'use strict';
// RUZINA Web Remote: polls /api/status, sends taps, pads, loops and faders (decks
// and mixer) to /api/control, draws each deck's overview from /api/wave and loads
// from the paged /api/library (ruzina-web). Every mutation carries
// X-DDJ-Control; faders carry this page's client token and a sequence, so a
// late request can never move a fader backwards.

const CLIENT = crypto.getRandomValues(new Uint32Array(1))[0] >>> 0;
let sequence = 0;
let applied = null;          // last applied status stamp
let desiredIdentity = null;
let loadedSnapshot = null;
let libraryRequestToken = 0;
let libraryInFlight = false;
let retryAt = 0;
let retryAttempt = 0;
let tracks = [];
let latency = null;          // smoothed status round trip, ms
const dragging = new Set();  // sliders the user is moving now
const painters = new Map();  // a knob's input -> redraws its dial
// Per deck: the last status, when it came, and the overview it shows.
const decks = { 1: {}, 2: {} };

const $ = (selector, root = document) => root.querySelector(selector);
const deckCard = (deck) => $(`.deck[data-deck="${deck}"]`);
const waveOf = (deck) => $(`.wave[data-deck="${deck}"]`);
const padsOf = (deck) => $(`.cues[data-deck="${deck}"]`);
// A deck's channel strip in the mixer: its PFL, meter, volume, EQ and filter.
const channel = (deck) => $(`.channel[data-deck="${deck}"]`);

function message(text) {
  $('#message').textContent = text || '';
}

async function mutate(url) {
  try {
    const response = await fetch(url, { method: 'POST', headers: { 'X-DDJ-Control': '1' } });
    const text = await response.text();
    if (!response.ok) message(text || `Request failed (${response.status})`);
    return response.ok;
  } catch (error) {
    message('Deck unreachable');
    return false;
  }
}

function control(deck, action, value) {
  const params = new URLSearchParams({ action });
  if (deck) params.set('deck', deck);
  if (value !== undefined) params.set('value', value);
  return mutate(`/api/control?${params}`);
}

// Latest value per control, at most one request in flight per ~90 ms.
const pending = new Map();
function sendContinuous(key, deck, action, value) {
  const entry = pending.get(key) || { timer: null, value };
  entry.value = value;
  if (!entry.timer) {
    entry.timer = setTimeout(() => {
      entry.timer = null;
      sequence = (sequence + 1) >>> 0;
      const params = new URLSearchParams({ action, value: entry.value, client: CLIENT, seq: sequence });
      if (deck) params.set('deck', deck);
      mutate(`/api/control?${params}`);
    }, 90);
  }
  pending.set(key, entry);
}

function supersedes(stamp, previous) {
  if (!previous || stamp.boot !== previous.boot) return true;
  return stamp.sequence !== previous.sequence &&
    ((stamp.sequence - previous.sequence) >>> 0) < 0x80000000;
}

function clock(ms) {
  const seconds = Math.floor((ms || 0) / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}

const percent = (value) => Math.round((Number(value) || 0) * 100 / 16383);

// Show a fader's reported value unless the user is moving it.
function showValue(slider, value) {
  if (slider && value !== undefined && !dragging.has(slider)) {
    slider.value = value;
    painters.get(slider)?.();
  }
}

// The playing tempo with two decimals: the status's hundredths, else its whole BPM.
function tempoX100(data) {
  if (Number.isFinite(data.tempo_x100) && data.tempo_x100 > 0) return data.tempo_x100;
  return (Number(data.bpm) || 0) * 100;
}

function renderDeck(deck, data, mixer) {
  const card = deckCard(deck);
  const wave = waveOf(deck);
  const pads = padsOf(deck);
  const strip = channel(deck);
  const field = (name, root = card) => $(`[data-field="${name}"]`, root);
  const loaded = data.state_text !== 'IDLE';
  decks[deck].data = data;
  decks[deck].at = Date.now();
  wave.dataset.duration = data.duration_ms;
  card.classList.toggle('playing', !!data.playing);
  const title = loaded ? (data.title || 'Untitled') : 'No track loaded';
  field('title').textContent = title;
  field('artist').textContent = data.artist || '';
  field('wtitle', wave).textContent = loaded ? title : 'No track';
  $(`#pill-${deck}`).textContent = loaded ? title : '—';
  const tempo = tempoX100(data);
  field('bpm').textContent = loaded && tempo ? String(Math.floor(tempo / 100)) : '—';
  field('bpm-fraction').textContent = loaded && tempo ? `.${String(tempo % 100).padStart(2, '0')}` : '';
  field('state').textContent = { READY: 'PAUSED', IDLE: 'EMPTY' }[data.state_text] || data.state_text;
  field('master').classList.toggle('on', !!data.sync_master);
  const pitch = Number(data.pitch_percent) || 0;
  const signed = `${pitch >= 0 ? '+' : ''}${pitch.toFixed(2)}%`;
  field('pitch').textContent = signed;
  field('tempo').textContent = pitch === 0 ? 'CENTER (0%)' : signed;
  showValue($('[data-continuous="pitch"]', card), data.pitch_raw);
  $('[data-action="sync"]', card).classList.toggle('active', !!data.sync_enabled);
  $('[data-action="sync"]', card).disabled = !loaded;
  // The pads: PLAY shows what a tap does next, and lit Hot Cues hold a cue.
  const play = $('[data-action="play_pause"]', pads);
  play.classList.toggle('active', !!data.playing);
  field('play', pads).textContent = data.playing ? '❚❚ PAUSE' : '▶ PLAY';
  field('play-state', pads).textContent = data.playing ? 'ACTIVE' : (loaded ? 'READY' : 'NO TRACK');
  for (const button of pads.querySelectorAll('[data-action]')) {
    button.disabled = !loaded;
    if (button.dataset.action === 'hot_cue') {
      button.classList.toggle('set', loaded && ((data.hot_cues || 0) >> (Number(button.dataset.value) - 1) & 1) === 1);
    }
  }
  $('[data-action="pfl"]', strip).classList.toggle('active', !!mixer[`pfl${deck}`]);
  const lit = Math.max(0, Math.min(10, Number(data.level) || 0));
  field('meter', strip).querySelectorAll('i').forEach((segment, index) => segment.classList.toggle('lit', index < lit));
  for (const action of ['volume', 'eq_low', 'eq_mid', 'eq_high', 'filter']) {
    const key = action === 'volume' || action === 'filter' ? `${action}${deck}` : action.replace('eq_', `eq${deck}_`);
    showValue($(`[data-continuous="${action}"]`, strip), mixer[key]);
  }
  drawWave(deck);
}

function renderMixer(mixer, status) {
  const section = $('.mixer');
  for (const action of ['master_volume', 'headphone_mix']) {
    showValue($(`[data-continuous="${action}"]`, section), mixer[action]);
  }
  showValue($('[data-continuous="crossfader"]'), mixer.crossfader);
  $('#master-value').textContent = `${percent(mixer.master_volume)}%`;
  const master = percent(mixer.headphone_mix);
  $('#phones-value').textContent = `${100 - master}/${master}`;
  $('[data-action="smart_cfx"]', section).classList.toggle('active', !!mixer.smart_cfx);
  $('[data-action="smart_fader"]', section).classList.toggle('active', !!mixer.smart_fader);
  // The master clock: the sync master's tempo, else a playing deck's, else deck 1's.
  const pick = [status.deck1, status.deck2];
  const lead = pick.find((d) => d && d.sync_master) || pick.find((d) => d && d.playing) ||
    pick.find((d) => d && d.state_text && d.state_text !== 'IDLE');
  const tempo = lead ? tempoX100(lead) : 0;
  $('#mclk').textContent = tempo ? (tempo / 100).toFixed(2) : '—';
  const synced = pick.some((d) => d && d.sync_enabled);
  $('#sync-lock').classList.toggle('on', synced);
  $('span', $('#sync-lock')).textContent = synced ? 'SYNC LOCK' : 'SYNC OFF';
}

// The overview: the peaks in the deck's colour, bright up to the playhead.
function drawWave(deck) {
  const wave = waveOf(deck);
  const canvas = $('[data-field="bar"]', wave);
  const state = decks[deck];
  const data = state.data || {};
  const duration = Number(data.duration_ms) || 0;
  let position = Number(data.position_ms) || 0;
  // Between polls a playing deck moves on at its tempo, for at most a second.
  if (data.playing && state.at) {
    position += Math.min(1000, Date.now() - state.at) * (1 + (Number(data.pitch_percent) || 0) / 100);
  }
  if (duration) position = Math.min(position, duration);
  const fraction = duration ? position / duration : 0;
  $('[data-field="time"]', wave).textContent = clock(position);
  $('[data-field="length"]', wave).textContent = clock(duration);
  $('[data-field="remain"]', wave).textContent = `-${clock(Math.max(0, duration - position))}`;
  $('[data-field="progress"]', wave).style.width = `${(fraction * 100).toFixed(2)}%`;
  canvas.ariaValueNow = String(Math.round(fraction * 100));
  const context = canvas.getContext ? canvas.getContext('2d') : null;
  if (!context) return;
  const ratio = (typeof devicePixelRatio === 'number' && devicePixelRatio) || 1;
  const width = Math.max(1, Math.round(canvas.clientWidth * ratio));
  const height = Math.max(1, Math.round(canvas.clientHeight * ratio));
  if (canvas.width !== width || canvas.height !== height) { canvas.width = width; canvas.height = height; }
  const color = getComputedStyle(wave).getPropertyValue('--deck').trim() || '#ff5f1f';
  context.clearRect(0, 0, width, height);
  const middle = height / 2;
  const peaks = state.peaks && data.track_key === state.key ? state.peaks : null;
  const head = Math.round(fraction * width);
  if (peaks && peaks.length) {
    const bar = Math.max(1, Math.round(2 * ratio));
    for (let x = 0; x < width; x += bar + 1) {
      const from = Math.floor(x * peaks.length / width);
      const to = Math.max(from + 1, Math.floor((x + bar) * peaks.length / width));
      let peak = 0;
      for (let i = from; i < to && i < peaks.length; i++) peak = Math.max(peak, peaks[i]);
      const half = Math.max(1, peak / 255 * (middle - 2));
      context.globalAlpha = x < head ? 1 : 0.38;
      context.fillStyle = color;
      context.fillRect(x, middle - half, bar, half * 2);
    }
  } else {
    context.globalAlpha = 0.5;
    context.fillStyle = color;
    context.fillRect(0, middle - ratio / 2, head, ratio);
    context.globalAlpha = 0.2;
    context.fillRect(head, middle - ratio / 2, width - head, ratio);
  }
  context.globalAlpha = 1;
  if (duration) {
    context.fillStyle = '#f5f7fa';
    context.fillRect(Math.min(width - 2 * ratio, head), 0, 2 * ratio, height);
  }
}

function animate() {
  for (const deck of [1, 2]) if (decks[deck].data && decks[deck].data.playing) drawWave(deck);
  requestAnimationFrame(animate);
}

// The overview of the track a deck has: fetched once per track, again after a miss.
async function loadWave(deck, key) {
  const state = decks[deck];
  if (state.waveInFlight || Date.now() < (state.waveRetryAt || 0)) return;
  state.waveInFlight = true;
  try {
    const response = await fetch(`/api/wave?deck=${deck}`, { cache: 'no-store' });
    if (!response.ok) throw new Error(String(response.status));
    const body = await response.json();
    if (body.track_key !== key) throw new Error('Track changed');
    const hex = String(body.peaks || '');
    const peaks = new Uint8Array(hex.length >> 1);
    for (let i = 0; i < peaks.length; i++) peaks[i] = parseInt(hex.substr(i * 2, 2), 16);
    state.key = key;
    state.peaks = peaks;
    drawWave(deck);
  } catch (error) {
    state.waveRetryAt = Date.now() + 1000;
  } finally {
    state.waveInFlight = false;
  }
}

async function pollStatus() {
  const sent = Date.now();
  try {
    const response = await fetch('/api/status', { cache: 'no-store' });
    if (!response.ok) throw new Error(String(response.status));
    const status = await response.json();
    const trip = Date.now() - sent;
    latency = latency === null ? trip : latency * 0.8 + trip * 0.2;
    $('#latency').textContent = `${Math.round(latency)}ms`;
    $('.live').classList.add('ok');
    $('#live-text').textContent = 'LIVE';
    // The deck's name: which deck this page drives, when there are several.
    if (status.name && $('#deck-name').textContent !== status.name) {
      $('#deck-name').textContent = status.name;
      document.title = `${status.name} · RUZINA`;
    }
    if (supersedes(status.stamp, applied)) {
      applied = status.stamp;
      renderDeck(1, status.deck1, status.mixer);
      renderDeck(2, status.deck2, status.mixer);
      // Each deck's load generation goes with its seeks, so a seek meant for a track
      // the deck has since replaced is refused.
      for (const deck of [1, 2]) {
        const generation = (status.stamp.load_generations || [])[deck - 1];
        waveOf(deck).dataset.generation = generation === undefined ? '' : String(generation);
        const key = status[`deck${deck}`].track_key;
        if (Number.isInteger(key) && key > 0 && key !== decks[deck].key) loadWave(deck, key);
      }
      renderMixer(status.mixer, status);
      const identity = {boot: status.stamp.boot, revision: status.stamp.library_revision};
      if (!sameIdentity(identity, desiredIdentity)) {
        desiredIdentity = identity;
        libraryRequestToken++;
        libraryInFlight = false;
        loadedSnapshot = null;
        tracks = [];
        retryAttempt = 0;
        retryAt = 0;
        $('#library-info').textContent = identity.revision === null ? 'No media connected' : 'Loading library';
        renderLibrary();
      }
    }
    if (desiredIdentity?.revision !== null && desiredIdentity && !loadedSnapshot &&
        !libraryInFlight && Date.now() >= retryAt) loadLibrary();
  } catch (error) {
    $('.live').classList.remove('ok');
    $('#live-text').textContent = 'OFFLINE';
  }
  setTimeout(pollStatus, 300);
}

async function loadLibrary() {
  const identity = desiredIdentity;
  const token = ++libraryRequestToken;
  const current = () => token === libraryRequestToken && sameIdentity(identity, desiredIdentity);
  libraryInFlight = true;
  const collected = [];
  const seen = new Set();
  let offset = 0;
  try {
    while (offset !== null) {
      if (seen.has(offset)) throw new Error('Repeated library page');
      seen.add(offset);
      const response = await fetch(`/api/library?offset=${offset}`, { cache: 'no-store' });
      if (!current()) return;
      if (!response.ok) throw new Error(await response.text());
      const page = await response.json();
      if (!current()) return;
      if (page.generation !== identity.revision) throw new Error('Library changed');
      if (page.next !== null && (!Number.isInteger(page.next) || page.next <= offset)) throw new Error('Invalid library page');
      collected.push(...page.tracks);
      offset = page.next;
    }
  } catch (error) {
    if (!current()) return;
    $('#library-info').textContent = 'Library unavailable';
    retryAt = Date.now() + Math.min(5000, 1000 * 2 ** Math.min(retryAttempt++, 3));
    return;
  } finally {
    if (current()) libraryInFlight = false;
  }
  if (!current()) return;
  loadedSnapshot = {identity, tracks: collected};
  retryAttempt = 0;
  tracks = collected;
  $('#library-info').textContent = `${tracks.length} tracks`;
  renderLibrary();
}

function sameIdentity(a, b) {
  return !!a && !!b && a.boot === b.boot && a.revision === b.revision;
}

function renderLibrary() {
  const snapshot = loadedSnapshot;
  const filter = $('#search').value.trim().toLowerCase();
  const list = $('#tracks');
  list.replaceChildren();
  for (const track of tracks) {
    if (filter && !`${track.title} ${track.artist}`.toLowerCase().includes(filter)) continue;
    const row = document.createElement('li');
    const name = document.createElement('div');
    name.className = 'name';
    const title = document.createElement('span');
    title.textContent = track.title;
    const who = document.createElement('span');
    who.className = 'who';
    who.textContent = track.artist;
    name.append(title, who);
    const meta = document.createElement('span');
    meta.className = 'num';
    meta.textContent = `${track.bpm} · ${clock(track.duration_ms)}`;
    row.append(name, meta);
    for (const deck of [1, 2]) {
      const button = document.createElement('button');
      button.textContent = `LOAD ${deck}`;
      button.addEventListener('click', () => {
        if (!snapshot || snapshot !== loadedSnapshot || !sameIdentity(snapshot.identity, desiredIdentity)) return;
        const params = new URLSearchParams({ track_key: track.track_key, generation: snapshot.identity.revision, deck });
        mutate(`/api/load?${params}`).then((ok) => { if (ok) message(''); });
      });
      row.append(button);
    }
    list.append(row);
  }
}

// A knob: its hidden range input holds the value; a vertical drag on the dial turns it.
function bindKnob(input, send) {
  const dial = $('.dial', input.closest('.knob'));
  const paint = () => {
    const turn = (Number(input.value) / 16383 - 0.5) * 270;
    dial.style.setProperty('--turn', `${turn.toFixed(1)}deg`);
  };
  painters.set(input, paint);
  paint();
  let start = null;
  dial.addEventListener('pointerdown', (event) => {
    start = { y: event.clientY, value: Number(input.value) };
    dragging.add(input);
    dial.setPointerCapture(event.pointerId);
    event.preventDefault();
  });
  dial.addEventListener('pointermove', (event) => {
    if (!start) return;
    const value = Math.round(Math.min(16383, Math.max(0, start.value + (start.y - event.clientY) * 16383 / 160)));
    if (value !== Number(input.value)) { input.value = value; paint(); send(); }
  });
  const end = () => { start = null; dragging.delete(input); };
  dial.addEventListener('pointerup', end);
  dial.addEventListener('pointercancel', end);
  dial.addEventListener('dblclick', () => { input.value = input.defaultValue; paint(); send(); });
  input.addEventListener('input', paint);
}

// SETUP: the deck's settings, read while the view is open and changed one at a time;
// each change answers with the settings as they then are.
let settings = null;
let settingsTimer = null;
function uptime(seconds) {
  const s = Number(seconds) || 0;
  const h = Math.floor(s / 3600), m = Math.floor(s / 60) % 60;
  return h ? `${h} h ${m} min` : `${m} min ${s % 60} s`;
}
function renderSettings(data) {
  settings = data;
  const mark = (selector, value) => {
    for (const button of document.querySelectorAll(`${selector} button`)) {
      button.classList.toggle('on', button.dataset.value === String(value));
    }
  };
  mark('[data-setting="load_lock"]', data.load_lock ? 1 : 0);
  mark('[data-setting="jog"][data-jog="1"]', data.jog[0]);
  mark('[data-setting="jog"][data-jog="2"]', data.jog[1]);
  mark('[data-setting="trim"]', data.trim_db);
  mark('[data-setting="screensaver"]', data.screensaver_min);
  $('#brightness').textContent = `${data.brightness.percent} %`;
  const info = data.info || {};
  $('#info-network').textContent = info.network || '—';
  $('#info-firmware').textContent = info.firmware || '—';
  $('#info-built').textContent = info.built || '—';
  $('#info-controller').textContent = info.controller || 'none';
  $('#info-media').textContent = { ready: 'READY', reading: 'READING…', removed: 'REMOVED' }[info.media] || '—';
  $('#info-tracks').textContent = String(info.tracks ?? '—');
  $('#info-uptime').textContent = uptime(info.uptime_s);
  renderProfiles(data.profiles || []);
}
// The controller profiles the deck carries: the attached controller's marked ACTIVE, and
// those tried on this deck's hardware TESTED.
function renderProfiles(profiles) {
  const list = $('#profiles');
  list.replaceChildren();
  for (const profile of profiles) {
    const row = document.createElement('li');
    row.className = profile.active ? 'active' : '';
    const name = document.createElement('b');
    name.textContent = profile.name;
    const id = document.createElement('code');
    id.textContent = profile.id;
    row.append(name, id);
    for (const [shown, text, on] of [[profile.active, 'ACTIVE', true], [profile.tested, 'TESTED', false]]) {
      if (!shown) continue;
      const tag = document.createElement('span');
      tag.className = on ? 'tag on' : 'tag';
      tag.textContent = text;
      row.append(tag);
    }
    list.append(row);
  }
}
// The query that changes one setting; null for a brightness already at its end.
function settingParams(name, value, jog) {
  const params = new URLSearchParams({ name });
  if (name === 'brightness') {
    if (!settings) return null;
    const b = settings.brightness;
    const next = Math.min(b.max, Math.max(b.min, b.percent + Number(value) * b.step));
    if (next === b.percent) return null;
    params.set('value', next);
  } else {
    params.set('value', value);
  }
  if (jog) params.set('deck', jog);
  return params;
}
// A new password for the deck's network: checked here as the deck checks it, sent in the
// body only, and never kept by the page.
function passwordProblem(password) {
  if (/^[0-9a-fA-F]{64}$/.test(password)) return null;
  if (password.length < 8 || password.length > 63) return 'Password must be 8 to 63 characters';
  if (!/^[\x20-\x7e]*$/.test(password)) return 'Use letters, digits, spaces and common symbols only';
  if (/["\\]/.test(password)) return 'Quotes and backslashes cannot be used';
  return null;
}
async function changePassword(password) {
  const problem = passwordProblem(password);
  if (problem) { message(problem); return false; }
  try {
    const response = await fetch('/api/network', {
      method: 'POST',
      headers: { 'X-DDJ-Control': '1', 'Content-Type': 'application/json' },
      body: JSON.stringify({ password }),
    });
    message(await response.text() || `Request failed (${response.status})`);
    return response.ok;
  } catch (error) {
    message('Deck unreachable');
    return false;
  }
}
async function loadSettings() {
  try {
    const response = await fetch('/api/settings', { cache: 'no-store' });
    if (response.ok) renderSettings(await response.json());
    else message('Settings unavailable');
  } catch (error) {
    message('Deck unreachable');
  }
}
async function changeSetting(params) {
  if (!params) return;
  try {
    const response = await fetch(`/api/settings?${params}`, { method: 'POST', headers: { 'X-DDJ-Control': '1' } });
    if (response.ok) { message(''); renderSettings(await response.json()); }
    else message(await response.text() || `Request failed (${response.status})`);
  } catch (error) {
    message('Deck unreachable');
  }
}

function setView(view) {
  $('.console').dataset.view = view;
  for (const tab of document.querySelectorAll('.tabs button')) tab.classList.toggle('on', tab.dataset.view === view);
  try { localStorage.setItem('ruzina-view', view); } catch (error) { /* private mode */ }
  if (settingsTimer) { clearInterval(settingsTimer); settingsTimer = null; }
  if (view === 'setup') {
    loadSettings();
    settingsTimer = setInterval(loadSettings, 2000);
    return;
  }
  for (const deck of [1, 2]) drawWave(deck);
}

function bindControls() {
  // Deck controls and channel strips carry their deck; the master section carries none.
  const deckOf = (element) => element.closest('[data-deck]')?.dataset.deck || null;
  for (const button of document.querySelectorAll('[data-action]')) {
    button.addEventListener('click', () => control(deckOf(button), button.dataset.action, button.dataset.value));
  }
  for (const slider of document.querySelectorAll('[data-continuous]')) {
    const deck = deckOf(slider);
    const action = slider.dataset.continuous;
    const send = () => sendContinuous(`${action}${deck || ''}`, deck, action, slider.value);
    slider.addEventListener('pointerdown', () => dragging.add(slider));
    slider.addEventListener('pointerup', () => dragging.delete(slider));
    slider.addEventListener('change', () => dragging.delete(slider));
    slider.addEventListener('input', send);
    slider.addEventListener('dblclick', () => { slider.value = slider.defaultValue; send(); });
    if (slider.closest('.knob')) bindKnob(slider, send);
  }
  for (const wave of document.querySelectorAll('.wave')) {
    const deck = wave.dataset.deck;
    const bar = $('[data-field="bar"]', wave);
    bar.addEventListener('click', (event) => {
      const duration = Number(wave.dataset.duration) || 0;
      const rect = bar.getBoundingClientRect();
      const fraction = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
      if (!duration) return;
      const params = new URLSearchParams({ action: 'seek', deck, value: Math.round(fraction * duration) });
      if (wave.dataset.generation) params.set('generation', wave.dataset.generation);
      mutate(`/api/control?${params}`);
    });
  }
  for (const toggle of document.querySelectorAll('.eq-toggle')) {
    toggle.addEventListener('click', () => {
      const folded = toggle.closest('.channel').classList.toggle('folded');
      toggle.textContent = folded ? 'EQ ▸' : 'EQ ▾';
      toggle.ariaExpanded = String(!folded);
    });
  }
  for (const tab of document.querySelectorAll('.tabs button')) {
    tab.addEventListener('click', () => setView(tab.dataset.view));
  }
  for (const group of document.querySelectorAll('[data-setting]')) {
    for (const button of group.querySelectorAll('button')) {
      button.addEventListener('click', () =>
        changeSetting(settingParams(group.dataset.setting, button.dataset.value, group.dataset.jog)));
    }
  }
  for (const button of document.querySelectorAll('[data-brightness]')) {
    button.addEventListener('click', () => changeSetting(settingParams('brightness', button.dataset.brightness)));
  }
  $('#password-show').addEventListener('click', () => {
    const input = $('#password');
    input.type = input.type === 'password' ? 'text' : 'password';
    $('#password-show').textContent = input.type === 'password' ? 'SHOW' : 'HIDE';
  });
  $('#password-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    const input = $('#password');
    if (await changePassword(input.value)) input.value = '';
  });
  $('#to-library').addEventListener('click', () => { $('#library').open = true; });
  $('#search').addEventListener('input', renderLibrary);
  if (typeof window !== 'undefined') {
    window.addEventListener('resize', () => { for (const deck of [1, 2]) drawWave(deck); });
  }
  let view = 'dual';
  try { view = localStorage.getItem('ruzina-view') || 'dual'; } catch (error) { /* private mode */ }
  if (['1', '2', 'dual', 'setup'].includes(view) && view !== 'dual') setView(view);
}

bindControls();
pollStatus();
if (typeof requestAnimationFrame === 'function') requestAnimationFrame(animate);
