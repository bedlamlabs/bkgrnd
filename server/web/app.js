const LS_BASE = "bkgrnd.web.baseUrl";
const LS_TOKEN = "bkgrnd.web.token";
const DEFAULT_BASE_URL = "https://bkgrnd.bedl.am";
const LEGACY_BASE_PATTERNS = [
  /wopr\.thriveos\.pro/i,
  /worp\.thriveos\.pro/i,
  /\/bkgrnd\/?$/i,
  /:8080?$/i
];

const el = (id) => document.getElementById(id);

const homeView = el("homeView");
const searchView = el("searchView");
const playerView = el("playerView");
const remoteScopeBtn = el("remoteScopeBtn");
const localScopeBtn = el("localScopeBtn");
const localActiveDot = el("localActiveDot");
const openSearchBtn = el("openSearchBtn");
const scopeStatus = el("scopeStatus");
const mixGrid = el("mixGrid");

const closeSearchBtn = el("closeSearchBtn");
const searchScope = el("searchScope");
const searchInput = el("searchInput");
const clearSearchBtn = el("clearSearchBtn");
const searchMsg = el("searchMsg");
const searchResults = el("searchResults");

const closePlayerBtn = el("closePlayerBtn");
const shareBtn = el("shareBtn");
const playerArt = el("playerArt");
const playerArtFallback = el("playerArtFallback");
const playerTitle = el("playerTitle");
const playerSub = el("playerSub");
const playerStatusTag = el("playerStatusTag");
const playerStopBtn = el("playerStopBtn");
const playerPauseBtn = el("playerPauseBtn");
const seekBackBtn = el("seekBackBtn");
const seekForwardBtn = el("seekForwardBtn");
const progressTrack = el("progressTrack");
const progressFill = el("progressFill");
const progressKnob = el("progressKnob");
const elapsedTime = el("elapsedTime");
const durationTime = el("durationTime");

const miniPlayer = el("miniPlayer");
const miniArt = el("miniArt");
const miniTitle = el("miniTitle");
const miniSub = el("miniSub");
const miniAction = el("miniAction");

const settingsBtn = el("settingsBtn");
const settings = el("settings");
const baseUrlInput = el("baseUrl");
const tokenInput = el("token");

const audio = el("player");

let activeScope = "remote";
let libraryItems = [];
let recentItems = [];
let searchItems = [];
let remoteNow = null;
let remoteQueue = [];
let remoteQueueIndex = -1;
let lastGridKey = "";
let localState = { online: false, status: null };
let activeStreamUrl = "";
let activeSourceUrl = "";
let activeSourceItem = null;
let activeResolveMeta = null;
let remotePlayToken = 0;
let playStartTimeout = null;
let acquisitionTimer = null;
let lastProgressAt = 0;
const prewarmed = new Set();
const prewarming = new Set();
const failedPrewarm = new Set();
let prewarmQueue = [];
let prewarmRunning = false;

function getBaseUrl() {
  const raw = (localStorage.getItem(LS_BASE) || "").trim();
  if (!raw) return "";
  const normalized = normalizeBaseUrl(raw);
  if (normalized !== raw.replace(/\/+$/, "")) {
    localStorage.setItem(LS_BASE, normalized);
  }
  return normalized;
}

function getToken() {
  return (localStorage.getItem(LS_TOKEN) || "").trim();
}

function withToken(url) {
  const token = getToken();
  if (!token) return url;
  const next = new URL(url, window.location.href);
  next.searchParams.set("token", token);
  return next.toString();
}

function apiBaseHref() {
  const base = getBaseUrl();
  if (base) return base.endsWith("/") ? base : `${base}/`;
  return new URL("./", window.location.href).toString();
}

function apiUrl(path) {
  return withToken(new URL(path, apiBaseHref()).toString());
}

function normalizeBaseUrl(raw) {
  const base = String(raw || "").trim().replace(/\/+$/, "");
  if (!base) return "";
  if (LEGACY_BASE_PATTERNS.some((pattern) => pattern.test(base))) return DEFAULT_BASE_URL;
  return base;
}

function setScreen(name) {
  homeView.classList.toggle("active", name === "home");
  searchView.classList.toggle("active", name === "search");
  playerView.classList.toggle("active", name === "player");
  miniPlayer.classList.toggle("on-player-screen", name === "player");
}

