import Foundation

struct PlaylistDoc: Codable {
  var version: Int
  var updatedAt: String
  var playlists: [Playlist]
}

struct Playlist: Codable, Identifiable {
  var id: String
  var name: String
  var items: [PlaylistItem]
}

struct PlaylistItem: Codable, Identifiable {
  var id: String { url }
  var url: String
  var title: String
  var channel: String?
  var thumbnail: String?
  var addedAt: String?
}

