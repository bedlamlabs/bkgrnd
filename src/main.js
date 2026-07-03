const tauriCore = window.__TAURI__?.core;
const invoke = tauriCore?.invoke ?? (async () => {
  throw new Error('Tauri API unavailable');
});
const tauriShell = window.__TAURI__?.shell;

// Elements
const appContainer = document.getElementById('appContainer');
const playerPanel = document.getElementById('playerPanel');
const gridView = document.getElementById('gridView');
const offlineView = document.getElementById('offlineView');
const restartBtn = document.getElementById('restartBtn');

const urlInput = document.getElementById('urlInput');
const submitInputBtn = document.getElementById('submitInputBtn');
const errorEl = document.getElementById('error');
const loadingEl = document.getElementById('loading');
const loadingText = document.getElementById('loadingText');

const trackTitle = document.getElementById('trackTitle');
const trackSubtitle = document.getElementById('trackSubtitle');
const statusText = document.getElementById('statusText');
const progressFill = document.getElementById('progressFill');
const progressKnob = document.getElementById('progressKnob');
const elapsedTime = document.getElementById('elapsedTime');
const durationTime = document.getElementById('durationTime');

const bookmarkBtn = document.getElementById('bookmarkBtn');
const pauseBtn = document.getElementById('pauseBtn');
const stopBtn = document.getElementById('stopBtn');
const seekBackBtn = document.getElementById('seekBackBtn');
const seekFwdBtn = document.getElementById('seekFwdBtn');
const playlistBtn = document.getElementById('playlistBtn');
const pauseIcon = document.getElementById('pauseIcon');
const playIcon = document.getElementById('playIcon');

const microStatus = document.getElementById('microStatus');
const microStatusText = document.getElementById('microStatusText');

const compactNow = document.getElementById('compactNow');
const compactCover = document.getElementById('compactCover');
const compactTitle = document.getElementById('compactTitle');
const compactSub = document.getElementById('compactSub');
const compactRuntime = document.getElementById('compactRuntime');

const librarySection = document.getElementById('librarySection');
const historyList = document.getElementById('historyList');
const gridNote = document.getElementById('gridNote');
const searchResults = document.getElementById('searchResults');
const searchResultsList = document.getElementById('searchResultsList');
const clearSearchBtn = document.getElementById('clearSearchBtn');

// State
let currentState = 'idle';
let currentStatus = null;
let lastActiveUrl = null;
let gridOpen = true;
let lastTitle = null;
let failCount = 0;
let searchVisible = false;
let pendingPlayItem = null;
let playlistItems = [];
let savedUrls = new Set();

// View Management
function showGrid({ focusInput = false } = {}) {
  gridOpen = true;
  offlineView.classList.add('hidden');
  playerPanel.classList.add('hidden');
  gridView.classList.remove('hidden');
  renderCompactNow();
  resizeToContent();
  if (focusInput) {
    setTimeout(() => urlInput.focus(), 50);
  }
}

function showPlayer() {
  if (!currentStatus?.isPlaying && currentState !== 'connecting') {
    showGrid();
    return;
  }
  gridOpen = false;
  offlineView.classList.add('hidden');
  gridView.classList.add('hidden');
  playerPanel.classList.remove('hidden');
  resizeToContent();
}

function showOffline() {
  playerPanel.classList.add('hidden');
  gridView.classList.add('hidden');
  offlineView.classList.remove('hidden');
  resizeToContent();
}

function resizeWindow(height) {
  invoke('resize_window', { height }).catch(() => {});
}

function resizeToContent() {
  requestAnimationFrame(() => {
    const visibleSurface = [playerPanel, gridView, offlineView]
      .find((surface) => !surface.classList.contains('hidden'));
    const measuredHeight = visibleSurface?.offsetHeight || appContainer?.offsetHeight || 178;
    // Clamp: set_size bypasses the window's maxHeight constraint, and the
    // grid scrolls internally past this point.
    resizeWindow(Math.min(Math.max(measuredHeight, 178), 598) + 2);
  });
}

function setControlsEnabled(enabled) {
  bookmarkBtn.disabled = !enabled;
  pauseBtn.disabled = !enabled;
  stopBtn.disabled = !enabled;
  seekBackBtn.disabled = !enabled;
  seekFwdBtn.disabled = !enabled;
}