function setInline(node, text) {
  if (!node) return;
  node.textContent = text || "";
  node.classList.toggle("hidden", !text);
}

function setPlayerStatus(text) {
  const label = String(text || "").replace(/\.+$/, "").trim() || "";
  setInline(playerStatusTag, label.toLowerCase());
}

function stopAcquisitionStatus() {
  if (acquisitionTimer) clearInterval(acquisitionTimer);
  acquisitionTimer = null;
}

function startAcquisitionStatus(sourceUrl) {
  stopAcquisitionStatus();
  const startedAt = Date.now();
  const update = () => {
    const elapsed = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
    const ready = prewarmed.has(sourceUrl);
    if (ready) {
      setPlayerStatus(elapsed < 3 ? "starting" : `buffering ${formatElapsed(elapsed)}`);
    } else if (elapsed < 15) {
      setPlayerStatus(`resolving ${formatElapsed(elapsed)}`);
    } else if (elapsed < 45) {
      setPlayerStatus(`cold ${formatElapsed(elapsed)}`);
    } else {
      setPlayerStatus(`slow ${formatElapsed(elapsed)}`);
    }
  };
  update();
  acquisitionTimer = setInterval(update, 1000);
}

function formatElapsed(seconds) {
  const mins = Math.floor(seconds / 60);
  const secs = String(seconds % 60).padStart(2, "0");
  return `${mins}:${secs}`;
}

function formatDuration(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return "--:--";
  const total = Math.floor(seconds);
  const mins = Math.floor(total / 60);
  const secs = String(total % 60).padStart(2, "0");
  return `${mins}:${secs}`;
}

function scopeLabel() {
  return activeScope === "local" ? "Remote" : "Recent";
}

function setScope(scope) {
  activeScope = scope;
  remoteScopeBtn.classList.toggle("active", scope === "remote");
  localScopeBtn.classList.toggle("active", scope === "local");
  searchScope.textContent = scopeLabel();
  renderGrid();
  renderMiniPlayer();
}

function escapeHtml(str) {
  const d = document.createElement("div");
  d.textContent = String(str || "");
  return d.innerHTML;
}

function normalizeItem(raw) {
  const url = raw?.url || raw?.sourceUrl || "";
  const artist = bestSubtitle(raw);
  return {
    title: raw?.title || url || "Untitled",
    url,
    channel: artist,
    thumbnail: raw?.thumbnail || ytThumbFromUrl(url)
  };
}

function bestSubtitle(raw) {
  const channel = String(raw?.channel || "").trim();
  const playlistTitle = String(raw?.playlistTitle || "").trim();
  const uploader = String(raw?.uploader || "").trim();
  const inferred = inferArtistFromTitle(raw?.title);
  if (isRealArtist(channel)) return channel;
  if (isRealArtist(uploader)) return uploader;
  if (inferred) return inferred;
  if (isRealArtist(playlistTitle)) return playlistTitle;
  return "";
}

function inferArtistFromTitle(title) {
  const text = String(title || "");
  const mixMatch = text.match(/^Mix\s*-\s*([^-|]+?)\s*-\s*/i);
  if (mixMatch && isRealArtist(mixMatch[1])) return cleanArtist(mixMatch[1]);

  const parts = text.split("|").map((part) => part.trim()).filter(Boolean);
  if (parts.length < 2) return "";
  const candidate = parts
    .slice(1)
    .map(cleanArtist)
    .find(isRealArtist);
  return candidate ? cleanArtist(candidate) : "";
}

