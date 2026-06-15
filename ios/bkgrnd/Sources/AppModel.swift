import AVFoundation
import Combine
import Foundation

private let defaultBaseURLString = "https://bkgrnd.bedl.am"

@MainActor
final class AppModel: ObservableObject {
  @Published var playlists: PlaylistDoc?
  @Published var recentPlaylist: Playlist?
  @Published var nowPlaying: PlaylistItem?
  @Published var isLoadingPlaylists = false
  @Published var lastSyncError: String?

  @Published var searchQuery = ""
  @Published var searchResults: [PlaylistItem] = []
  @Published var remoteStatus: WOPRClient.LocalStatusResponse?
  @Published var lastRemoteError: String?

  @Published var baseURLString: String = UserDefaults.standard.string(forKey: "woprBaseURL") ?? defaultBaseURLString
  @Published var bearerToken: String = UserDefaults.standard.string(forKey: "woprBearerToken") ?? ""

  let audioPlayer = AudioPlayer()
  private var client: WOPRClient

  init() {
    let url = URL(string: UserDefaults.standard.string(forKey: "woprBaseURL") ?? defaultBaseURLString)!
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

  private func syncClientConfig() async {
    guard let baseURL = URL(string: baseURLString) else { return }
    await client.updateConfig(.init(baseURL: baseURL, bearerToken: bearerToken.isEmpty ? nil : bearerToken))
  }

  func refreshPlaylists() async {
    lastSyncError = nil
    isLoadingPlaylists = true
    defer { isLoadingPlaylists = false }
    do {
      let doc = try await client.fetchPlaylists()
      playlists = doc
      recentPlaylist = doc.playlists.first(where: { $0.id == "recent" || $0.id == "streams" || $0.id == "recent-mixes" }) ?? doc.playlists.first
    } catch {
      lastSyncError = error.localizedDescription
    }
  }

  func play(_ item: PlaylistItem) async {
    guard let url = URL(string: item.url) else { return }

    nowPlaying = item

    await syncClientConfig()

    let stream = await client.streamURL(for: url)
    let headers = await client.streamHeaders()
    await audioPlayer.play(url: stream, headers: headers, title: item.title, artist: item.channel ?? "", artworkURL: item.thumbnail)
  }

  func performSearch() async {
    let q = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !q.isEmpty else {
      searchResults = []
      return
    }

    await syncClientConfig()
    do {
      let results = try await client.search(query: q)
      searchResults = results.map {
        PlaylistItem(url: $0.url, title: $0.title, channel: $0.channel, thumbnail: $0.thumbnail, addedAt: nil)
      }
    } catch {
      // Fail quietly; keep last results.
    }
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
