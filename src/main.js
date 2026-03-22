const { invoke } = window.__TAURI__.core;

// ── Elements ──

const idleView = document.getElementById('idleView');
const playingView = document.getElementById('playingView');

const urlInput = document.getElementById('urlInput');
const urlInputPlaying = document.getElementById('urlInputPlaying');
const errorEl = document.getElementById('error');
const loadingEl = document.getElementById('loading');

const trackTitle = document.getElementById('trackTitle');
const trackSubtitle = document.getElementById('trackSubtitle');
const npThumbnail = document.getElementById('npThumbnail');

const pauseBtn = document.getElementById('pauseBtn');
const stopBtn = document.getElementById('stopBtn');
const seekBackBtn = document.getElementById('seekBackBtn');
const seekFwdBtn = document.getElementById('seekFwdBtn');
const prevBtn = document.getElementById('prevBtn');
const nextBtn = document.getElementById('nextBtn');

const volumeSlider = document.getElementById('volumeSlider');
const historyList = document.getElementById('historyList');
const showMoreBtn = document.getElementById('showMoreBtn');

const pauseIcon = document.getElementById('pauseIcon');
const playIcon = document.getElementById('playIcon');

const playingHistory = document.getElementById('playingHistory');
const playingHistoryList = document.getElementById('playingHistoryList');

const searchResults = document.getElementById('searchResults');
const searchResultsList = document.getElementById('searchResultsList');
const clearSearchBtn = document.getElementById('clearSearchBtn');
const idleHistorySection = document.getElementById('idleHistorySection');
const loadingText = document.getElementById('loadingText');

// ── State ──

let currentState = 'idle';
let lastTitle = null;
let historyExpanded = false;
let playingHistoryVisible = false;
let historyData = [];
let volumeInitialized = false;
let searchVisible = false;

// ── View Management ──

function showView(view) {
  idleView.classList.add('hidden');
  playingView.classList.add('hidden');

  if (view === 'idle') {
    idleView.classList.remove('hidden');
    computeIdleHeight();
  } else if (view === 'playing') {
    playingView.classList.remove('hidden');
    if (playingHistoryVisible) {
      computePlayingHeight();
    } else {
      resizeWindow(260);
    }
  }
}

function resizeWindow(height) {
  invoke('resize_window', { height }).catch(() => {});
}

function computeIdleHeight() {
  if (searchVisible) {
    const itemCount = searchResultsList.children.length;
    const base = 80; // input + loading + section header
    const perItem = 40;
    const height = base + (itemCount * perItem) + 8;
    resizeWindow(Math.max(height, 120));
  } else {
    const itemCount = historyList.children.length;
    const base = 107;
    const perItem = 40;
    const height = base + (itemCount * perItem) + 8;
    resizeWindow(Math.max(height, 120));
  }
}

// ── Status Polling ──

async function fetchStatus() {
  try {
    const status = await invoke('get_status');
    applyStatus(status);
    return status;
  } catch {
    return null;
  }
}

function applyStatus(status) {
  if (status.isPlaying) {
    currentState = 'playing';

    trackTitle.textContent = status.title || 'Playing...';

    if (status.queueLength > 1 && status.playlistTitle) {
      trackSubtitle.textContent = `${status.playlistTitle} \u2022 ${status.queuePosition + 1} of ${status.queueLength}`;
    } else if (status.queueLength > 1) {
      trackSubtitle.textContent = `Track ${status.queuePosition + 1} of ${status.queueLength}`;
    } else if (status.playlistTitle) {
      trackSubtitle.textContent = status.playlistTitle;
    } else {
      trackSubtitle.textContent = '';
    }

    if (status.thumbnail) {
      npThumbnail.innerHTML = `<img src="${escapeAttr(status.thumbnail)}" alt="" />`;
    } else if (status.videoId) {
      npThumbnail.innerHTML = `<img src="https://img.youtube.com/vi/${escapeAttr(status.videoId)}/mqdefault.jpg" alt="" />`;
    } else {
      npThumbnail.innerHTML = '';
    }

    if (status.isPaused) {
      pauseIcon.classList.add('hidden');
      playIcon.classList.remove('hidden');
      pauseBtn.title = 'Play';
    } else {
      pauseIcon.classList.remove('hidden');
      playIcon.classList.add('hidden');
      pauseBtn.title = 'Pause';
    }

    if (status.queueLength > 1) {
      prevBtn.classList.toggle('hidden', status.queuePosition <= 0);
      nextBtn.classList.toggle('hidden', status.queuePosition >= status.queueLength - 1);
    } else {
      prevBtn.classList.add('hidden');
      nextBtn.classList.add('hidden');
    }

    showView('playing');

    let trayState = status.isPaused ? 'paused' : 'playing';
    invoke('set_tray_state', { state: trayState }).catch(() => {});

    if (status.title && status.title !== lastTitle && !status.isPaused) {
      lastTitle = status.title;
      fireNotification(status);
    }

  } else {
    if (currentState !== 'idle') {
      currentState = 'idle';
      lastTitle = null;
      showView('idle');
      invoke('set_tray_state', { state: 'idle' }).catch(() => {});
      fetchHistory();
    }
  }
}