function cleanArtist(value) {
  return String(value || "")
    .replace(/#\d+\b/g, "")
    .replace(/\b4K\b|\bUHD\b|\bHD\b/gi, "")
    .replace(/\s+/g, " ")
    .trim();
}

function isRealArtist(value) {
  const text = cleanArtist(value);
  if (!text) return false;
  if (/^(youtube|bkgrnd|recent|remote|recent mixes|remote mixes|local mixes|mixes)$/i.test(text)) return false;
  if (/^(music|live|stream|playlist|video|unknown)$/i.test(text)) return false;
  if (/(chillout|relax|lounge|bar music|live stream|deep house mix)/i.test(text)) return false;
  return true;
}

function ytThumbFromUrl(url) {
  const match = String(url || "").match(/(?:v=|youtu\.be\/|\/shorts\/)([a-zA-Z0-9_-]{11})/);
  return match ? `https://i.ytimg.com/vi/${match[1]}/mqdefault.jpg` : "";
}

function createArtwork(src, className = "mix-art") {
  const img = document.createElement("img");
  img.className = className;
  img.alt = "";
  if (src) img.src = thumbnailSrc(src);
  else img.classList.add("thumb-missing");
  img.addEventListener("error", () => {
    img.removeAttribute("src");
    img.classList.add("thumb-missing");
  }, { once: true });
  return img;
}

function thumbnailSrc(src) {
  const raw = String(src || "").trim();
  if (!raw) return "";
  try {
    const url = new URL(raw);
    if (url.hostname === "i.ytimg.com" || url.hostname.endsWith(".ytimg.com")) {
      const proxy = new URL("api/v1/thumbnail", apiBaseHref());
      proxy.searchParams.set("src", raw);
      return withToken(proxy.toString());
    }
  } catch {}
  return raw;
}

async function fetchJson(path, fallback) {
  try {
    const resp = await fetch(apiUrl(path), { cache: "no-store" });
    if (!resp.ok) return fallback;
    return await resp.json();
  } catch {
    return fallback;
  }
}

async function loadLibrary() {
  const [playlistDoc, historyDoc] = await Promise.all([
    fetchJson("api/v1/playlists.json", null),
    fetchJson("api/v1/history.json", [])
  ]);

  const playlistItems = [];
  const playlists = Array.isArray(playlistDoc?.playlists) ? playlistDoc.playlists : [];
  for (const playlist of playlists) {
    const items = Array.isArray(playlist.items) ? playlist.items : [];
    for (const item of items) {
      playlistItems.push(normalizeItem({ ...item, playlistTitle: playlist.name }));
    }
  }

  recentItems = Array.isArray(historyDoc) ? historyDoc.map(normalizeItem) : [];
  const seen = new Set();
  libraryItems = [...playlistItems, ...recentItems].filter((item) => {
    if (!item.url || seen.has(item.url)) return false;
    seen.add(item.url);
    return true;
  });

  renderGrid();
}

function renderGrid() {
  const localStatus = localState.status;
  const localPlaying = Boolean(localState.online && localStatus?.isPlaying);

  // This runs on every 3s status poll; skip the full DOM rebuild (24 cards +
  // images) when nothing it renders has actually changed.
  const items = libraryItems.length ? libraryItems : recentItems;
  const gridKey = JSON.stringify([
    activeScope,
    localState.online,
    localPlaying,
    localStatus?.title || "",
    items.slice(0, 24).map((item) => item.url)
  ]);
  if (gridKey === lastGridKey) return;
  lastGridKey = gridKey;

  localActiveDot.classList.toggle("hidden", !localPlaying);

  if (activeScope === "local" && !localState.online) {
    scopeStatus.textContent = "Mac is unavailable. Open bkgrnd on the Mac to enable Remote.";
    scopeStatus.classList.remove("hidden");
  } else if (activeScope === "local" && localPlaying) {
    scopeStatus.textContent = `Playing on Mac: ${localStatus.title || "Untitled"}`;
    scopeStatus.classList.remove("hidden");
  } else {
    scopeStatus.classList.add("hidden");
  }

  mixGrid.innerHTML = "";
  if (!items.length) {
    mixGrid.innerHTML = `<div class="scope-status">No recent streams yet. Use search to start one.</div>`;
    return;
  }

  for (const item of items.slice(0, 24)) {
    mixGrid.appendChild(renderCard(item));
  }
  prewarmItems(items.slice(0, 6));
}

function renderCard(item) {
  const button = document.createElement("button");
  button.className = "mix-card";
  button.type = "button";
  button.addEventListener("click", () => playItem(item));

  const artWrap = document.createElement("div");
  artWrap.className = "mix-art-wrap";
  artWrap.appendChild(createArtwork(item.thumbnail));
  artWrap.insertAdjacentHTML("beforeend", `<div class="fallback-mark">b</div>`);

  const body = document.createElement("div");
  body.className = "mix-body";
  body.innerHTML = `
    <div class="mix-title">${escapeHtml(item.title)}</div>
    ${item.channel ? `<div class="mix-channel">${escapeHtml(item.channel)}</div>` : ""}
    <div class="mix-play">PLAY</div>
  `;

  button.appendChild(artWrap);
  button.appendChild(body);
  return button;
}

function renderResult(item) {
  const button = document.createElement("button");
  button.className = "result-row";
  button.type = "button";
  button.addEventListener("click", () => playItem(item));
  button.appendChild(createArtwork(item.thumbnail, ""));
  button.insertAdjacentHTML("beforeend", `
    <div>
      <div class="mix-title">${escapeHtml(item.title)}</div>
      <div class="mix-channel">${escapeHtml(item.channel || scopeLabel())}</div>
    </div>
    <div class="result-add">${activeScope === "local" ? "▶" : "+"}</div>
  `);
  return button;
}

async function search(query) {
  const q = query.trim();
  if (!q) return;

  searchResults.innerHTML = "";
  searchItems = [];
  setInline(searchMsg, "Searching...");

  if (/^https?:\/\//i.test(q)) {
    const item = normalizeItem({ url: q, title: q });
    searchItems = [item];
    setInline(searchMsg, "");
    searchResults.appendChild(renderResult(item));
    return;
  }

  let resp;
  try {
    const url = new URL("api/v1/search", apiBaseHref());
    url.searchParams.set("q", q);
    resp = await fetch(withToken(url.toString()), { cache: "no-store" });
  } catch {
    setInline(searchMsg, "Network error talking to WOPR.");
    return;
  }

  if (!resp.ok) {
    setInline(searchMsg, `Search failed (${resp.status}).`);
    return;
  }

  const items = await resp.json().catch(() => []);
  searchItems = Array.isArray(items) ? items.map(normalizeItem) : [];
  if (!searchItems.length) {
    setInline(searchMsg, "No results.");
    return;
  }

  setInline(searchMsg, "");
  for (const item of searchItems) {
    searchResults.appendChild(renderResult(item));
  }
}

async function playItem(item) {
  if (activeScope === "local") {
    await sendLocalCommand("play", item);
    await refreshLocalStatus();
    setScreen("home");
    return;
  }
  await playRemote(item);
  setScreen("home");
}

function isSpotifyUrl(url) {
  return /^(https?:\/\/open\.spotify\.com\/|spotify:)/i.test(String(url || "").trim());
}

async function playRemote(item) {
  if (isSpotifyUrl(item.url)) {
    await playSpotifyRemote(item);
    return;
  }
  remoteQueue = [];
  remoteQueueIndex = -1;
  await playRemoteTrack(item);
}

// Spotify URLs can't be resolved by yt-dlp; the server converts them into a
// YouTube-backed queue which we then play with client-side auto-advance.
// Two-phase: convert just the first track so audio starts in seconds, then
// swap in the full queue when the complete conversion lands.
let spotifyConversionToken = 0;

async function fetchSpotifyQueue(sourceUrl, maxTracks) {
  const url = new URL("api/v1/spotify/queue", apiBaseHref());
  url.searchParams.set("url", sourceUrl);
  if (maxTracks) url.searchParams.set("max_tracks", String(maxTracks));
  return fetch(withToken(url.toString()), { cache: "no-store" });
}

async function playSpotifyRemote(item) {
  const playToken = ++remotePlayToken;
  const conversionToken = ++spotifyConversionToken;
  remoteNow = item;
  activeSourceItem = item;
  activeResolveMeta = null;
  renderMiniPlayer();
  renderPlayer();
  setPlayerStatus("converting");
  startAcquisitionStatus(item.url);

  let body;
  try {
    const resp = await fetchSpotifyQueue(item.url, 1);
    if (playToken !== remotePlayToken) return;
    if (!resp.ok) {
      const text = (await resp.text()).trim();
      console.warn("Spotify conversion failed", resp.status, text);
      stopAcquisitionStatus();
      setPlayerStatus(resolveFailureStatus(resp, text));
      return;
    }
    body = await resp.json();
  } catch {
    if (playToken !== remotePlayToken) return;
    stopAcquisitionStatus();
    setPlayerStatus("error");
    return;
  }

  if (playToken !== remotePlayToken) return;
  const firstItems = Array.isArray(body?.items) ? body.items.map(normalizeItem) : [];
  if (!firstItems.length) {
    stopAcquisitionStatus();
    setPlayerStatus("no matches");
    return;
  }

  remoteQueue = firstItems;
  remoteQueueIndex = 0;
  const firstPlay = playRemoteTrack(firstItems[0]);

  // Phase 2: full conversion in the background; extend the queue in place
  // unless the user has started a different conversion meanwhile.
  fetchSpotifyQueue(item.url)
    .then((resp) => (resp.ok ? resp.json() : null))
    .then((full) => {
      if (conversionToken !== spotifyConversionToken) return;
      const items = Array.isArray(full?.items) ? full.items.map(normalizeItem) : [];
      if (items.length <= remoteQueue.length) return;
      const currentUrl = remoteQueue[remoteQueueIndex]?.url;
      const currentIndex = items.findIndex((entry) => entry.url === currentUrl);
      remoteQueue = items;
      remoteQueueIndex = currentIndex >= 0 ? currentIndex : 0;
    })
    .catch(() => {});

  await firstPlay;
}

function advanceRemoteQueue(step) {
  if (!remoteQueue.length) return false;
  const next = remoteQueueIndex + step;
  if (next < 0 || next >= remoteQueue.length) return false;
  remoteQueueIndex = next;
  playRemoteTrack(remoteQueue[next]);
  return true;
}

async function playRemoteTrack(item) {
  const sourceUrl = String(item.url || "").trim();
  if (!sourceUrl) return;
  const playToken = ++remotePlayToken;
  if (activeSourceUrl === sourceUrl && activeStreamUrl && !audio.paused) {
    return;
  }

  setPlayerStatus("connecting");
  remoteNow = item;
  activeSourceItem = item;
  activeResolveMeta = null;
  renderMiniPlayer();
  renderPlayer();

  startAcquisitionStatus(sourceUrl);
  const streamUrl = await resolveStreamUrl(sourceUrl);
  if (playToken !== remotePlayToken) return;
  if (!streamUrl) return;

  activeSourceUrl = sourceUrl;
  activeStreamUrl = proxiedStreamUrl(sourceUrl);
  audio.src = activeStreamUrl;
  try {
    audio.load();
    const promise = audio.play();
    if (promise?.catch) {
      promise.catch(() => {
        cleanupFailedAudio();
        setPlayerStatus("tap");
      });
    }
    if (playStartTimeout) clearTimeout(playStartTimeout);
    playStartTimeout = setTimeout(() => setPlayerStatus("buffering"), 900);
  } catch {
    cleanupFailedAudio();
    setPlayerStatus("tap");
  }
}

function proxiedStreamUrl(sourceUrl) {
  const proxyUrl = new URL("api/v1/stream", apiBaseHref());
  proxyUrl.searchParams.set("url", sourceUrl);
  proxyUrl.searchParams.set("proxy", "true");
  return withToken(proxyUrl.toString());
}

function resolveFailureStatus(resp, text) {
  if (resp.status === 401 || resp.status === 403) return "auth";
  if (resp.status === 408 || resp.status === 504 || /timed out/i.test(text)) return "timeout";
  if (resp.status >= 500) return `server ${resp.status}`;
  if (resp.status >= 400 && text) return "unavailable";
  return `resolve ${resp.status}`;
}

async function resolveStreamUrl(sourceUrl) {
  const url = new URL("api/v1/resolve", apiBaseHref());
  url.searchParams.set("url", sourceUrl);
  try {
    const resp = await fetch(withToken(url.toString()), { cache: "no-store" });
    if (!resp.ok) {
      const text = (await resp.text()).trim();
      console.warn("Stream resolve failed", resp.status, text);
      setPlayerStatus(resolveFailureStatus(resp, text));
      return "";
    }
    const body = await resp.json();
    if (!body?.streamUrl) {
      setPlayerStatus("error");
      return "";
    }
    activeResolveMeta = {
      cached: Boolean(body.cached),
      source: body.source || "",
      resolveMs: Number(body.resolveMs || 0)
    };
    if (activeResolveMeta.cached) {
      setPlayerStatus("cached");
    } else if (activeResolveMeta.source) {
      setPlayerStatus(activeResolveMeta.source);
    }
    prewarmed.add(sourceUrl);
    return body.streamUrl;
  } catch {
    setPlayerStatus("error");
    return "";
  }
}

function cleanupFailedAudio() {
  if (playStartTimeout) clearTimeout(playStartTimeout);
  playStartTimeout = null;
  try {
    audio.pause();
    audio.removeAttribute("src");
    audio.load();
  } catch {}
}

function prewarmItems(items) {
  if (activeScope !== "remote") return;
  for (const item of items) {
    prewarmStream(item.url);
  }
}

function prewarmStream(sourceUrl) {
  const url = String(sourceUrl || "").trim();
  if (!url || prewarmed.has(url) || prewarming.has(url) || failedPrewarm.has(url)) return;
  prewarming.add(url);
  prewarmQueue.push(url);
  runPrewarmQueue();
}

async function runPrewarmQueue() {
  if (prewarmRunning) return;
  prewarmRunning = true;

  while (prewarmQueue.length) {
    const url = prewarmQueue.shift();
    const prewarmUrl = new URL("api/v1/prewarm", apiBaseHref());
    prewarmUrl.searchParams.set("url", url);
    try {
      const resp = await fetch(withToken(prewarmUrl.toString()), { cache: "no-store" });
      if (resp.ok) prewarmed.add(url);
      else failedPrewarm.add(url);
    } catch {
      prewarmed.delete(url);
      failedPrewarm.add(url);
    } finally {
      prewarming.delete(url);
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }

  prewarmRunning = false;
}

async function sendLocalCommand(action, item = {}) {
  const body = {
    action,
    url: item.url || "",
    title: item.title || "",
    thumbnail: item.thumbnail || "",
    sourceUrl: item.url || ""
  };
  try {
    const resp = await fetch(apiUrl("api/v1/local/commands"), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body)
    });
    if (!resp.ok) {
      setPlayerStatus("error");
    }
  } catch {
    setPlayerStatus("error");
  }
}

