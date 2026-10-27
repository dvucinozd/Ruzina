'use strict';
// RUZINA Web Remote: polls /api/status, sends taps, loops and faders (decks
// and mixer) to /api/control and loads from the paged /api/library
// (ruzina-web). Every mutation carries
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
const dragging = new Set();  // sliders the user is moving now

const $ = (selector, root = document) => root.querySelector(selector);
const deckCard = (deck) => $(`.deck[data-deck="${deck}"]`);

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

// Show a fader's reported value unless the user is moving it.
function showValue(slider, value) {
  if (slider && value !== undefined && !dragging.has(slider)) slider.value = value;
}

function renderDeck(deck, data, mixer) {
  const card = deckCard(deck);
  const field = (name) => $(`[data-field="${name}"]`, card);
  const loaded = data.state_text !== 'IDLE';
  card.dataset.duration = data.duration_ms;
  field('title').textContent = loaded ? data.title : 'No track loaded';
  field('artist').textContent = data.artist || '';
  field('bpm').textContent = loaded ? `${data.bpm} BPM` : '— BPM';
  field('time').textContent = `${clock(data.position_ms)} / ${clock(data.duration_ms)}`;
  field('state').textContent = data.state_text;
  const pitch = Number(data.pitch_percent);
  field('pitch').textContent = `${pitch >= 0 ? '+' : ''}${pitch.toFixed(2)}%`;
  showValue($('[data-continuous="pitch"]', card), data.pitch_raw);
  const fraction = data.duration_ms ? Math.min(1, data.position_ms / data.duration_ms) : 0;
  $('.fill', field('bar')).style.width = `${(fraction * 100).toFixed(2)}%`;
  const play = $('[data-action="play_pause"]', card);
  play.textContent = data.playing ? 'PAUSE' : 'PLAY';
  play.classList.toggle('active', data.playing);
  $('[data-action="sync"]', card).classList.toggle('active', data.sync_enabled);
  $('[data-action="pfl"]', card).classList.toggle('active', mixer[`pfl${deck}`]);
  for (const button of card.querySelectorAll('[data-action]')) {
    if (button.dataset.action !== 'pfl') button.disabled = !loaded;
  }
  for (const action of ['volume', 'eq_low', 'eq_mid', 'eq_high', 'filter']) {
    const key = action === 'volume' || action === 'filter' ? `${action}${deck}` : action.replace('eq_', `eq${deck}_`);
    showValue($(`[data-continuous="${action}"]`, card), mixer[key]);
  }
}

function renderMixer(mixer) {
  const section = $('.mixer');
  for (const action of ['crossfader', 'master_volume', 'headphone_mix']) {
    showValue($(`[data-continuous="${action}"]`, section), mixer[action]);
  }
  $('[data-action="smart_cfx"]', section).classList.toggle('active', mixer.smart_cfx);
  $('[data-action="smart_fader"]', section).classList.toggle('active', mixer.smart_fader);
}

async function pollStatus() {
  try {
    const response = await fetch('/api/status', { cache: 'no-store' });
    if (!response.ok) throw new Error(String(response.status));
    const status = await response.json();
    $('#conn').classList.add('ok');
    if (supersedes(status.stamp, applied)) {
      applied = status.stamp;
      renderDeck(1, status.deck1, status.mixer);
      renderDeck(2, status.deck2, status.mixer);
      renderMixer(status.mixer);
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
    $('#conn').classList.remove('ok');
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

function bindControls() {
  // Deck controls carry their card's deck; mixer controls carry none.
  const deckOf = (element) => element.closest('.deck')?.dataset.deck || null;
  for (const button of document.querySelectorAll('[data-action]')) {
    button.addEventListener('click', () => control(deckOf(button), button.dataset.action));
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
  }
  for (const card of document.querySelectorAll('.deck')) {
    const deck = card.dataset.deck;
    const bar = $('[data-field="bar"]', card);
    bar.addEventListener('click', (event) => {
      const duration = Number(card.dataset.duration) || 0;
      const rect = bar.getBoundingClientRect();
      const fraction = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
      if (duration) control(deck, 'seek', Math.round(fraction * duration));
    });
  }
  $('#search').addEventListener('input', renderLibrary);
}

bindControls();
pollStatus();