// ── Notifications ──

function fireNotification(status) {
  if (typeof Notification === 'undefined') return;
  if (Notification.permission !== 'granted') return;

  const opts = { body: status.title };
  if (status.thumbnail) {
    opts.icon = status.thumbnail;
  } else if (status.videoId) {
    opts.icon = `https://img.youtube.com/vi/${status.videoId}/mqdefault.jpg`;
  }
  try {
    new Notification('Now Playing', opts);
  } catch {}
}

// ── History ──

async function fetchHistory() {
  try {
    historyData = await invoke('get_history');
  } catch {
    historyData = [];
  }
  renderHistory();
}

function renderHistory() {
  historyList.innerHTML = '';

  if (!historyData || historyData.length === 0) {
    historyList.innerHTML = '<div class="history-empty">No recent plays</div>';
    showMoreBtn.classList.add('hidden');
    computeIdleHeight();
    return;
  }

  const limit = historyExpanded ? historyData.length : Math.min(5, historyData.length);

  for (let i = 0; i < limit; i++) {
    const item = historyData[i];
    const el = document.createElement('div');
    el.className = 'history-item';
    el.setAttribute('role', 'button');
    el.tabIndex = 0;

    const thumbUrl = item.thumbnail ||
      (item.url && extractVideoId(item.url) ? `https://img.youtube.com/vi/${extractVideoId(item.url)}/mqdefault.jpg` : '');

    const timeStr = formatRelativeTime(item.playedAt);
    const meta = item.trackCount > 1 ? `${item.trackCount} tracks \u2022 ${timeStr}` : timeStr;

    el.innerHTML = `
      ${thumbUrl ? `<img class="history-thumb" src="${escapeAttr(thumbUrl)}" alt="" />` : '<div class="history-thumb"></div>'}
      <div class="history-info">
        <div class="history-title">${escapeHtml(item.title || 'Untitled')}</div>
        <div class="history-meta">${escapeHtml(meta)}</div>
      </div>
      <div class="history-play-icon">
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M3 1.5L12 7L3 12.5V1.5Z" fill="currentColor"/>
        </svg>
      </div>
    `;

    el.addEventListener('click', () => playUrl(item.url));
    el.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') playUrl(item.url);
    });

    historyList.appendChild(el);
  }

  if (historyData.length > 5) {
    showMoreBtn.classList.remove('hidden');
    showMoreBtn.textContent = historyExpanded ? 'Show Less' : 'Show All';
  } else {
    showMoreBtn.classList.add('hidden');
  }

  computeIdleHeight();
}

// ── Playing History (expand when input focused) ──

function showPlayingHistory() {
  if (playingHistoryVisible) return;
  playingHistoryVisible = true;
  renderPlayingHistory();
  playingHistory.classList.remove('hidden');
  computePlayingHeight();
}

function hidePlayingHistory() {
  if (!playingHistoryVisible) return;
  playingHistoryVisible = false;
  playingHistory.classList.add('hidden');
  resizeWindow(260);
}

function renderPlayingHistory() {
  playingHistoryList.innerHTML = '';
  if (!historyData || historyData.length === 0) return;

  const limit = Math.min(5, historyData.length);
  for (let i = 0; i < limit; i++) {
    const item = historyData[i];
    const el = document.createElement('div');
    el.className = 'history-item';
    el.setAttribute('role', 'button');
    el.tabIndex = 0;

    const thumbUrl = item.thumbnail ||
      (item.url && extractVideoId(item.url) ? `https://img.youtube.com/vi/${extractVideoId(item.url)}/mqdefault.jpg` : '');

    const timeStr = formatRelativeTime(item.playedAt);
    const meta = item.trackCount > 1 ? `${item.trackCount} tracks \u2022 ${timeStr}` : timeStr;

    el.innerHTML = `
      ${thumbUrl ? `<img class="history-thumb" src="${escapeAttr(thumbUrl)}" alt="" />` : '<div class="history-thumb"></div>'}
      <div class="history-info">
        <div class="history-title">${escapeHtml(item.title || 'Untitled')}</div>
        <div class="history-meta">${escapeHtml(meta)}</div>
      </div>
      <div class="history-play-icon">
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M3 1.5L12 7L3 12.5V1.5Z" fill="currentColor"/>
        </svg>
      </div>
    `;

    el.addEventListener('mousedown', (e) => {
      e.preventDefault(); // prevent blur on the input
      playUrl(item.url);
    });

    playingHistoryList.appendChild(el);
  }
}