async function refreshLocalStatus() {
  localState = await fetchJson("api/v1/local/status", { online: false, status: null });
  renderGrid();
  renderMiniPlayer();
  if (activeScope === "local") renderPlayer();
}

function getCurrentNow() {
  if (activeScope === "local") {
    const status = localState.status;
    if (!localState.online || !status?.isPlaying) return null;
    return normalizeItem({
      title: status.title,
      channel: status.playlistTitle || "Playing on Mac",
      thumbnail: status.thumbnail,
      url: status.sourceUrl
    });
  }
  return remoteNow;
}

function renderMiniPlayer() {
  const current = getCurrentNow();
  miniPlayer.classList.toggle("hidden", !current);
  if (!current) return;

  const title = current.title || "Untitled";
  miniTitle.innerHTML = `<span class="mini-title-track"><span>${escapeHtml(title)}</span><span aria-hidden="true">${escapeHtml(title)}</span></span>`;
  miniSub.textContent = activeScope === "local" ? "Remote" : current.channel;
  miniArt.src = current.thumbnail || "";
  miniAction.textContent = isPaused() ? "▶" : "Ⅱ";
  requestAnimationFrame(() => {
    const track = miniTitle.querySelector(".mini-title-track");
    const first = track?.firstElementChild;
    miniPlayer.classList.toggle("is-overflowing", Boolean(first && first.scrollWidth > miniTitle.clientWidth));
  });
}

