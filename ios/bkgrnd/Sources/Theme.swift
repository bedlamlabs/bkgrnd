import SwiftUI

/// Design tokens for the final direction: coral/clean browse surface,
/// brass/serif "Marquee" stage. Mirrors server/web/styles.css.
enum Theme {
  static let bg = Color(red: 0.051, green: 0.055, blue: 0.067)          // #0d0e11
  static let card = Color(red: 0.090, green: 0.094, blue: 0.114)        // #17181d
  static let coral = Color(red: 0.933, green: 0.427, blue: 0.455)       // #ee6d74
  static let brass = Color(red: 0.847, green: 0.655, blue: 0.357)       // #d8a75b
  static let paper = Color.white.opacity(0.93)

  static let stroke = Color.white.opacity(0.08)
  static let strokeStrong = Color.white.opacity(0.16)
  static let text = Color.white.opacity(0.94)
  static let muted = Color.white.opacity(0.55)
  static let muted2 = Color.white.opacity(0.36)
}

extension String {
  /// YouTube thumbnails ladder: swap any size for maxres (1280x720).
  /// Callers should fall back to the original if maxres 404s.
  var maxresThumbnail: String {
    replacingOccurrences(
      of: #"/(default|mqdefault|hqdefault|sddefault)\.jpg"#,
      with: "/maxresdefault.jpg",
      options: .regularExpression
    )
  }
}

func formatClock(_ seconds: Double?) -> String {
  guard let seconds, seconds.isFinite, seconds >= 0 else { return "--:--" }
  let total = Int(seconds)
  let h = total / 3600
  let m = (total % 3600) / 60
  let s = total % 60
  if h > 0 { return String(format: "%d:%02d:%02d", h, m, s) }
  return String(format: "%d:%02d", m, s)
}
