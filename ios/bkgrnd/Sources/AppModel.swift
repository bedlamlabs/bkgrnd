import AVFoundation
import Combine
import Foundation

private let defaultBaseURLString = "https://bkgrnd.bedl.am"

enum PlaybackScope: String, CaseIterable {
  case recent = "Recent"
  case remote = "Remote"
}

@MainActor
final class AppModel: ObservableObject {
  /// Single instance shared by the SwiftUI phone UI and the CarPlay scene so
  /// both drive the same queue, player, and Now Playing state.
  static let shared = AppModel()

  @Published var playlists: PlaylistDoc?
  @Published var recentPlaylist: Playlist?
  @Published var isLoadingPlaylists = false
  @Published var lastSyncError: String?

  @Published var scope: PlaybackScope = .recent

  // Local (on-phone) playback state
  @Published var nowPlaying: PlaylistItem?
  @Published var queue: [PlaylistItem] = []
  @Published var queueIndex: Int = -1
  @Published var queueTitle: String = ""
  /// Playlist-level artwork (e.g. the Spotify playlist cover) — when set, the
  /// stage/ribbon/island show this instead of per-track YouTube art.
  @Published var queueArtwork: String?
  @Published var shuffleEnabled = false
  @Published var statusMessage: String = ""
  @Published var showStage = false

  /// Artwork the UI should show for current playback.
  var displayArtwork: String? {
    queueArtwork ?? nowPlaying?.thumbnail
  }

  @Published var searchQuery = ""
  @Published var searchResults: [PlaylistItem] = []
  @Published var isSearching = false

  @Published var remoteStatus: WOPRClient.LocalStatusResponse?
  @Published var lastRemoteError: String?

  @Published var baseURLString: String
  @Published var bearerToken: String

  let audioPlayer = AudioPlayer()
  private var client: WOPRClient
  // Invalidates in-flight Spotify conversions when the user moves on.
  private var playGeneration = 0

  init() {
    let storedBase = UserDefaults.standard.string(forKey: "woprBaseURL") ?? defaultBaseURLString
    // Token lives in the Keychain (survives reinstall-adjacent events, unlike
    // the PWA's localStorage). Migrate any legacy UserDefaults value once.
    var storedToken = KeychainStore.read("woprBearerToken") ?? ""
    if storedToken.isEmpty, let legacy = UserDefaults.standard.string(forKey: "woprBearerToken"), !legacy.isEmpty {
      storedToken = legacy
      KeychainStore.write("woprBearerToken", value: legacy)
      UserDefaults.standard.removeObject(forKey: "woprBearerToken")
    }

    baseURLString = storedBase
    bearerToken = storedToken
    let url = URL(string: storedBase) ?? URL(string: defaultBaseURLString)!
    client = WOPRClient(config: .init(baseURL: url, bearerToken: storedToken.isEmpty ? nil : storedToken))

    audioPlayer.onItemEnded = { [weak self] in
      // Queue exhausted, or nothing was pre-enqueued in time — advanceQueue
      // picks the next target (shuffle-aware) or stops cleanly.
      guard let self else { return }
      Task { await self.advanceQueue(by: 1) }
    }
    audioPlayer.onAdvanced = { [weak self] in
      guard let self else { return }
      let next = self.pendingNextIndex ?? (self.queueIndex + 1)
      self.pendingNextIndex = nil
      guard self.queue.indices.contains(next) else { return }
      self.queueIndex = next
      self.recoveryPolicy.reset()
      let item = self.queue[next]
      self.nowPlaying = item
      self.recordAppPlay(item.url)
      self.audioPlayer.setNowPlayingMeta(title: item.title, artist: item.channel ?? "", artworkURL: self.displayArtwork)
      self.enqueueUpcoming()
    }
    audioPlayer.onSkipRequested = { [weak self] forward in
      Task { await self?.advanceQueue(by: forward ? 1 : -1) }
    }
    audioPlayer.onPlaybackError = { [weak self] message in
      self?.recoverPlayback(after: message)
    }
    audioPlayer.onPlaybackStalled = { [weak self] message in
      self?.recoverPlayback(after: message)
    }
  }