function renderPlayer() {
  const current = getCurrentNow();
  if (!current) {
    playerTitle.textContent = activeScope === "local" ? "Mac unavailable" : "Nothing playing";
    playerSub.textContent = scopeLabel();
    playerArt.classList.add("hidden");
    playerArtFallback.classList.remove("hidden");
    updateProgress();
    setPlayerStatus("");
    return;
  }

  playerTitle.textContent = current.title || "Untitled";
  playerSub.textContent = current.channel || scopeLabel();
  playerArt.classList.toggle("hidden", !current.thumbnail);
  playerArtFallback.classList.toggle("hidden", Boolean(current.thumbnail));
  if (current.thumbnail) playerArt.src = current.thumbnail;
  playerPauseBtn.textContent = isPaused() ? "▶" : "Ⅱ";
  updateProgress();
}

function progressState() {
  if (activeScope === "local") {
    const status = localState.status || {};
    return {
      position: Number(status.position || 0),
      duration: Number(status.duration || 0)
    };
  }
  return {
    position: Number(audio.currentTime || 0),
    duration: Number(audio.duration || 0)
  };
}

function updateProgress() {
  const { position, duration } = progressState();
  const hasDuration = Number.isFinite(duration) && duration > 0;
  const pct = hasDuration ? Math.max(0, Math.min(100, (position / duration) * 100)) : 0;
  progressFill.style.width = `${pct}%`;
  progressKnob.style.left = `${pct}%`;
  elapsedTime.textContent = formatDuration(position).replace("--:--", "0:00");
  durationTime.textContent = hasDuration ? formatDuration(duration) : "--:--";
}