function computePlayingHeight() {
  const itemCount = playingHistoryList.children.length;
  const base = 260; // playing view base height
  const perItem = 40;
  const padding = 8;
  resizeWindow(base + (itemCount * perItem) + padding);
}

// ── Input Detection ──

function isUrl(input) {
  return /^https?:\/\//i.test(input) || /^(www\.)?(youtube\.com|youtu\.be|music\.youtube\.com)/i.test(input);
}

// ── Search ──

async function doSearch(query) {
  if (!query) return;

  errorEl.classList.add('hidden');
  loadingText.textContent = 'Searching...';
  loadingEl.classList.remove('hidden');

  try {
    const results = await invoke('search', { query });
    showSearchResults(results);
  } catch (err) {
    errorEl.textContent = typeof err === 'string' ? err : (err.message || 'Search failed');
    errorEl.classList.remove('hidden');
  } finally {
    loadingEl.classList.add('hidden');
  }
}

function showSearchResults(results) {
  searchResultsList.innerHTML = '';

  if (!results || results.length === 0) {
    searchResultsList.innerHTML = '<div class="history-empty">No results found</div>';
    searchResults.classList.remove('hidden');
    idleHistorySection.classList.add('hidden');
    searchVisible = true;
    computeIdleHeight();
    return;
  }

  for (const item of results) {
    const el = document.createElement('div');
    el.className = 'history-item';
    el.setAttribute('role', 'button');
    el.tabIndex = 0;

    const isPlaylist = item.trackCount != null;
    const durationStr = item.duration ? formatDuration(item.duration) : '';
    const trackStr = isPlaylist && item.trackCount ? `${item.trackCount} tracks` : '';
    const meta = [item.channel, trackStr, durationStr].filter(Boolean).join(' \u2022 ');

    el.innerHTML = `
      ${item.thumbnail ? `<img class="history-thumb" src="${escapeAttr(item.thumbnail)}" alt="" />` : '<div class="history-thumb"></div>'}
      <div class="history-info">
        <div class="history-title">${escapeHtml(item.title)}</div>
        <div class="history-meta">${escapeHtml(meta)}</div>
      </div>
      <div class="history-play-icon">
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M3 1.5L12 7L3 12.5V1.5Z" fill="currentColor"/>
        </svg>
      </div>
    `;

    const playUrlForItem = isPlaylist
      ? item.url
      : `https://www.youtube.com/watch?v=${item.videoId}&list=RD${item.videoId}`;

    el.addEventListener('click', () => {
      clearSearch();
      playUrl(playUrlForItem);
    });
    el.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        clearSearch();
        playUrl(playUrlForItem);
      }
    });

    searchResultsList.appendChild(el);
  }

  searchResults.classList.remove('hidden');
  idleHistorySection.classList.add('hidden');
  searchVisible = true;
  computeIdleHeight();
}

function clearSearch() {
  searchResults.classList.add('hidden');
  searchResultsList.innerHTML = '';
  idleHistorySection.classList.remove('hidden');
  searchVisible = false;
  urlInput.value = '';
  computeIdleHeight();
}

// ── Play URL ──

async function playUrl(url) {
  if (!url) return;

  errorEl.classList.add('hidden');
  loadingText.textContent = 'Connecting...';
  loadingEl.classList.remove('hidden');

  try {
    const status = await invoke('play', { url });
    hidePlayingHistory();
    urlInputPlaying.blur();
    applyStatus(status);
    urlInput.value = '';
    urlInputPlaying.value = '';
  } catch (err) {
    errorEl.textContent = typeof err === 'string' ? err : (err.message || 'Playback failed');
    errorEl.classList.remove('hidden');
  } finally {
    loadingEl.classList.add('hidden');
  }
}

// ── Actions ──

async function doPause() {
  try {
    const status = await invoke('toggle_pause');
    applyStatus(status);
  } catch {}
}

async function doStop() {
  try {
    const status = await invoke('stop');
    applyStatus(status);
  } catch {}
}