  func updateServerSettings(baseURL: String, token: String) {
    baseURLString = baseURL
    bearerToken = token
    UserDefaults.standard.set(baseURL, forKey: "woprBaseURL")
    KeychainStore.write("woprBearerToken", value: token)

    if let url = URL(string: baseURL) {
      Task {
        await client.updateConfig(.init(baseURL: url, bearerToken: token.isEmpty ? nil : token))
        // The initial fetches likely 401'd before credentials existed;
        // re-fetch immediately so saving settings brings the grid alive.
        await refreshPlaylists()
        await refreshRemoteStatus()
      }
    }
  }

  private func syncClientConfig() async {
    guard let baseURL = URL(string: baseURLString) else { return }
    await client.updateConfig(.init(baseURL: baseURL, bearerToken: bearerToken.isEmpty ? nil : bearerToken))
  }

  // Per-scope ordering: Remote mirrors the Mac's canonical order exactly;
  // Recent re-sorts by plays made FROM THIS APP — local playback and
  // remote-commanded alike (server order as tiebreak for never-played items).
  private var appPlayHistory: [String: Date] = {
    (UserDefaults.standard.dictionary(forKey: "localPlayHistory") as? [String: Double])?
      .mapValues { Date(timeIntervalSince1970: $0) } ?? [:]
  }()

  private func recordAppPlay(_ url: String) {
    appPlayHistory[url] = Date()
    UserDefaults.standard.set(appPlayHistory.mapValues { $0.timeIntervalSince1970 }, forKey: "localPlayHistory")
    objectWillChange.send() // re-sort the Recent grid
  }

  // Dedup key: the YouTube video id, so a plain URL and its "&list=RD…" Mix
  // variant collapse to one grid entry. Non-YouTube URLs key on themselves.
  private static let youtubeIdRegex = try! NSRegularExpression(pattern: "(?:v=|youtu\\.be/|/shorts/)([A-Za-z0-9_-]{11})")
  private func videoKey(_ url: String) -> String {
    let range = NSRange(url.startIndex..., in: url)
    if let match = Self.youtubeIdRegex.firstMatch(in: url, range: range),
       let idRange = Range(match.range(at: 1), in: url) {
      return String(url[idRange])
    }
    return url
  }

  var gridItems: [PlaylistItem] {
    var seen = Set<String>()
    let base = (recentPlaylist?.items ?? []).filter { seen.insert(videoKey($0.url)).inserted }
    guard scope == .recent, !appPlayHistory.isEmpty else { return base }
    return base.enumerated().sorted { a, b in
      let pa = appPlayHistory[a.element.url]
      let pb = appPlayHistory[b.element.url]
      switch (pa, pb) {
      case let (.some(da), .some(db)): return da > db
      case (.some, .none): return true
      case (.none, .some): return false
      case (.none, .none): return a.offset < b.offset
      }
    }.map(\.element)
  }

  func refreshPlaylists() async {
    lastSyncError = nil
    isLoadingPlaylists = true
    defer { isLoadingPlaylists = false }
    await syncClientConfig()
    do {
      let doc = try await client.fetchPlaylists()
      playlists = doc
      recentPlaylist = doc.playlists.first(where: { $0.id == "streams" || $0.id == "recent" || $0.id == "recent-mixes" }) ?? doc.playlists.first
    } catch {
      lastSyncError = error.localizedDescription
    }
  }

  // MARK: - Playing (routes by scope)

  func playItem(_ item: PlaylistItem) async {
    if scope == .remote {
      await playRemote(item)
      return
    }
    if item.isSpotify {
      await playSpotify(item)
      return
    }
    playGeneration += 1
    recoveryPolicy.reset()
    queue = [item]
    queueIndex = 0
    queueTitle = ""
    queueArtwork = nil
    pendingNextIndex = nil
    showStage = true
    await startPlayback(item)
  }

