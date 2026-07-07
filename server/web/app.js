const LS_BASE = "bkgrnd.web.baseUrl";
const DEFAULT_BASE_URL = "https://bkgrnd.bedl.am";
const DEFAULT_THEME_COLOR = "#0d0e11";
const LEGACY_BASE_PATTERNS = [
  /wopr\.thriveos\.pro/i,
  /worp\.thriveos\.pro/i,
  /\/bkgrnd\/?$/i,
  /:8080?$/i
];

const el = (id) => document.getElementById(id);

const authView = el("authView");
const authTitle = el("authTitle");
const authUser = el("authUser");
const authPass = el("authPass");
const authMsg = el("authMsg");
const authSubmit = el("authSubmit");
const authToggle = el("authToggle");

const homeView = el("homeView");
const searchView = el("searchView");
const playerView = el("playerView");
const openSearchBtn = el("openSearchBtn");
const mixGrid = el("mixGrid");

const closeSearchBtn = el("closeSearchBtn");
const searchScope = el("searchScope");
const searchInput = el("searchInput");
const clearSearchBtn = el("clearSearchBtn");
const searchMsg = el("searchMsg");
const searchResults = el("searchResults");

const closePlayerBtn = el("closePlayerBtn");
const shareBtn = el("shareBtn");
const castBtn = el("castBtn");
const playerArt = el("playerArt");
const playerArtFallback = el("playerArtFallback");
const playerAmbient = el("playerAmbient");
const stageKicker = el("stageKicker");
const stageQueueChip = el("stageQueueChip");
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
const settingsUser = el("settingsUser");
const baseUrlInput = el("baseUrl");
const logoutBtn = el("logoutBtn");
const themeColorMeta = document.querySelector('meta[name="theme-color"]');

const audio = el("player");

let libraryItems = [];
let recentItems = [];
let searchItems = [];
let remoteNow = null;
let remoteQueue = [];
let remoteQueueIndex = -1;
let lastGridKey = "";
let activeStreamUrl = "";
let activeSourceUrl = "";
let activeSourceItem = null;
let activeResolveMeta = null;
let remotePlayToken = 0;
let playStartTimeout = null;
let acquisitionTimer = null;
let lastProgressAt = 0;
let currentUser = "";
let authMode = "login"; // or "register"
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

function apiBaseHref() {
  const base = getBaseUrl();
  if (base) return base.endsWith("/") ? base : `${base}/`;
  return new URL("./", window.location.href).toString();
}

function apiUrl(path) {
  return new URL(path, apiBaseHref()).toString();
}

// Cookies carry auth now; `include` covers the configurable cross-origin base.
function req(path, opts = {}) {
  const url = /^https?:/i.test(path) ? path : apiUrl(path);
  return fetch(url, { credentials: "include", cache: "no-store", ...opts });
}

function normalizeBaseUrl(raw) {
  const base = String(raw || "").trim().replace(/\/+$/, "");
  if (!base) return "";
  if (LEGACY_BASE_PATTERNS.some((pattern) => pattern.test(base))) return DEFAULT_BASE_URL;
  return base;
}

// ---- Auth -----------------------------------------------------------------

function showAuth(message) {
  authView.classList.add("active");
  homeView.classList.remove("active");
  searchView.classList.remove("active");
  playerView.classList.remove("active");
  setInline(authMsg, message || "");
}

function setAuthMode(mode) {
  authMode = mode;
  const registering = mode === "register";
  authTitle.textContent = registering ? "Create account" : "Sign in";
  authSubmit.textContent = registering ? "Create account" : "Sign in";
  authToggle.textContent = registering ? "Have an account? Sign in" : "Create an account";
  authPass.setAttribute("autocomplete", registering ? "new-password" : "current-password");
  setInline(authMsg, "");
}

async function fetchMe() {
  try {
    const resp = await req("api/v1/pwa/me");
    if (!resp.ok) return null;
    return await resp.json();
  } catch {
    return null;
  }
}

async function submitAuth() {
  const username = authUser.value.trim();
  const password = authPass.value;
  if (!username || !password) {
    setInline(authMsg, "Enter a username and password.");
    return;
  }
  authSubmit.disabled = true;
  setInline(authMsg, authMode === "register" ? "Creating account..." : "Signing in...");
  const path = authMode === "register" ? "api/v1/pwa/register" : "api/v1/pwa/login";
  try {
    const resp = await req(path, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ username, password })
    });
    if (!resp.ok) {
      const text = (await resp.text()).trim();
      setInline(authMsg, text || `Failed (${resp.status}).`);
      return;
    }
    authPass.value = "";
    await enterApp(username);
  } catch {
    setInline(authMsg, "Network error.");
  } finally {
    authSubmit.disabled = false;
  }
}