function setPendingPlayer(item) {
  currentState = 'connecting';
  pendingPlayItem = item;
  currentStatus = null;
  trackTitle.textContent = 'Connecting...';
  trackSubtitle.textContent = sourceLabel(item?.url || '');
  setMicroStatus('Connecting');
  setCoverImage(playerPanel, item?.thumbnail || '');
  setControlsEnabled(false);
  setProgress(null, null);
  showPlayer();
}

// Status Polling
async function fetchStatus() {
  try {
    const status = await invoke('get_status');
    failCount = 0;
    applyStatus(status);
    return status;
  } catch {
    failCount++;
    if (failCount >= 3) {
      showOffline();
    }
    return null;
  }
}

function applyStatus(status) {
  if (status?.isPlaying) {
    const shouldOpenPlayer = currentState === 'connecting' || !gridOpen;
    currentState = 'playing';
    currentStatus = status;
    updatePlayerSurface(status);
    renderCompactNow();
    setControlsEnabled(true);

    // Keep the grid's "Playing" pill in sync with the active stream.
    if (status.sourceUrl !== lastActiveUrl) {
      lastActiveUrl = status.sourceUrl;
      renderLibrary();
    }

    if (shouldOpenPlayer) {
      showPlayer();
    } else {
      showGrid();
    }

    const trayState = status.isPaused ? 'paused' : 'playing';
    invoke('set_tray_state', { state: trayState }).catch(() => {});

    if (status.title && status.title !== lastTitle && !status.isPaused) {
      lastTitle = status.title;
      fireNotification(status);
    }
    return;
  }

  if (currentState === 'connecting') {
    return;
  }

  if (currentState !== 'idle') {
    lastTitle = null;
    lastActiveUrl = null;
    invoke('set_tray_state', { state: 'idle' }).catch(() => {});
    fetchLibrary();
  }

  currentState = 'idle';
  currentStatus = null;
  setControlsEnabled(false);
  showGrid();
}

function updatePlayerSurface(status) {
  const title = status.title || 'Playing...';
  trackTitle.textContent = title;
  trackSubtitle.textContent = statusSubtitle(status);
  setMicroStatus(status.isPaused ? 'Paused' : 'Playing');

  setCoverImage(playerPanel, thumbnailForStatus(status));
  setProgress(status.position, status.duration);

  if (status.isPaused) {
    pauseIcon.classList.add('hidden');
    playIcon.classList.remove('hidden');
    pauseBtn.title = 'Play';
    pauseBtn.setAttribute('aria-label', 'Play');
  } else {
    pauseIcon.classList.remove('hidden');
    playIcon.classList.add('hidden');
    pauseBtn.title = 'Pause';
    pauseBtn.setAttribute('aria-label', 'Pause');
  }

  syncBookmarkButton(status.sourceUrl);
}

function statusSubtitle(status) {
  if (status.channel) {
    return status.channel;
  }
  if (status.queueLength > 1 && status.playlistTitle) {
    return `${status.playlistTitle} - ${status.queuePosition + 1} of ${status.queueLength}`;
  }
  if (status.queueLength > 1) {
    return `Track ${status.queuePosition + 1} of ${status.queueLength}`;
  }
  return '';
}

function renderCompactNow() {
  if (!currentStatus?.isPlaying) {
    compactNow.classList.add('hidden');
    return;
  }

  compactNow.classList.remove('hidden');
  compactTitle.textContent = currentStatus.title || 'Playing...';
  compactSub.textContent = statusSubtitle(currentStatus) || (currentStatus.isPaused ? 'Paused' : 'Playing');
  compactRuntime.textContent = formatDuration(currentStatus.duration) || fallbackMetaForUrl(currentStatus.sourceUrl) || '--:--';
  setCoverImage(compactCover, thumbnailForStatus(currentStatus));
}

function setProgress(position, duration) {
  const pos = numberOrNull(position);
  const dur = numberOrNull(duration);
  const pct = pos != null && dur != null && dur > 0
    ? Math.max(0, Math.min(100, (pos / dur) * 100))
    : 0;

  progressFill.style.width = `${pct}%`;
  progressKnob.style.left = `${pct}%`;
  elapsedTime.textContent = formatDuration(pos) || '0:00';
  durationTime.textContent = formatDuration(dur) || '--:--';
}

function setMicroStatus(text) {
  if (microStatusText) microStatusText.textContent = text || '';
}

