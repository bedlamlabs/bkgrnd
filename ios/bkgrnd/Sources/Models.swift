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

struct PlaylistItem: Codable, Identifiable, Equatable {
  var id: String { url }
  var url: String
  var title: String
  var channel: String?
  var thumbnail: String?
  var addedAt: String?
  var duration: Double?
  var type: String?

  /// Live 24/7 streams report tiny/zero durations; only chip real ones.
  var durationChip: String? {
    guard let duration, duration > 60 else { return nil }
    return formatClock(duration)
  }

  var isLive: Bool { type == "stream" }

  var isSpotify: Bool {
    url.hasPrefix("https://open.spotify.com/") || url.hasPrefix("spotify:")
  }
}

struct SpotifyQueueResponse: Codable {
  var title: String
  var thumbnail: String
  var sourceCount: Int
  var matchedCount: Int
  var items: [SpotifyQueueItem]
}

struct SpotifyQueueItem: Codable {
  var url: String
  var videoId: String
  var title: String
  var thumbnail: String
  var channel: String
  var duration: Double?

  var playlistItem: PlaylistItem {
    PlaylistItem(url: url, title: title, channel: channel, thumbnail: thumbnail, addedAt: nil, duration: duration, type: nil)
  }
}
