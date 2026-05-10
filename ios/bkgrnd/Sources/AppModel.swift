import AVFoundation
import Combine
import Foundation

@MainActor
final class AppModel: ObservableObject {
  @Published var playlists: PlaylistDoc?
  @Published var recentMixes: Playlist?
  @Published var nowPlaying: PlaylistItem?
  @Published var isLoadingPlaylists = false
  @Published var lastSyncError: String?

  @Published var searchQuery = ""
  @Published var searchResults: [PlaylistItem] = []

  @Published var baseURLString: String = UserDefaults.standard.string(forKey: "woprBaseURL") ?? "http://worp.thriveos.pro:808"
  @Published var bearerToken: String = UserDefaults.standard.string(forKey: "woprBearerToken") ?? ""

  let audioPlayer = AudioPlayer()
  private var client: WOPRClient

  init() {
    let url = URL(string: UserDefaults.standard.string(forKey: "woprBaseURL") ?? "http://worp.thriveos.pro:808")!
    client = WOPRClient(config: .init(baseURL: url, bearerToken: UserDefaults.standard.string(forKey: "woprBearerToken")))
  }

  func updateServerSettings(baseURL: String, token: String) {
    baseURLString = baseURL
    bearerToken = token
    UserDefaults.standard.set(baseURL, forKey: "woprBaseURL")
    UserDefaults.standard.set(token, forKey: "woprBearerToken")

    if let url = URL(string: baseURL) {
      Task { await client.updateConfig(.init(baseURL: url, bearerToken: token.isEmpty ? nil : token)) }
    }
  }

  func refreshPlaylists() async {
    lastSyncError = nil
    isLoadingPlaylists = true
    defer { isLoadingPlaylists = false }
    do {
      let doc = try await client.fetchPlaylists()
      playlists = doc
      recentMixes = doc.playlists.first(where: { $0.id == "recent-mixes" }) ?? doc.playlists.first
    } catch {
      lastSyncError = error.localizedDescription
    }
  }

  func play(_ item: PlaylistItem) async {
    guard let url = URL(string: item.url),
          let baseURL = URL(string: baseURLString)
    else { return }

    nowPlaying = item

    // Keep config in sync if base URL changed.
    await client.updateConfig(.init(baseURL: baseURL, bearerToken: bearerToken.isEmpty ? nil : bearerToken))

    let stream = await client.streamURL(for: url)
    let headers = await client.streamHeaders()
    await audioPlayer.play(url: stream, headers: headers, title: item.title, artist: item.channel ?? "", artworkURL: item.thumbnail)
  }

  func performSearch() async {
    let q = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !q.isEmpty, let baseURL = URL(string: baseURLString) else {
      searchResults = []
      return
    }

    await client.updateConfig(.init(baseURL: baseURL, bearerToken: bearerToken.isEmpty ? nil : bearerToken))
    do {
      let results = try await client.search(query: q)
      searchResults = results.map {
        PlaylistItem(url: $0.url, title: $0.title, channel: $0.channel, thumbnail: $0.thumbnail, addedAt: nil)
      }
    } catch {
      // Fail quietly; keep last results.
    }
  }
}