function seekRelative(seconds) {
  if (activeScope === "local") return;
  if (!Number.isFinite(audio.duration)) return;
  audio.currentTime = Math.max(0, Math.min(audio.duration, audio.currentTime + seconds));
  updateProgress();
}

function isPaused() {
  if (activeScope === "local") return Boolean(localState.status?.isPaused);
  return audio.paused;
}

function isActivelyPlaying() {
  return !audio.paused && !audio.ended && audio.currentTime > 0 && audio.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA;
}

function stopRemote() {
  remotePlayToken += 1;
  audio.pause();
  audio.removeAttribute("src");
  activeSourceUrl = "";
  activeStreamUrl = "";
  activeSourceItem = null;
  activeResolveMeta = null;
  audio.load();
  remoteNow = null;
  remoteQueue = [];
  remoteQueueIndex = -1;
  stopAcquisitionStatus();
  setPlayerStatus("");
  renderMiniPlayer();
  renderPlayer();
}

async function toggleCurrentPause() {
  if (activeScope === "local") {
    await sendLocalCommand("pause_toggle");
    await refreshLocalStatus();
    return;
  }
  if (audio.paused) audio.play().catch(() => setPlayerStatus("tap"));
  else audio.pause();
  renderMiniPlayer();
  renderPlayer();
}