  /// Two-phase Spotify conversion, mirroring the PWA: convert just the first
  /// track so audio starts in seconds, swap in the full queue when it lands.
  private func playSpotify(_ item: PlaylistItem) async {
    playGeneration += 1
    recoveryPolicy.reset()
    let generation = playGeneration
    statusMessage = "converting"
    nowPlaying = item
    showStage = true
    await syncClientConfig()

    do {
      let first = try await client.spotifyQueue(for: item.url, maxTracks: 1)
      guard generation == playGeneration else { return }
      guard let firstItem = first.items.first?.playlistItem else {
        statusMessage = "no matches"
        return
      }
      queue = [firstItem]
      queueIndex = 0
      queueTitle = first.title
      queueArtwork = first.thumbnail.isEmpty ? nil : first.thumbnail
      pendingNextIndex = nil
      await startPlayback(firstItem)
    } catch {
      guard generation == playGeneration else { return }
      statusMessage = error.localizedDescription
      return
    }

    Task { [weak self] in
      guard let self else { return }
      if let full = try? await self.client.spotifyQueue(for: item.url) {
        guard generation == self.playGeneration else { return }
        let items = full.items.map(\.playlistItem)
        guard items.count > self.queue.count else { return }
        let currentURL = self.queue.indices.contains(self.queueIndex) ? self.queue[self.queueIndex].url : nil
        self.queue = items
        self.queueIndex = items.firstIndex(where: { $0.url == currentURL }) ?? 0
        // Track 1 started with a single-item queue; arm the next track now.
        self.enqueueUpcoming()
      }
    }
  }

  private func startPlayback(_ item: PlaylistItem) async {
    guard let url = URL(string: item.url) else { return }
    nowPlaying = item
    // Pasted URLs arrive titled with the raw link (and no artwork). Resolve the
    // real name/thumbnail in the background and patch Now Playing.
    if item.title == item.url {
      Task { await self.enrichNowPlaying(item, url: url) }
    }
    statusMessage = "resolving"
    await syncClientConfig()
    // No blocking prewarm here: /api/v1/stream resolves (and caches) on-demand,
    // so awaiting a separate prewarm just serialized the cold yt-dlp resolve
    // (~1.6s, worse under load) ahead of AVPlayer. Hand the proxy URL straight
    // to the player and let the resolve overlap with its connection setup.
    let stream = await client.streamURL(for: url)
    let headers = await client.streamHeaders()
    await audioPlayer.play(url: stream, headers: headers, title: item.title, artist: item.channel ?? "", artworkURL: displayArtwork ?? item.thumbnail)
    statusMessage = ""
    recordAppPlay(item.url)
    enqueueUpcoming()
  }

  /// Fetch a pasted URL's real title/channel/thumbnail and patch Now Playing
  /// (UI + lock screen), if it's still the current track.
  private func enrichNowPlaying(_ item: PlaylistItem, url: URL) async {
    guard let meta = await client.meta(for: url), !meta.title.isEmpty else { return }
    guard nowPlaying?.url == item.url else { return }
    var updated = item
    updated.title = meta.title
    if !meta.channel.isEmpty { updated.channel = meta.channel }
    if (updated.thumbnail ?? "").isEmpty, !meta.thumbnail.isEmpty { updated.thumbnail = meta.thumbnail }
    nowPlaying = updated
    if queue.indices.contains(queueIndex) { queue[queueIndex] = updated }
    audioPlayer.setNowPlayingMeta(title: updated.title, artist: updated.channel ?? "", artworkURL: displayArtwork ?? updated.thumbnail)
  }

  /// Pre-resolve and pre-enqueue the next queue track so AVQueuePlayer can
  /// cross the track boundary with no app-side work (the app gets suspended
  /// if it needs the network right when the session goes silent).
  private var recoveryPolicy = PlaybackRecoveryPolicy()
  /// The index the pre-enqueued item corresponds to (shuffle picks ahead).
  private var pendingNextIndex: Int?

  /// Shuffle semantics: converted playlists (multi-track queues) shuffle
  /// within the queue and keep going; a lone stream with shuffle on hops to
  /// a random saved stream (appended to the queue so pre-enqueue still works).
  private func chooseNextIndex() -> Int? {
    if shuffleEnabled {
      if queue.count > 1 {
        var candidate = queueIndex
        while candidate == queueIndex {
          candidate = Int.random(in: 0..<queue.count)
        }
        return candidate
      }
      let currentURL = nowPlaying?.url
      let candidates = (recentPlaylist?.items ?? []).filter { $0.url != currentURL }
      if let pick = candidates.randomElement() {
        queue.append(pick)
        return queue.count - 1
      }
      return nil
    }
    let next = queueIndex + 1
    return queue.indices.contains(next) ? next : nil
  }

  func toggleShuffle() {
    shuffleEnabled.toggle()
    // Re-pick and replace the pre-enqueued track under the new mode.
    pendingNextIndex = nil
    enqueueUpcoming()
  }

