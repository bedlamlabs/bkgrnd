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
  @Published var statusMessage: String = ""
  @Published var showStage = false

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
      Task { await self?.advanceQueue(by: 1) }
    }
    audioPlayer.onSkipRequested = { [weak self] forward in
      Task { await self?.advanceQueue(by: forward ? 1 : -1) }
    }
  }

  func updateServerSettings(baseURL: String, token: String) {
    baseURLString = baseURL
    bearerToken = token
    UserDefaults.standard.set(baseURL, forKey: "woprBaseURL")
    KeychainStore.write("woprBearerToken", value: token)

    if let url = URL(string: baseURL) {
      Task { await client.updateConfig(.init(baseURL: url, bearerToken: token.isEmpty ? nil : token)) }
    }
  }

  private func syncClientConfig() async {
    guard let baseURL = URL(string: baseURLString) else { return }
    await client.updateConfig(.init(baseURL: baseURL, bearerToken: bearerToken.isEmpty ? nil : bearerToken))
  }

  var gridItems: [PlaylistItem] {
    recentPlaylist?.items ?? []
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
    queue = [item]
    queueIndex = 0
    queueTitle = ""
    await startPlayback(item)
  }

  /// Two-phase Spotify conversion, mirroring the PWA: convert just the first
  /// track so audio starts in seconds, swap in the full queue when it lands.
  private func playSpotify(_ item: PlaylistItem) async {
    playGeneration += 1
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
      }
    }
  }

  private func startPlayback(_ item: PlaylistItem) async {
    guard let url = URL(string: item.url) else { return }
    nowPlaying = item
    statusMessage = "resolving"
    await syncClientConfig()
    // Warm the resolver so the stream request is a cache hit.
    await client.prewarm(sourceURL: url)
    let stream = await client.streamURL(for: url)
    let headers = await client.streamHeaders()
    await audioPlayer.play(url: stream, headers: headers, title: item.title, artist: item.channel ?? "", artworkURL: item.thumbnail)
    statusMessage = ""
    prewarmNext()
  }

  private func prewarmNext() {
    let next = queueIndex + 1
    guard queue.indices.contains(next), let url = URL(string: queue[next].url) else { return }
    Task { await client.prewarm(sourceURL: url) }
  }

  func advanceQueue(by step: Int) async {
    let next = queueIndex + step
    guard queue.indices.contains(next) else {
      if step > 0 { stopLocal() }
      return
    }
    queueIndex = next
    await startPlayback(queue[next])
  }

  func stopLocal() {
    playGeneration += 1
    audioPlayer.stop()
    nowPlaying = nil
    queue = []
    queueIndex = -1
    queueTitle = ""
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
      await refreshRemoteStatus()
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