// Library
async function fetchLibrary() {
  try {
    const doc = await invoke('get_playlists');
    playlistItems = flattenPlaylistDoc(doc);
  } catch {
    try {
      const history = await invoke('get_history');
      playlistItems = Array.isArray(history) ? history.map((item) => normalizeGridItem(item)) : [];
    } catch {
      playlistItems = [];
    }
  }

  savedUrls = new Set(playlistItems.map((item) => item.url).filter(Boolean));
  renderLibrary();
  if (currentStatus?.isPlaying) {
    syncBookmarkButton(currentStatus.sourceUrl);
    renderCompactNow();
  }
}

function flattenPlaylistDoc(doc) {
  const items = [];
  const playlists = Array.isArray(doc?.playlists) ? doc.playlists : [];

  for (const playlist of playlists) {
    const playlistItemsRaw = Array.isArray(playlist.items) ? playlist.items : [];
    for (const item of playlistItemsRaw) {
      items.push(normalizeGridItem({ ...item, playlistTitle: playlist.name }));
    }
  }

  const seen = new Set();
  return items.filter((item) => {
    if (!item.url || seen.has(item.url)) return false;
    seen.add(item.url);
    return true;
  });
}

function normalizeGridItem(raw, fallbackChannel = '') {
  const url = raw?.url || raw?.sourceUrl || '';
  return {
    title: raw?.title || url || 'Untitled',
    url,
    videoId: raw?.videoId || extractVideoId(url) || '',
    channel: raw?.channel || fallbackChannel || '',
    thumbnail: raw?.thumbnail || thumbnailFromUrl(url),
    duration: numberOrNull(raw?.duration),
    trackCount: raw?.trackCount ?? null,
    addedAt: raw?.addedAt || raw?.playedAt || '',
  };
}

function renderLibrary() {
  if (searchVisible) return;

  historyList.innerHTML = '';
  gridNote.textContent = playlistItems.length ? `${playlistItems.length} saved` : 'Playlist';

  if (!playlistItems.length) {
    historyList.innerHTML = '<div class="empty-grid">No saved streams yet. Search or paste a URL to start one.</div>';
    resizeToContent();
    return;
  }

  for (const item of playlistItems) {
    historyList.appendChild(renderTile(item, { saved: true }));
  }

  resizeToContent();
}

function renderTile(item, { saved = false, search = false } = {}) {
  const tile = document.createElement('div');
  tile.className = 'stream-tile';
  tile.setAttribute('role', 'button');
  tile.tabIndex = 0;
  tile.title = item.title || 'Play';

  const isActive = Boolean(currentStatus?.sourceUrl && item.url === currentStatus.sourceUrl);
  if (isActive) {
    tile.classList.add('active');
  }

  const thumb = document.createElement('div');
  thumb.className = 'stream-thumb';
  setCoverImage(thumb, item.thumbnail);
  tile.appendChild(thumb);

  if (isActive) {
    const pill = document.createElement('span');
    pill.className = 'active-pill';
    pill.innerHTML = '<span class="pulse-dot"></span> Playing';
    tile.appendChild(pill);
  }

  const copy = document.createElement('div');
  copy.className = 'tile-copy';

  const title = document.createElement('div');
  title.className = 'tile-title';
  title.textContent = item.title || 'Untitled';

  const ch = document.createElement('div');
  ch.className = 'tile-ch';
  ch.setAttribute('role', 'link');
  ch.tabIndex = 0;
  const meta = tileMetaLabel(item, { search });
  ch.textContent = [item.channel, meta].filter(Boolean).join(' · ') || item.channel || '';

  copy.appendChild(title);
  copy.appendChild(ch);
  tile.appendChild(copy);

  const playTarget = search ? playUrlForSearchItem(item) : item.url;
  const play = () => playUrl(playTarget, item);
  const openSrc = (event) => {
    event.stopPropagation();
    openExternalUrl(canonicalStreamUrl(item.url || playTarget));
  };
  ch.addEventListener('click', openSrc);
  ch.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      openSrc(event);
    }
  });
  tile.addEventListener('click', play);
  tile.addEventListener('keydown', (event) => {
    if (event.target.closest('.tile-ch')) return;
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      play();
    }
  });

  return tile;
}

function tileMetaLabel(item, { search } = {}) {
  const duration = formatDuration(item.duration);
  if (duration) return duration;

  if (search && item.trackCount) {
    return `${item.trackCount} tracks`;
  }

  return '';
}

