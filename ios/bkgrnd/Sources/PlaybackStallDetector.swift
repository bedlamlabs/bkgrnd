import Foundation

/// Small, deterministic policy object kept separate from AVPlayer so the
/// no-progress timeout can be unit tested without media/network dependencies.
struct PlaybackStallDetector {
  let timeout: TimeInterval
  let minimumProgress: Double

  private var lastProgressAt: Date?
  private var lastPosition: Double = 0
  private var hasReported = false

  init(timeout: TimeInterval = 15, minimumProgress: Double = 0.25) {
    self.timeout = timeout
    self.minimumProgress = minimumProgress
  }

  mutating func reset(at now: Date = Date(), position: Double) {
    lastProgressAt = now
    lastPosition = position.isFinite ? max(position, 0) : 0
    hasReported = false
  }

  mutating func evaluate(
    at now: Date = Date(),
    position: Double,
    intendsToPlay: Bool
  ) -> Bool {
    guard let lastProgressAt else {
      reset(at: now, position: position)
      return false
    }

    let safePosition = position.isFinite ? max(position, 0) : lastPosition
    if !intendsToPlay {
      self.lastProgressAt = now
      lastPosition = safePosition
      return false
    }

    if safePosition >= lastPosition + minimumProgress {
      self.lastProgressAt = now
      lastPosition = safePosition
      return false
    }

    guard !hasReported, now.timeIntervalSince(lastProgressAt) >= timeout else {
      return false
    }
    hasReported = true
    return true
  }
}

/// Claims at most one automatic recovery for the current user playback
/// attempt. Reset only when the user starts/changes/stops an item.
struct PlaybackRecoveryPolicy {
  private var attemptedURL: String?

  mutating func claimRetry(for url: String) -> Bool {
    guard attemptedURL != url else { return false }
    attemptedURL = url
    return true
  }

  mutating func reset() {
    attemptedURL = nil
  }
}