async function logout() {
  try {
    await req("api/v1/pwa/logout", { method: "POST" });
  } catch {}
  stopRemote();
  currentUser = "";
  authUser.value = "";
  authPass.value = "";
  setAuthMode("login");
  showAuth("Signed out.");
}

// Any API 401 mid-session (expired cookie) bounces back to the login screen.
function onUnauthorized() {
  if (!authView.classList.contains("active")) {
    currentUser = "";
    setAuthMode("login");
    showAuth("Session expired — sign in again.");
  }
}

async function enterApp(username) {
  currentUser = username || currentUser;
  authView.classList.remove("active");
  homeView.classList.add("active");
  settingsUser.textContent = currentUser || "—";
  await loadLibrary();
}

// ---- Rendering helpers ----------------------------------------------------

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
  const label = String(text || "").replace(/\.+$/, "").trim().toLowerCase();
  const fallback = getCurrentNow() ? (isPaused() ? "paused" : "streaming") : "idle";
  if (playerStatusTag) playerStatusTag.textContent = label || fallback;
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

function escapeHtml(str) {
  const d = document.createElement("div");
  d.textContent = String(str || "");
  return d.innerHTML;
}

function normalizeItem(raw) {
  const url = raw?.url || raw?.sourceUrl || "";
  const artist = bestSubtitle(raw);
  const duration = Number(raw?.duration);
  return {
    title: raw?.title || url || "Untitled",
    url,
    channel: artist,
    thumbnail: raw?.thumbnail || ytThumbFromUrl(url),
    duration: Number.isFinite(duration) && duration > 60 ? duration : null,
    live: raw?.type === "stream"
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
  const candidate = parts.slice(1).map(cleanArtist).find(isRealArtist);
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
      return proxy.toString();
    }
  } catch {}
  return raw;
}

