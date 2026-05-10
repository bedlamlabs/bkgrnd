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
    comps.queryItems = [URLQueryItem(name: "url", value: sourceURL.absoluteString)]
    return comps.url!
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

  private func addAuth(_ req: inout URLRequest) {
    if let token = config.bearerToken, !token.isEmpty {
      req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
    }
  }
}