  private func enqueueUpcoming() {
    guard let next = pendingNextIndex ?? chooseNextIndex(),
          queue.indices.contains(next),
          let url = URL(string: queue[next].url)
    else { return }
    pendingNextIndex = next
    let expected = queue[next].url
    Task {
      await client.prewarm(sourceURL: url)
      let stream = await client.streamURL(for: url)
      let headers = await client.streamHeaders()
      // Queue/mode may have changed while resolving.
      guard pendingNextIndex == next, queue.indices.contains(next), queue[next].url == expected else { return }
      audioPlayer.enqueueNext(url: stream, headers: headers)
    }
  }

  func advanceQueue(by step: Int) async {
    let target: Int?
    if step > 0 {
      target = pendingNextIndex ?? chooseNextIndex()
    } else {
      target = queueIndex > 0 ? queueIndex - 1 : nil
    }
    pendingNextIndex = nil
    guard let target, queue.indices.contains(target) else {
      if step > 0 { stopLocal() }
      return
    }
    queueIndex = target
    recoveryPolicy.reset()
    await startPlayback(queue[target])
  }

  /// A player item can remain waiting forever without transitioning to
  /// AVPlayerItem.Status.failed. Give each user-initiated item one automatic
  /// recovery: evict the relay's cached direct URL, resolve again, and replace
  /// the AVPlayerItem. A second failure is surfaced instead of retry-looping.
  private func recoverPlayback(after message: String) {
    statusMessage = message
    guard queue.indices.contains(queueIndex) else { return }
    let item = queue[queueIndex]
    guard recoveryPolicy.claimRetry(for: item.url) else { return }
    let generation = playGeneration
    statusMessage = "retrying"

    Task {
      guard let sourceURL = URL(string: item.url) else { return }
      _ = await client.refreshStream(sourceURL: sourceURL)
      guard generation == playGeneration, nowPlaying?.url == item.url else { return }
      await startPlayback(item)
    }
  }

  func stopLocal() {
    playGeneration += 1
    recoveryPolicy.reset()
    audioPlayer.stop()
    nowPlaying = nil
    queue = []
    queueIndex = -1
    queueTitle = ""
    queueArtwork = nil
    pendingNextIndex = nil
    statusMessage = ""
    showStage = false
  }

  // MARK: - Search

  func performSearch() async {
    let q = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !q.isEmpty else {
      searchResults = []
      return
    }

    // Pasted URLs play directly (Spotify URLs convert via the relay).
    if q.hasPrefix("http://") || q.hasPrefix("https://") || q.hasPrefix("spotify:") {
      let item = PlaylistItem(url: q, title: q, channel: nil, thumbnail: nil, addedAt: nil, duration: nil, type: nil)
      searchResults = []
      await playItem(item)
      return
    }

    isSearching = true
    defer { isSearching = false }
    await syncClientConfig()
    do {
      let results = try await client.search(query: q)
      searchResults = results.map {
        PlaylistItem(url: $0.url, title: $0.title, channel: $0.channel, thumbnail: $0.thumbnail, addedAt: nil, duration: $0.duration, type: nil)
      }
    } catch {
      // Fail quietly; keep last results.
    }
  }

  // MARK: - Remote (Mac) control

  var remotePlayerStatus: WOPRClient.LocalPlayerStatus? {
    remoteStatus?.status
  }

  var remoteIsPlaying: Bool {
    remoteStatus?.online == true && remotePlayerStatus?.isPlaying == true
  }

  func refreshRemoteStatus() async {
    lastRemoteError = nil
    await syncClientConfig()
    do {
      remoteStatus = try await client.fetchLocalStatus()
    } catch {
      lastRemoteError = error.localizedDescription
    }
  }

  func playRemote(_ item: PlaylistItem) async {
    await syncClientConfig()
    do {
      try await client.sendLocalCommand(.init(
        action: "play",
        url: item.url,
        title: item.title,
        thumbnail: item.thumbnail ?? "",
        sourceUrl: item.url
      ))
      recordAppPlay(item.url)
      await refreshRemoteStatus()
      showStage = true
    } catch {
      lastRemoteError = error.localizedDescription
    }
  }

  func toggleRemotePause() async {
    await sendRemoteCommand("pause_toggle")
  }

  func stopRemote() async {
    await sendRemoteCommand("stop")
  }

  private func sendRemoteCommand(_ action: String) async {
    await syncClientConfig()
    do {
      try await client.sendLocalCommand(.init(action: action))
      await refreshRemoteStatus()
    } catch {
      lastRemoteError = error.localizedDescription
    }
  }
}