async function stopCurrent() {
  if (activeScope === "local") {
    await sendLocalCommand("stop");
    await refreshLocalStatus();
    return;
  }
  stopRemote();
}

async function shareCurrent() {
  const current = getCurrentNow();
  const url = current?.url || window.location.href;
  const title = current?.title || "Bkgrnd";
  if (navigator.share) {
    try {
      await navigator.share({ title, url });
      return;
    } catch {}
  }
  await navigator.clipboard?.writeText(url).catch(() => {});
  setPlayerStatus("copied");
}

function openCurrentSource() {
  const current = getCurrentNow();
  if (current?.url) window.open(current.url, "_blank", "noopener");
}

audio.addEventListener("playing", () => {
  if (playStartTimeout) clearTimeout(playStartTimeout);
  playStartTimeout = null;
  stopAcquisitionStatus();
  lastProgressAt = Date.now();
  setPlayerStatus("");
  renderMiniPlayer();
  renderPlayer();
  updateMediaSessionMetadata();
  // Warm the next queued track so auto-advance starts instantly.
  if (remoteQueue.length && remoteQueueIndex >= 0 && remoteQueueIndex < remoteQueue.length - 1) {
    prewarmStream(remoteQueue[remoteQueueIndex + 1].url);
  }
});
audio.addEventListener("ended", () => {
  if (!advanceRemoteQueue(1)) {
    renderMiniPlayer();
    renderPlayer();
  }
});
audio.addEventListener("timeupdate", () => {
  lastProgressAt = Date.now();
  updateProgress();
  if (isActivelyPlaying()) {
    stopAcquisitionStatus();
    setPlayerStatus("");
  }
});
audio.addEventListener("durationchange", updateProgress);
audio.addEventListener("loadedmetadata", updateProgress);
audio.addEventListener("pause", () => {
  renderMiniPlayer();
  renderPlayer();
});
audio.addEventListener("waiting", () => {
  if (!isActivelyPlaying() && !acquisitionTimer) setPlayerStatus("buffering");
});
audio.addEventListener("stalled", () => {
  const recentlyProgressed = Date.now() - lastProgressAt < 3000;
  if (!isActivelyPlaying() && !recentlyProgressed && !acquisitionTimer) setPlayerStatus("reconnect");
});
audio.addEventListener("error", () => {
  stopAcquisitionStatus();
  const code = audio.error?.code;
  const label = code === 2 ? "network" : code === 3 ? "decode" : code === 4 ? "unsupported" : "error";
  setPlayerStatus(label);
});