function fallbackMetaForUrl(url) {
  const item = playlistItems.find((candidate) => candidate.url === url);
  if (!item) return '';
  return formatDuration(item.duration) || '';
}

async function openExternalUrl(url) {
  if (!url) return;
  try {
    if (tauriShell?.open) {
      await tauriShell.open(url);
      return;
    }
  } catch {}

  try {
    await invoke('plugin:shell|open', { path: url });
    return;
  } catch {}

  try {
    window.open(url, '_blank', 'noopener,noreferrer');
  } catch {}
}

function canonicalStreamUrl(url) {
  const videoId = extractVideoId(url);
  if (videoId) return `https://www.youtube.com/watch?v=${encodeURIComponent(videoId)}`;
  return url;
}

async function removeSavedItem(item) {
  if (!item?.url) return;
  try {
    const doc = await invoke('remove_playlist_item', { url: item.url });
    playlistItems = flattenPlaylistDoc(doc);
    savedUrls = new Set(playlistItems.map((entry) => entry.url).filter(Boolean));
    renderLibrary();
    if (currentStatus?.isPlaying) {
      syncBookmarkButton(currentStatus.sourceUrl);
      renderCompactNow();
    }
  } catch (err) {
    showError(errorMessage(err, 'Could not remove item'));
  }
}

async function saveCurrent(status = currentStatus, sourceItem = pendingPlayItem) {
  const url = status?.sourceUrl || sourceItem?.url || '';
  if (!url) return;

  const payload = {
    url,
    title: status?.title || sourceItem?.title || url,
    thumbnail: thumbnailForStatus(status) || sourceItem?.thumbnail || '',
    channel: sourceItem?.channel || status?.channel || '',
    duration: numberOrNull(sourceItem?.duration) ?? numberOrNull(status?.duration),
  };

  try {
    const doc = await invoke('save_playlist_item', payload);
    playlistItems = flattenPlaylistDoc(doc);
    savedUrls = new Set(playlistItems.map((item) => item.url).filter(Boolean));
    renderLibrary();
    syncBookmarkButton(url);
  } catch {}
}

async function toggleCurrentSaved() {
  const url = currentStatus?.sourceUrl;
  if (!url) return;

  if (savedUrls.has(url)) {
    await removeSavedItem({ url });
  } else {
    await saveCurrent(currentStatus, pendingPlayItem);
  }
}

function syncBookmarkButton(url) {
  const saved = Boolean(url && savedUrls.has(url));
  bookmarkBtn.classList.toggle('saved', saved);
  bookmarkBtn.classList.toggle('unsaved', !saved);
  bookmarkBtn.title = saved ? 'Remove from playlist' : 'Save to playlist';
  bookmarkBtn.setAttribute('aria-label', bookmarkBtn.title);
}

// Search
async function doSearch(query) {
  if (!query) return;

  hideError();
  loadingText.textContent = 'Searching...';
  loadingEl.classList.remove('hidden');
  searchResultsList.innerHTML = '';

  try {
    const results = await invoke('search', { query });
    showSearchResults(results);
  } catch (err) {
    showError(errorMessage(err, 'Search failed'));
  } finally {
    loadingEl.classList.add('hidden');
    resizeToContent();
  }
}

function showSearchResults(results) {
  searchResultsList.innerHTML = '';

  if (!results || results.length === 0) {
    searchResultsList.innerHTML = '<div class="empty-grid">No results found.</div>';
  } else {
    for (const raw of results) {
      const item = normalizeGridItem(raw, raw.channel || 'YouTube');
      searchResultsList.appendChild(renderTile(item, { search: true }));
    }
  }

  searchResults.classList.remove('hidden');
  librarySection.classList.add('hidden');
  searchVisible = true;
}

function clearSearch() {
  searchResults.classList.add('hidden');
  searchResultsList.innerHTML = '';
  librarySection.classList.remove('hidden');
  searchVisible = false;
  urlInput.value = '';
  renderLibrary();
}

