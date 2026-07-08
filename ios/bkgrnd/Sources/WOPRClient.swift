import Foundation

actor WOPRClient {
  struct Config: Equatable {
    var baseURL: URL
    var bearerToken: String?
  }

  private(set) var config: Config

  init(config: Config) {
    self.config = config
  }

  func updateConfig(_ config: Config) {
    self.config = config
  }

  func putPlaylists(_ doc: PlaylistDoc) async throws {
    var req = URLRequest(url: config.baseURL.appendingPathComponent("/api/v1/playlists.json"))
    req.httpMethod = "PUT"
    req.setValue("application/json", forHTTPHeaderField: "content-type")
    addAuth(&req)
    req.httpBody = try JSONEncoder().encode(doc)
    let (_, resp) = try await URLSession.shared.data(for: req)
    guard let status = (resp as? HTTPURLResponse)?.statusCode, (200..<300).contains(status) else {
      throw URLError(.badServerResponse)
    }
  }

  func fetchPlaylists() async throws -> PlaylistDoc {
    var req = URLRequest(url: config.baseURL.appendingPathComponent("/api/v1/playlists.json"))
    req.httpMethod = "GET"
    addAuth(&req)
    let (data, resp) = try await URLSession.shared.data(for: req)
    guard (resp as? HTTPURLResponse)?.statusCode == 200 else {
      throw URLError(.badServerResponse)
    }
    return try JSONDecoder().decode(PlaylistDoc.self, from: data)
  }

  func streamURL(for sourceURL: URL) -> URL {
    var comps = URLComponents(url: config.baseURL.appendingPathComponent("/api/v1/stream"), resolvingAgainstBaseURL: false)!
    // proxy=true is required off-network: without it the server 302s to the
    // raw googlevideo URL, which is IP-locked to the server and 403s for us.
    // The token query param rides along because AVPlayer's follow-up range
    // requests don't reliably carry custom Authorization headers.
    var items = [
      URLQueryItem(name: "url", value: sourceURL.absoluteString),
      URLQueryItem(name: "proxy", value: "true"),
    ]
    if let token = config.bearerToken, !token.isEmpty {
      items.append(URLQueryItem(name: "token", value: token))
    }
    comps.queryItems = items
    return comps.url!
  }

  /// Ask the server to resolve (and cache) the stream URL ahead of playback
  /// so the first range request is served without a cold yt-dlp resolve.
  func prewarm(sourceURL: URL) async {
    var comps = URLComponents(url: config.baseURL.appendingPathComponent("/api/v1/prewarm"), resolvingAgainstBaseURL: false)!
    comps.queryItems = [URLQueryItem(name: "url", value: sourceURL.absoluteString)]
    var req = URLRequest(url: comps.url!)
    req.httpMethod = "GET"
    addAuth(&req)
    _ = try? await URLSession.shared.data(for: req)
  }

  func streamHeaders() -> [String: String] {
    guard let token = config.bearerToken, !token.isEmpty else { return [:] }
    return ["Authorization": "Bearer \(token)"]
  }

  struct SearchResult: Decodable, Identifiable {
    var id: String { url }
    var title: String
    var url: String
    var videoId: String
    var thumbnail: String
    var channel: String
    var duration: Double?
  }

  /// Convert a Spotify playlist/album/track URL into a YouTube-backed queue
  /// via the relay. maxTracks=1 is the fast first-track phase.
  func spotifyQueue(for spotifyURL: String, maxTracks: Int? = nil) async throws -> SpotifyQueueResponse {
    var comps = URLComponents(url: config.baseURL.appendingPathComponent("/api/v1/spotify/queue"), resolvingAgainstBaseURL: false)!
    var items = [URLQueryItem(name: "url", value: spotifyURL)]
    if let maxTracks {
      items.append(URLQueryItem(name: "max_tracks", value: String(maxTracks)))
    }
    comps.queryItems = items
    var req = URLRequest(url: comps.url!)
    req.httpMethod = "GET"
    req.timeoutInterval = 120
    addAuth(&req)
    let (data, resp) = try await URLSession.shared.data(for: req)
    guard (resp as? HTTPURLResponse)?.statusCode == 200 else {
      let message = String(data: data, encoding: .utf8) ?? ""
      throw NSError(domain: "bkgrnd", code: 1, userInfo: [NSLocalizedDescriptionKey: message.isEmpty ? "Spotify conversion failed" : message])
    }
    return try JSONDecoder().decode(SpotifyQueueResponse.self, from: data)
  }

  struct LocalStatusResponse: Decodable {
    var online: Bool
    var receivedAgoMs: UInt64?
    var status: LocalPlayerStatus?
  }

  struct LocalPlayerStatus: Decodable {
    var isPlaying: Bool
    var isPaused: Bool
    var title: String
    var thumbnail: String
    var sourceUrl: String
    var playlistTitle: String
    var duration: Double?
    var position: Double?
  }

  struct LocalCommandRequest: Encodable {
    var action: String
    var url: String = ""
    var title: String = ""
    var thumbnail: String = ""
    var sourceUrl: String = ""
  }

  func search(query: String) async throws -> [SearchResult] {
    var comps = URLComponents(url: config.baseURL.appendingPathComponent("/api/v1/search"), resolvingAgainstBaseURL: false)!
    comps.queryItems = [URLQueryItem(name: "q", value: query)]
    var req = URLRequest(url: comps.url!)
    req.httpMethod = "GET"
    addAuth(&req)
    let (data, resp) = try await URLSession.shared.data(for: req)
    guard (resp as? HTTPURLResponse)?.statusCode == 200 else {
      throw URLError(.badServerResponse)
    }
    return try JSONDecoder().decode([SearchResult].self, from: data)
  }

  struct MetaResponse: Decodable {
    var title: String = ""
    var channel: String = ""
    var thumbnail: String = ""
  }

  /// Video title/channel/thumbnail via the relay's oEmbed proxy — lets a pasted
  /// URL show a real name instead of the raw link.
  func meta(for sourceURL: URL) async -> MetaResponse? {
    var comps = URLComponents(url: config.baseURL.appendingPathComponent("/api/v1/meta"), resolvingAgainstBaseURL: false)!
    comps.queryItems = [URLQueryItem(name: "url", value: sourceURL.absoluteString)]
    var req = URLRequest(url: comps.url!)
    req.httpMethod = "GET"
    addAuth(&req)
    guard let (data, resp) = try? await URLSession.shared.data(for: req),
          (resp as? HTTPURLResponse)?.statusCode == 200 else { return nil }
    return try? JSONDecoder().decode(MetaResponse.self, from: data)
  }

  func fetchLocalStatus() async throws -> LocalStatusResponse {
    var req = URLRequest(url: config.baseURL.appendingPathComponent("/api/v1/local/status"))
    req.httpMethod = "GET"
    addAuth(&req)
    let (data, resp) = try await URLSession.shared.data(for: req)
    guard (resp as? HTTPURLResponse)?.statusCode == 200 else {
      throw URLError(.badServerResponse)
    }
    return try JSONDecoder().decode(LocalStatusResponse.self, from: data)
  }

  func sendLocalCommand(_ command: LocalCommandRequest) async throws {
    var req = URLRequest(url: config.baseURL.appendingPathComponent("/api/v1/local/commands"))
    req.httpMethod = "POST"
    req.setValue("application/json", forHTTPHeaderField: "content-type")
    addAuth(&req)
    req.httpBody = try JSONEncoder().encode(command)
    let (_, resp) = try await URLSession.shared.data(for: req)
    guard let status = (resp as? HTTPURLResponse)?.statusCode, (200..<300).contains(status) else {
      throw URLError(.badServerResponse)
    }
  }

  private func addAuth(_ req: inout URLRequest) {
    if let token = config.bearerToken, !token.isEmpty {
      req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
    }
  }
}
