'use strict';
// RUZINA Web Remote: polls /api/status, sends taps and faders to /api/control
// and loads from the paged /api/library (ruzina-web). Every mutation carries
// X-DDJ-Control; faders carry this page's client token and a sequence, so a
// late request can never move a fader backwards.

const CLIENT = crypto.getRandomValues(new Uint32Array(1))[0] >>> 0;
let sequence = 0;
let applied = null;          // last applied status stamp
let libraryRevision = null;  // catalog revision the list came from
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

function renderDeck(deck, data, pfl, volume) {
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
  const fraction = data.duration_ms ? Math.min(1, data.position_ms / data.duration_ms) : 0;
  $('.fill', field('bar')).style.width = `${(fraction * 100).toFixed(2)}%`;
  const play = $('[data-action="play_pause"]', card);
  play.textContent = data.playing ? 'PAUSE' : 'PLAY';
  play.classList.toggle('active', data.playing);
  $('[data-action="sync"]', card).classList.toggle('active', data.sync_enabled);
  $('[data-action="pfl"]', card).classList.toggle('active', pfl);
  for (const button of card.querySelectorAll('[data-action]')) {
    if (button.dataset.action !== 'pfl') button.disabled = !loaded;
  }
  const slider = $('[data-continuous="volume"]', card);
  if (!dragging.has(slider)) slider.value = volume;
}

async function pollStatus() {
  try {
    const response = await fetch('/api/status', { cache: 'no-store' });
    if (!response.ok) throw new Error(String(response.status));
    const status = await response.json();
    $('#conn').classList.add('ok');
    if (supersedes(status.stamp, applied)) {
      applied = status.stamp;
      renderDeck(1, status.deck1, status.mixer.pfl1, status.mixer.volume1);
      renderDeck(2, status.deck2, status.mixer.pfl2, status.mixer.volume2);
      const crossfader = $('#crossfader');
      if (!dragging.has(crossfader)) crossfader.value = status.mixer.crossfader;
      if (status.stamp.library_revision !== libraryRevision) {
        libraryRevision = status.stamp.library_revision;
        loadLibrary();
      }
    }
  } catch (error) {
    $('#conn').classList.remove('ok');
  }
  setTimeout(pollStatus, 300);
}

async function loadLibrary() {
  if (libraryRevision === null) {
    tracks = [];
    $('#library-info').textContent = 'No media connected';
    renderLibrary();
    return;
  }
  const revision = libraryRevision;
  const collected = [];
  let offset = 0;
  try {
    while (offset !== null) {
      const response = await fetch(`/api/library?offset=${offset}`, { cache: 'no-store' });
      if (!response.ok) throw new Error(await response.text());
      const page = await response.json();
      if (page.generation !== revision) return; // a newer revision reloads
      collected.push(...page.tracks);
      offset = page.next;
    }
  } catch (error) {
    $('#library-info').textContent = 'Library unavailable';
    return;
  }
  tracks = collected;
  $('#library-info').textContent = `${tracks.length} tracks`;
  renderLibrary();
}

function renderLibrary() {
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
        const params = new URLSearchParams({ track_key: track.track_key, generation: libraryRevision, deck });
        mutate(`/api/load?${params}`).then((ok) => { if (ok) message(''); });
      });
      row.append(button);
    }
    list.append(row);
  }
}

function bindControls() {
  for (const card of document.querySelectorAll('.deck')) {
    const deck = card.dataset.deck;
    for (const button of card.querySelectorAll('[data-action]')) {
      button.addEventListener('click', () => control(deck, button.dataset.action));
    }
    for (const slider of card.querySelectorAll('[data-continuous]')) {
      const action = slider.dataset.continuous;
      slider.addEventListener('pointerdown', () => dragging.add(slider));
      slider.addEventListener('pointerup', () => dragging.delete(slider));
      slider.addEventListener('input', () => sendContinuous(`${action}${deck}`, deck, action, slider.value));
    }
    const bar = $('[data-field="bar"]', card);
    bar.addEventListener('click', (event) => {
      const duration = Number(card.dataset.duration) || 0;
      const rect = bar.getBoundingClientRect();
      const fraction = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
      if (duration) control(deck, 'seek', Math.round(fraction * duration));
    });
  }
  const crossfader = $('#crossfader');
  crossfader.addEventListener('pointerdown', () => dragging.add(crossfader));
  crossfader.addEventListener('pointerup', () => dragging.delete(crossfader));
  crossfader.addEventListener('input', () => sendContinuous('crossfader', null, 'crossfader', crossfader.value));
  $('#search').addEventListener('input', renderLibrary);
}

bindControls();
pollStatus();