// Playback Actions
async function playUrl(url, item = null) {
  if (!url) return;
  const playTarget = normalizePlayableUrl(url);
  const playItem = normalizeGridItem({ ...(item || {}), url: playTarget }, item?.channel || '');

  hideError();
  loadingText.textContent = 'Connecting...';
  loadingEl.classList.remove('hidden');
  setPendingPlayer(playItem);

  try {
    const status = await invoke('play', { url: playTarget });
    pendingPlayItem = playItem;
    applyStatus(status);
    await saveCurrent(status, playItem);
    urlInput.value = '';
    clearSearchAfterPlay();
  } catch (err) {
    currentState = 'idle';
    showError(errorMessage(err, 'Playback failed'));
    try {
      const status = await invoke('stop');
      applyStatus(status);
    } catch {
      currentStatus = null;
      showGrid();
    }
  } finally {
    loadingEl.classList.add('hidden');
    resizeToContent();
  }
}

function clearSearchAfterPlay() {
  searchResults.classList.add('hidden');
  searchResultsList.innerHTML = '';
  librarySection.classList.remove('hidden');
  searchVisible = false;
}

async function doPause() {
  try {
    const status = await invoke('toggle_pause');
    applyStatus(status);
  } catch {}
}

async function doStop() {
  try {
    currentState = 'stopping';
    const status = await invoke('stop');
    currentState = 'idle';
    applyStatus(status);
    await fetchLibrary();
    showGrid();
  } catch {}
}

async function doSeek(seconds) {
  try {
    const status = await invoke('seek_relative', { seconds });
    applyStatus(status);
  } catch {}
}

// Notifications
function fireNotification(status) {
  if (typeof Notification === 'undefined') return;
  if (Notification.permission !== 'granted') return;

  const opts = { body: status.title };
  const icon = thumbnailForStatus(status);
  if (icon) opts.icon = icon;

  try {
    new Notification('Now Playing', opts);
  } catch {}
}

// Input
function submitInput() {
  const input = urlInput.value.trim();
  if (!input) return;

  if (isUrl(input)) {
    clearSearchAfterPlay();
    playUrl(input, { url: input, title: input });
  } else {
    doSearch(input);
  }
}

urlInput.addEventListener('input', hideError);
urlInput.addEventListener('keydown', (event) => {
  if (event.key === 'Enter') {
    submitInput();
  }
  if (event.key === 'Escape' && searchVisible) {
    event.stopPropagation();
    clearSearch();
  }
});

submitInputBtn.addEventListener('click', submitInput);
clearSearchBtn.addEventListener('click', () => {
  clearSearch();
  urlInput.focus();
});

pauseBtn.addEventListener('click', doPause);
stopBtn.addEventListener('click', doStop);
seekBackBtn.addEventListener('click', () => doSeek(-10));
seekFwdBtn.addEventListener('click', () => doSeek(10));
playlistBtn.addEventListener('click', () => showGrid({ focusInput: false }));
bookmarkBtn.addEventListener('click', toggleCurrentSaved);
compactNow.addEventListener('click', showPlayer);

restartBtn.addEventListener('click', () => {
  invoke('plugin:process|restart').catch(() => {});
});

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    if (searchVisible) {
      clearSearch();
      return;
    }
    invoke('hide_window').catch(() => {});
  }
  if (event.key === 'q' && event.metaKey) {
    event.preventDefault();
    invoke('quit_app').catch(() => {});
  }
  if (event.key === ' ' && currentStatus?.isPlaying) {
    const tag = document.activeElement?.tagName;
    if (tag !== 'INPUT' && tag !== 'TEXTAREA') {
      event.preventDefault();
      doPause();
    }
  }
});

// Helpers
function isUrl(input) {
  return /^https?:\/\//i.test(input) ||
    /^spotify:(playlist|album|track):/i.test(input) ||
    /^(www\.)?(youtube\.com|youtu\.be|music\.youtube\.com|open\.spotify\.com)/i.test(input);
}

function normalizePlayableUrl(input) {
  const trimmed = String(input || '').trim();
  if (/^(www\.)?(youtube\.com|youtu\.be|music\.youtube\.com|open\.spotify\.com)/i.test(trimmed)) {
    return `https://${trimmed}`;
  }
  return trimmed;
}

function playUrlForSearchItem(item) {
  if (item.trackCount != null) return item.url;
  if (item.videoId) {
    return `https://www.youtube.com/watch?v=${item.videoId}&list=RD${item.videoId}`;
  }
  return item.url;
}

function sourceLabel(url) {
  try {
    const host = new URL(url).hostname.replace(/^www\./, '');
    if (host === 'open.spotify.com') return 'Converting Spotify to YouTube queue';
    return host;
  } catch {
    if (/^spotify:/i.test(url)) return 'Converting Spotify to YouTube queue';
    return '';
  }
}