remoteScopeBtn.addEventListener("click", () => setScope("remote"));
localScopeBtn.addEventListener("click", () => setScope("local"));
openSearchBtn.addEventListener("click", () => {
  searchScope.textContent = scopeLabel();
  setScreen("search");
  searchInput.focus();
});
closeSearchBtn.addEventListener("click", () => setScreen("home"));
clearSearchBtn.addEventListener("click", () => {
  searchInput.value = "";
  searchResults.innerHTML = "";
  setInline(searchMsg, "");
});
searchInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") search(searchInput.value);
});

miniPlayer.addEventListener("click", () => {
  renderPlayer();
  setScreen("player");
});
miniPlayer.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" && event.key !== " ") return;
  event.preventDefault();
  renderPlayer();
  setScreen("player");
});
miniAction.addEventListener("click", (event) => {
  event.stopPropagation();
  toggleCurrentPause();
});
closePlayerBtn.addEventListener("click", () => setScreen("home"));
shareBtn.addEventListener("click", shareCurrent);
playerPauseBtn.addEventListener("click", toggleCurrentPause);
playerStopBtn.addEventListener("click", stopCurrent);
seekBackBtn.addEventListener("click", () => seekRelative(-15));
seekForwardBtn.addEventListener("click", () => seekRelative(15));
progressTrack.addEventListener("click", (event) => {
  if (activeScope === "local" || !Number.isFinite(audio.duration) || audio.duration <= 0) return;
  const rect = progressTrack.getBoundingClientRect();
  const pct = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
  audio.currentTime = audio.duration * pct;
  updateProgress();
});
playerArt.addEventListener("click", openCurrentSource);
playerArtFallback.addEventListener("click", openCurrentSource);

settingsBtn.addEventListener("click", () => {
  baseUrlInput.value = getBaseUrl();
  tokenInput.value = localStorage.getItem(LS_TOKEN) || "";
  settings.showModal();
});
settings.addEventListener("close", () => {
  localStorage.setItem(LS_BASE, normalizeBaseUrl(baseUrlInput.value || ""));
  localStorage.setItem(LS_TOKEN, (tokenInput.value || "").trim());
  loadLibrary();
  refreshLocalStatus();
});

// Lock-screen / control-center metadata for the current remote track.
function updateMediaSessionMetadata() {
  if (!("mediaSession" in navigator)) return;
  const current = remoteNow;
  if (!current) return;
  try {
    const artwork = current.thumbnail
      ? [{ src: thumbnailSrc(current.thumbnail), sizes: "320x180", type: "image/jpeg" }]
      : [];
    navigator.mediaSession.metadata = new MediaMetadata({
      title: current.title || "bkgrnd",
      artist: current.channel || "",
      album: "bkgrnd",
      artwork
    });
  } catch {}
}

if ("mediaSession" in navigator) {
  try {
    navigator.mediaSession.setActionHandler("play", () => audio.play().catch(() => {}));
    navigator.mediaSession.setActionHandler("pause", () => audio.pause());
    navigator.mediaSession.setActionHandler("nexttrack", () => advanceRemoteQueue(1));
    navigator.mediaSession.setActionHandler("previoustrack", () => advanceRemoteQueue(-1));
  } catch {}
}

if ("serviceWorker" in navigator) {
  window.addEventListener("load", () => {
    navigator.serviceWorker.getRegistrations().then((registrations) => {
      const ownRegistrations = registrations.filter((registration) => {
        try {
          return new URL(registration.scope).origin === window.location.origin;
        } catch {
          return false;
        }
      });
      Promise.all(ownRegistrations.map((registration) => registration.unregister())).then(() => {
        if (navigator.serviceWorker.controller && sessionStorage.getItem("bkgrnd.swReloaded") !== "1") {
          sessionStorage.setItem("bkgrnd.swReloaded", "1");
          window.location.reload();
        }
      });
    }).catch(() => {});
  });
}

setScope("remote");
loadLibrary();
refreshLocalStatus();
setInterval(refreshLocalStatus, 3000);