async function doSeek(seconds) {
  try {
    const status = await invoke('seek_relative', { seconds });
    applyStatus(status);
  } catch {}
}

async function doNext() {
  try {
    const status = await invoke('play_next');
    applyStatus(status);
  } catch {}
}

async function doPrev() {
  try {
    const status = await invoke('play_prev');
    applyStatus(status);
  } catch {}
}

async function doVolume(volume) {
  try {
    await invoke('set_volume', { volume: parseInt(volume, 10) });
  } catch {}
}

async function loadVolume() {
  try {
    const volume = await invoke('get_volume');
    if (typeof volume === 'number') {
      volumeSlider.value = volume;
    }
    volumeInitialized = true;
  } catch {
    volumeInitialized = true;
  }
}

// ── Event Listeners ──

urlInput.addEventListener('input', () => {
  errorEl.classList.add('hidden');
});

urlInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && urlInput.value.trim()) {
    const input = urlInput.value.trim();
    if (isUrl(input)) {
      clearSearch();
      playUrl(input);
    } else {
      doSearch(input);
    }
  }
  if (e.key === 'Escape' && searchVisible) {
    e.stopPropagation();
    clearSearch();
  }
});

urlInputPlaying.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && urlInputPlaying.value.trim()) {
    const input = urlInputPlaying.value.trim();
    if (isUrl(input)) {
      playUrl(input);
    } else {
      doSearch(input);
    }
  }
});

urlInputPlaying.addEventListener('focus', () => {
  fetchHistory().then(() => showPlayingHistory());
});

urlInputPlaying.addEventListener('blur', () => {
  hidePlayingHistory();
});

pauseBtn.addEventListener('click', doPause);
stopBtn.addEventListener('click', doStop);
seekBackBtn.addEventListener('click', () => doSeek(-10));
seekFwdBtn.addEventListener('click', () => doSeek(10));
prevBtn.addEventListener('click', doPrev);
nextBtn.addEventListener('click', doNext);

let volumeTimeout = null;
volumeSlider.addEventListener('input', () => {
  clearTimeout(volumeTimeout);
  volumeTimeout = setTimeout(() => {
    doVolume(volumeSlider.value);
  }, 50);
});

showMoreBtn.addEventListener('click', () => {
  historyExpanded = !historyExpanded;
  renderHistory();
});

clearSearchBtn.addEventListener('click', () => {
  clearSearch();
  urlInput.focus();
});

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    invoke('hide_window').catch(() => {});
  }
  if (e.key === 'q' && e.metaKey) {
    e.preventDefault();
    invoke('quit_app').catch(() => {});
  }
  if (e.key === ' ' && currentState === 'playing') {
    const tag = document.activeElement?.tagName;
    if (tag !== 'INPUT' && tag !== 'TEXTAREA') {
      e.preventDefault();
      doPause();
    }
  }
});

// ── Helpers ──

function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str;
  return div.innerHTML;
}

function escapeAttr(str) {
  return str.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/'/g, '&#39;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function extractVideoId(url) {
  if (!url) return null;
  const m = url.match(/(?:v=|youtu\.be\/|\/shorts\/)([a-zA-Z0-9_-]{11})/);
  return m ? m[1] : null;
}

function formatDuration(seconds) {
  if (!seconds || seconds <= 0) return '';
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  return `${m}:${String(s).padStart(2, '0')}`;
}

function formatRelativeTime(isoStr) {
  if (!isoStr) return '';
  try {
    const date = new Date(isoStr);
    const now = new Date();
    const diffMs = now - date;
    const diffMins = Math.floor(diffMs / 60000);
    const diffHrs = Math.floor(diffMs / 3600000);
    const diffDays = Math.floor(diffMs / 86400000);

    if (diffMins < 1) return 'Just now';
    if (diffMins < 60) return `${diffMins}m ago`;
    if (diffHrs < 24) return `${diffHrs}h ago`;
    if (diffDays < 7) return `${diffDays}d ago`;
    return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  } catch {
    return '';
  }
}

// ── Initialization ──

if (typeof Notification !== 'undefined' && Notification.permission === 'default') {
  Notification.requestPermission().catch(() => {});
}

loadVolume();
fetchHistory();

fetchStatus().then(() => {
  if (currentState === 'idle') {
    urlInput.focus();
  }
});

setInterval(fetchStatus, 1500);

document.addEventListener('visibilitychange', () => {
  if (!document.hidden) {
    if (currentState === 'idle') {
      setTimeout(() => urlInput.focus(), 50);
    }
    if (currentState === 'idle') {
      fetchHistory();
    }
    loadVolume();
  }
});