function thumbnailForStatus(status) {
  if (!status) return '';
  if (status.thumbnail) return status.thumbnail;
  if (status.videoId) return thumbnailFromVideoId(status.videoId);
  return thumbnailFromUrl(status.sourceUrl);
}

function thumbnailFromUrl(url) {
  const id = extractVideoId(url);
  return id ? thumbnailFromVideoId(id) : '';
}

function thumbnailFromVideoId(videoId) {
  return videoId ? `https://i.ytimg.com/vi/${videoId}/mqdefault.jpg` : '';
}

const COVER_FALLBACK = 'linear-gradient(135deg, #28313a, #111318)';

function setCoverImage(el, src) {
  if (!src) {
    el.style.removeProperty('--cover-image');
    el.style.backgroundImage = COVER_FALLBACK;
    return;
  }

  if (el === playerPanel) {
    setPlayerCover(src);
    return;
  }

  el.style.backgroundImage = `url(${JSON.stringify(src)}), ${COVER_FALLBACK}`;
}

// The big player cover loads the crisp maxres thumbnail when available,
// showing the mqdefault immediately and upgrading once maxres confirms it
// exists (YouTube returns a 120px stub for videos without maxres).
let coverProbeToken = 0;
function setPlayerCover(src) {
  playerPanel.style.backgroundImage = COVER_FALLBACK;
  const setVar = (url) => playerPanel.style.setProperty('--cover-image', `url(${JSON.stringify(url)})`);
  setVar(src);
  const maxres = upgradeToMaxres(src);
  if (maxres === src) return;
  const token = ++coverProbeToken;
  const probe = new Image();
  probe.onload = () => {
    if (token === coverProbeToken && probe.naturalWidth > 200) setVar(maxres);
  };
  probe.onerror = () => {};
  probe.src = maxres;
}

function upgradeToMaxres(src) {
  return String(src || '').replace(/\/(?:default|mqdefault|hqdefault|sddefault)\.jpg(\?.*)?$/i, '/maxresdefault.jpg$1');
}

function extractVideoId(url) {
  if (!url) return null;
  const match = String(url).match(/(?:v=|youtu\.be\/|\/shorts\/)([a-zA-Z0-9_-]{11})/);
  return match ? match[1] : null;
}

function formatDuration(value) {
  const seconds = numberOrNull(value);
  if (seconds == null || seconds <= 0) return '';

  const rounded = Math.floor(seconds);
  const h = Math.floor(rounded / 3600);
  const m = Math.floor((rounded % 3600) / 60);
  const s = rounded % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  return `${m}:${String(s).padStart(2, '0')}`;
}

function numberOrNull(value) {
  if (value == null) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = String(str || '');
  return div.innerHTML;
}

function bookmarkSvg() {
  return `
    <svg viewBox="0 0 18 18" fill="currentColor" aria-hidden="true">
      <path d="M5 3.2h8a.8.8 0 0 1 .8.8v10.55L9 11.6l-4.8 2.95V4A.8.8 0 0 1 5 3.2Z"/>
    </svg>
  `;
}

function showError(message) {
  errorEl.textContent = message;
  errorEl.classList.remove('hidden');
}

function hideError() {
  errorEl.classList.add('hidden');
}

function errorMessage(err, fallback) {
  if (typeof err === 'string' && err.trim()) return err;
  if (err?.message) return err.message;
  return fallback;
}

// Initialization
if (typeof Notification !== 'undefined' && Notification.permission === 'default') {
  Notification.requestPermission().catch(() => {});
}

setControlsEnabled(false);
showGrid();
fetchLibrary();

fetchStatus().then(() => {
  if (!currentStatus?.isPlaying) {
    showGrid({ focusInput: true });
  }
});

setInterval(fetchStatus, 1500);

document.addEventListener('visibilitychange', () => {
  if (!document.hidden) {
    fetchStatus().then(() => {
      // Opening the popover mid-playback lands on the player surface, even
      // when playback was started remotely (phone/PWA) while we sat in the
      // grid. The grid icon is one tap away.
      if (currentStatus?.isPlaying) {
        showPlayer();
      } else {
        setTimeout(() => urlInput.focus(), 50);
      }
    });
    fetchLibrary();
  }
});