async function fetchJson(path, fallback) {
  try {
    const resp = await req(path);
    if (resp.status === 401) {
      onUnauthorized();
      return fallback;
    }
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
  const items = libraryItems.length ? libraryItems : recentItems;
  const gridKey = JSON.stringify(items.slice(0, 24).map((item) => item.url));
  if (gridKey === lastGridKey) return;
  lastGridKey = gridKey;

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

function formatDurationChip(seconds) {
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${s}`;
  return `${m}:${s}`;
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
  if (item.live) {
    artWrap.insertAdjacentHTML("beforeend", `<span class="live-chip">LIVE</span>`);
  } else if (item.duration) {
    artWrap.insertAdjacentHTML("beforeend", `<span class="dur-chip">${formatDurationChip(item.duration)}</span>`);
  }

  const body = document.createElement("div");
  body.className = "mix-body";
  body.innerHTML = `
    <div class="mix-title">${escapeHtml(item.title)}</div>
    ${item.channel ? `<div class="mix-channel">${escapeHtml(item.channel)}</div>` : ""}
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
      <div class="mix-channel">${escapeHtml(item.channel || "Recent")}</div>
    </div>
    <div class="result-add">+</div>
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
    resp = await req(url.toString());
  } catch {
    setInline(searchMsg, "Network error talking to WOPR.");
    return;
  }

  if (resp.status === 401) {
    onUnauthorized();
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
  const ok = await playRemote(item);
  // On failure the item is pinned to the Now Playing stage; don't override it.
  if (ok !== false) setScreen("home");
}

function isSpotifyUrl(url) {
  return /^(https?:\/\/open\.spotify\.com\/|spotify:)/i.test(String(url || "").trim());
}

async function playRemote(item) {
  if (isSpotifyUrl(item.url)) {
    return await playSpotifyRemote(item);
  }
  remoteQueue = [];
  remoteQueueIndex = -1;
  return await playRemoteTrack(item);
}

let spotifyConversionToken = 0;

async function fetchSpotifyQueue(sourceUrl, maxTracks) {
  const url = new URL("api/v1/spotify/queue", apiBaseHref());
  url.searchParams.set("url", sourceUrl);
  if (maxTracks) url.searchParams.set("max_tracks", String(maxTracks));
  return req(url.toString());
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
    if (resp.status === 401) { onUnauthorized(); return; }
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
  if (playToken !== remotePlayToken) return true;
  if (!streamUrl) {
    markPlaybackFailed("couldn't play");
    return false;
  }

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
  return true;
}

function proxiedStreamUrl(sourceUrl, castsig) {
  const proxyUrl = new URL("api/v1/stream", apiBaseHref());
  proxyUrl.searchParams.set("url", sourceUrl);
  proxyUrl.searchParams.set("proxy", "true");
  if (castsig) proxyUrl.searchParams.set("castsig", castsig);
  return proxyUrl.toString();
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
    const resp = await req(url.toString());
    if (resp.status === 401) { onUnauthorized(); return ""; }
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

// A stream that fails to start stays pinned in Now Playing (with the failure
// visible) so it can be removed with the Stop control — there's otherwise no
// handle on it once it's the active item.
function markPlaybackFailed(reason) {
  if (playStartTimeout) clearTimeout(playStartTimeout);
  playStartTimeout = null;
  stopAcquisitionStatus();
  try {
    audio.pause();
    audio.removeAttribute("src");
    audio.load();
  } catch {}
  activeStreamUrl = "";
  setPlayerStatus(reason || "couldn't play");
  renderMiniPlayer();
  renderPlayer();
  setScreen("player");
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
      const resp = await req(prewarmUrl.toString());
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

function getCurrentNow() {
  return remoteNow;
}

function renderMiniPlayer() {
  const current = getCurrentNow();
  miniPlayer.classList.toggle("hidden", !current);
  if (!current) return;

  const title = current.title || "Untitled";
  miniTitle.innerHTML = `<span class="mini-title-track"><span>${escapeHtml(title)}</span><span aria-hidden="true">${escapeHtml(title)}</span></span>`;
  miniSub.textContent = current.channel;
  miniArt.src = current.thumbnail || "";
  miniAction.textContent = isPaused() ? "▶" : "Ⅱ";
  requestAnimationFrame(() => {
    const track = miniTitle.querySelector(".mini-title-track");
    const first = track?.firstElementChild;
    miniPlayer.classList.toggle("is-overflowing", Boolean(first && first.scrollWidth > miniTitle.clientWidth));
  });
}

let stageArtToken = 0;
let lastStageArt = "";
function setStageArt(src) {
  if (src === lastStageArt) return;
  lastStageArt = src;
  const token = ++stageArtToken;
  if (!src) {
    playerAmbient.style.backgroundImage = "";
    playerArt.removeAttribute("src");
    setThemeColor(DEFAULT_THEME_COLOR);
    return;
  }
  const proxied = thumbnailSrc(src);
  playerAmbient.style.backgroundImage = `url(${JSON.stringify(proxied)})`;
  playerArt.src = proxied;
  updateThemeColorFrom(proxied);

  const maxres = String(src).replace(/\/(?:default|mqdefault|hqdefault|sddefault)\.jpg(\?.*)?$/i, "/maxresdefault.jpg$1");
  if (maxres === src) return;
  const probe = new Image();
  probe.onload = () => {
    if (token !== stageArtToken || probe.naturalWidth <= 200) return;
    const upgraded = thumbnailSrc(maxres);
    playerArt.src = upgraded;
    playerAmbient.style.backgroundImage = `url(${JSON.stringify(upgraded)})`;
  };
  probe.src = maxres;
}

function renderPlayer() {
  const current = getCurrentNow();
  if (!current) {
    playerTitle.textContent = "Nothing playing";
    playerSub.textContent = "Recent";
    playerArt.classList.add("hidden");
    playerArtFallback.classList.remove("hidden");
    stageKicker.textContent = "";
    stageQueueChip.classList.add("hidden");
    setStageArt("");
    updateProgress();
    setPlayerStatus("");
    return;
  }

  playerTitle.textContent = current.title || "Untitled";
  playerSub.textContent = current.channel || "Recent";
  playerArt.classList.toggle("hidden", !current.thumbnail);
  playerArtFallback.classList.toggle("hidden", Boolean(current.thumbnail));
  setStageArt(current.thumbnail || "");

  if (remoteQueue.length > 1 && remoteQueueIndex >= 0) {
    stageKicker.textContent = `No. ${remoteQueueIndex + 1} of ${remoteQueue.length}`;
  } else {
    stageKicker.textContent = current.live ? "Live stream" : "";
  }

  if (remoteQueue.length > 1) {
    stageQueueChip.textContent = `Queue · ${remoteQueue.length}`;
    stageQueueChip.classList.remove("hidden");
  } else {
    stageQueueChip.classList.add("hidden");
  }

  playerPauseBtn.textContent = isPaused() ? "▶" : "Ⅱ";
  updateProgress();
}

function progressState() {
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
  miniPlayer.style.setProperty("--mini-progress", `${pct}%`);
  updateMediaSessionPosition();
}

function seekRelative(seconds) {
  if (!Number.isFinite(audio.duration)) return;
  audio.currentTime = Math.max(0, Math.min(audio.duration, audio.currentTime + seconds));
  updateProgress();
}

function isPaused() {
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
  if (audio.paused) audio.play().catch(() => setPlayerStatus("tap"));
  else audio.pause();
  renderMiniPlayer();
  renderPlayer();
}

async function stopCurrent() {
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

// ---- Cast (Android AirPlay equivalent via the Remote Playback API) --------

const remotePlayback = audio.remote && typeof audio.remote.watchAvailability === "function" ? audio.remote : null;
if (remotePlayback) {
  remotePlayback
    .watchAvailability((available) => castBtn.classList.toggle("hidden", !available))
    .catch(() => castBtn.classList.add("hidden"));
}

async function fetchCastSig() {
  try {
    const resp = await req("api/v1/pwa/cast-sig");
    if (!resp.ok) return "";
    const body = await resp.json();
    return body?.castsig || "";
  } catch {
    return "";
  }
}

async function castCurrent() {
  if (!remotePlayback) return;
  // Cast receivers fetch the stream URL themselves and can't send our cookie,
  // so swap the source to a short-lived signed URL (keeping position) first.
  if (activeSourceUrl) {
    const sig = await fetchCastSig();
    if (sig) {
      const pos = audio.currentTime;
      activeStreamUrl = proxiedStreamUrl(activeSourceUrl, sig);
      audio.src = activeStreamUrl;
      try {
        audio.load();
        await audio.play().catch(() => {});
        if (Number.isFinite(pos) && pos > 0) audio.currentTime = pos;
      } catch {}
    }
  }
  try {
    await remotePlayback.prompt();
  } catch {}
}

// ---- Media Session (Android lock-screen / notification controls) ----------

function updateMediaSessionMetadata() {
  if (!("mediaSession" in navigator)) return;
  const current = remoteNow;
  if (!current) return;
  try {
    const art = current.thumbnail ? thumbnailSrc(current.thumbnail) : "";
    const artwork = art
      ? [
          { src: art, sizes: "320x180", type: "image/jpeg" },
          { src: art, sizes: "640x360", type: "image/jpeg" }
        ]
      : [];
    navigator.mediaSession.metadata = new MediaMetadata({
      title: current.title || "bkgrnd",
      artist: current.channel || "",
      album: "bkgrnd",
      artwork
    });
  } catch {}
}

function updateMediaSessionPosition() {
  if (!("mediaSession" in navigator) || !("setPositionState" in navigator.mediaSession)) return;
  const duration = Number(audio.duration);
  if (!Number.isFinite(duration) || duration <= 0) return;
  try {
    navigator.mediaSession.setPositionState({
      duration,
      playbackRate: audio.playbackRate || 1,
      position: Math.min(Math.max(audio.currentTime || 0, 0), duration)
    });
    navigator.mediaSession.playbackState = audio.paused ? "paused" : "playing";
  } catch {}
}

if ("mediaSession" in navigator) {
  const set = (action, handler) => {
    try {
      navigator.mediaSession.setActionHandler(action, handler);
    } catch {}
  };
  set("play", () => audio.play().catch(() => {}));
  set("pause", () => audio.pause());
  set("nexttrack", () => advanceRemoteQueue(1));
  set("previoustrack", () => advanceRemoteQueue(-1));
  set("seekforward", (d) => seekRelative(d.seekOffset || 15));
  set("seekbackward", (d) => seekRelative(-(d.seekOffset || 15)));
  set("seekto", (d) => {
    if (typeof d.seekTime === "number" && Number.isFinite(audio.duration)) {
      audio.currentTime = Math.max(0, Math.min(audio.duration, d.seekTime));
      updateProgress();
    }
  });
}

// ---- Dynamic Android status-bar tint from the artwork ---------------------

const themeCanvas = document.createElement("canvas");
function setThemeColor(color) {
  if (themeColorMeta) themeColorMeta.setAttribute("content", color);
}

function updateThemeColorFrom(src) {
  if (!src || !themeColorMeta) return;
  const img = new Image();
  img.crossOrigin = "anonymous";
  img.onload = () => {
    try {
      themeCanvas.width = 8;
      themeCanvas.height = 8;
      const ctx = themeCanvas.getContext("2d", { willReadFrequently: true });
      ctx.drawImage(img, 0, 0, 8, 8);
      const { data } = ctx.getImageData(0, 0, 8, 8);
      let r = 0, g = 0, b = 0, n = 0;
      for (let i = 0; i < data.length; i += 4) {
        r += data[i]; g += data[i + 1]; b += data[i + 2]; n++;
      }
      // Darken toward the app's chrome so the status bar stays legible.
      const mix = (c) => Math.round((c / n) * 0.55);
      setThemeColor(`rgb(${mix(r)}, ${mix(g)}, ${mix(b)})`);
    } catch {
      setThemeColor(DEFAULT_THEME_COLOR);
    }
  };
  img.onerror = () => setThemeColor(DEFAULT_THEME_COLOR);
  img.src = src;
}

// ---- Audio element events -------------------------------------------------

audio.addEventListener("playing", () => {
  if (playStartTimeout) clearTimeout(playStartTimeout);
  playStartTimeout = null;
  stopAcquisitionStatus();
  lastProgressAt = Date.now();
  setPlayerStatus("");
  renderMiniPlayer();
  renderPlayer();
  updateMediaSessionMetadata();
  updateMediaSessionPosition();
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
  updateMediaSessionPosition();
});
audio.addEventListener("waiting", () => {
  if (!isActivelyPlaying() && !acquisitionTimer) setPlayerStatus("buffering");
});
audio.addEventListener("stalled", () => {
  const recentlyProgressed = Date.now() - lastProgressAt < 3000;
  if (!isActivelyPlaying() && !recentlyProgressed && !acquisitionTimer) setPlayerStatus("reconnect");
});
audio.addEventListener("error", () => {
  const code = audio.error?.code;
  const label = code === 2 ? "network" : code === 3 ? "decode" : code === 4 ? "unsupported" : "error";
  // A real source that errored before playing = failed to start: pin it in Now
  // Playing. (Guard on activeStreamUrl so clearing the src can't re-trigger.)
  if (remoteNow && activeStreamUrl && !isActivelyPlaying()) {
    markPlaybackFailed(label);
  } else {
    stopAcquisitionStatus();
    setPlayerStatus(label);
  }
});

// ---- Wiring ---------------------------------------------------------------

authSubmit.addEventListener("click", submitAuth);
authToggle.addEventListener("click", () => setAuthMode(authMode === "login" ? "register" : "login"));
authPass.addEventListener("keydown", (event) => {
  if (event.key === "Enter") submitAuth();
});
authUser.addEventListener("keydown", (event) => {
  if (event.key === "Enter") authPass.focus();
});

openSearchBtn.addEventListener("click", () => {
  searchScope.textContent = "Recent";
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
castBtn.addEventListener("click", castCurrent);
playerPauseBtn.addEventListener("click", toggleCurrentPause);
playerStopBtn.addEventListener("click", stopCurrent);
seekBackBtn.addEventListener("click", () => seekRelative(-15));
seekForwardBtn.addEventListener("click", () => seekRelative(15));
progressTrack.addEventListener("click", (event) => {
  if (!Number.isFinite(audio.duration) || audio.duration <= 0) return;
  const rect = progressTrack.getBoundingClientRect();
  const pct = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
  audio.currentTime = audio.duration * pct;
  updateProgress();
});
playerArt.addEventListener("click", openCurrentSource);
playerArtFallback.addEventListener("click", openCurrentSource);

settingsBtn.addEventListener("click", () => {
  baseUrlInput.value = getBaseUrl();
  settingsUser.textContent = currentUser || "—";
  settings.showModal();
});
settings.addEventListener("close", () => {
  localStorage.setItem(LS_BASE, normalizeBaseUrl(baseUrlInput.value || ""));
  loadLibrary();
});
logoutBtn.addEventListener("click", () => {
  settings.close();
  logout();
});

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

// ---- Boot -----------------------------------------------------------------

setAuthMode("login");
(async function boot() {
  const me = await fetchMe();
  if (me?.username) {
    await enterApp(me.username);
  } else {
    showAuth("");
  }
})();

setInterval(() => {
  if (!authView.classList.contains("active")) loadLibrary();
}, 60000);
document.addEventListener("visibilitychange", () => {
  if (!document.hidden && !authView.classList.contains("active")) loadLibrary();
});
