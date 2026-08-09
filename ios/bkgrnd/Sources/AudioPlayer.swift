import AVFoundation
import Combine
import MediaPlayer
import OSLog
import UIKit

/// AVQueuePlayer-based engine. The next queue track is pre-resolved and
/// pre-enqueued so track transitions happen entirely inside AVFoundation —
/// with zero app-side network work at the boundary. (Doing the resolve at
/// AVPlayerItemDidPlayToEndTime got the app suspended mid-advance when
/// backgrounded: the session goes silent the moment a track ends.)
@MainActor
final class AudioPlayer: ObservableObject {
  @Published private(set) var isPlaying = false
  @Published private(set) var currentTime: Double = 0
  @Published private(set) var duration: Double = 0

  /// Fires when the queue is exhausted (last item played to its end).
  var onItemEnded: (() -> Void)?
  /// Fires when playback auto-advanced into the pre-enqueued next item.
  var onAdvanced: (() -> Void)?
  /// Fires on lock-screen/island prev-next taps.
  var onSkipRequested: ((_ forward: Bool) -> Void)?
  /// Fires with a human-readable message when the player item fails.
  var onPlaybackError: ((String) -> Void)?
  /// Fires once when an intended playback item makes no progress for 15s.
  var onPlaybackStalled: ((String) -> Void)?

  private let log = Logger(subsystem: "com.bedlamlabs.bkgrnd.ios", category: "player")
  private let player = AVQueuePlayer()
  private var timeObserver: Any?
  private var currentItemCancellable: AnyCancellable?
  private var itemCancellables = Set<AnyCancellable>()
  private var timeControlCancellable: AnyCancellable?
  private var watchdogTask: Task<Void, Never>?
  private var stallDetector = PlaybackStallDetector()
  private var commandsConfigured = false
  private var suppressItemCallbacks = false
  private var enqueuedNextItem: AVPlayerItem?

  private var nowPlayingTitle = ""
  private var nowPlayingArtist = ""
  private var nowPlayingArtwork: MPMediaItemArtwork?

  /// True if playback was active when an interruption (call/SMS/other audio)
  /// began — the cue to resume once the interruption ends.
  private var wasPlayingBeforeInterruption = false
  private var interruptionObserver: NSObjectProtocol?

  init() {
    // Start as soon as the first playable data arrives instead of letting
    // AVPlayer's stall-avoidance heuristic sit on a cold item filling its
    // forward buffer first — that heuristic was the bulk of the 12-15s
    // start delay on new streams. The proxy serves the head fast (~1MB in
    // 0.1s), so early-start rarely stalls.
    player.automaticallyWaitsToMinimizeStalling = false

    // Auto-resume after a call/SMS/other-audio interruption. Without this
    // observer nothing ever restarts the stream once iOS pauses it.
    interruptionObserver = NotificationCenter.default.addObserver(
      forName: AVAudioSession.interruptionNotification,
      object: nil,
      queue: .main
    ) { [weak self] note in
      guard
        let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
        let type = AVAudioSession.InterruptionType(rawValue: raw)
      else { return }
      let optionsRaw = (note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt) ?? 0
      let options = AVAudioSession.InterruptionOptions(rawValue: optionsRaw)
      Task { @MainActor in
        self?.handleInterruption(type: type, options: options)
      }
    }

    timeObserver = player.addPeriodicTimeObserver(
      forInterval: CMTime(seconds: 1.0, preferredTimescale: 600),
      queue: DispatchQueue.main
    ) { [weak self] (t: CMTime) in
      Task { @MainActor in
        guard let self else { return }
        self.currentTime = t.seconds
        if let d = self.player.currentItem?.duration.seconds, d.isFinite {
          self.duration = d
        }
        // Keep the island/lock-screen scrubber honest.
        self.pushNowPlayingInfo()
      }
    }

    currentItemCancellable = player.publisher(for: \.currentItem).sink { [weak self] item in
      Task { @MainActor in
        guard let self, !self.suppressItemCallbacks else { return }
        if item == nil {
          // Last item played out (or failed away) — the queue is done.
          self.isPlaying = false
          self.onItemEnded?()
        } else if let item, item === self.enqueuedNextItem {
          // AVQueuePlayer advanced into the pre-enqueued next track.
          self.enqueuedNextItem = nil
          self.currentTime = 0
          self.duration = 0
          self.observeStatus(of: item)
          self.onAdvanced?()
        }
      }
    }

    timeControlCancellable = player.publisher(for: \.timeControlStatus).sink { [weak self] status in
      Task { @MainActor in
        guard let self else { return }
        let waiting = self.player.reasonForWaitingToPlay?.rawValue ?? "none"
        self.log.info(
          "timeControl=\(Self.timeControlName(status), privacy: .public) waiting=\(waiting, privacy: .public) position=\(self.safeCurrentTime, privacy: .public)"
        )
      }
    }
  }

  deinit {
    watchdogTask?.cancel()
    if let interruptionObserver {
      NotificationCenter.default.removeObserver(interruptionObserver)
    }
  }

  private func makeItem(url: URL, headers: [String: String]) -> AVPlayerItem {
    let assetOptions: [String: Any]? = headers.isEmpty ? nil : ["AVURLAssetHTTPHeaderFieldsKey": headers]
    let asset = AVURLAsset(url: url, options: assetOptions)
    let item = AVPlayerItem(asset: asset)
    // Start once ~15s is buffered instead of gulping whole mid-size files.
    item.preferredForwardBufferDuration = 15
    return item
  }

  func play(url: URL, headers: [String: String], title: String, artist: String, artworkURL: String?) async {
    configureAudioSession()

    watchdogTask?.cancel()
    suppressItemCallbacks = true
    player.removeAllItems()
    enqueuedNextItem = nil
    let item = makeItem(url: url, headers: headers)
    player.insert(item, after: nil)
    suppressItemCallbacks = false

    currentTime = 0
    duration = 0
    setNowPlayingMeta(title: title, artist: artist, artworkURL: artworkURL)
    observeStatus(of: item)
    setupRemoteCommands()

    player.play()
    isPlaying = true
    startWatchdog(for: item)
    pushNowPlayingInfo()
  }

  /// Pre-enqueue the next queue track so the transition needs no app work.
  func enqueueNext(url: URL, headers: [String: String]) {
    if let stale = enqueuedNextItem {
      player.remove(stale)
      enqueuedNextItem = nil
    }
    guard let current = player.currentItem else { return }
    let item = makeItem(url: url, headers: headers)
    guard player.canInsert(item, after: current) else { return }
    player.insert(item, after: current)
    enqueuedNextItem = item
    log.info("pre-enqueued next track")
  }

  /// Update island/lock-screen metadata (used when auto-advancing).
  func setNowPlayingMeta(title: String, artist: String, artworkURL: String?) {
    nowPlayingTitle = title
    nowPlayingArtist = artist
    nowPlayingArtwork = nil
    loadArtwork(artworkURL)
    pushNowPlayingInfo()
  }

  func stop() {
    watchdogTask?.cancel()
    watchdogTask = nil
    suppressItemCallbacks = true
    player.pause()
    player.removeAllItems()
    enqueuedNextItem = nil
    suppressItemCallbacks = false
    itemCancellables.removeAll()
    isPlaying = false
    currentTime = 0
    duration = 0
    MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
  }

  func togglePlayPause() {
    guard player.currentItem != nil else { return }
    if isPlaying {
      player.pause()
      isPlaying = false
    } else {
      player.play()
      isPlaying = true
      stallDetector.reset(position: safeCurrentTime)
    }
    pushNowPlayingInfo()
  }

  func seek(to seconds: Double) {
    guard player.currentItem != nil else { return }
    player.seek(to: CMTime(seconds: max(seconds, 0), preferredTimescale: 600))
    currentTime = max(seconds, 0)
    stallDetector.reset(position: max(seconds, 0))
    pushNowPlayingInfo()
  }

  func skip(_ delta: Double) {
    seek(to: currentTime + delta)
  }

  private func observeStatus(of item: AVPlayerItem) {
    itemCancellables.removeAll()
    item.publisher(for: \.status).sink { [weak self, weak item] status in
      Task { @MainActor in
        guard let self else { return }
        switch status {
        case .failed:
          self.reportFailure(item: item, fallback: "playback failed")
        case .readyToPlay:
          self.log.info("player item ready")
          // With automaticallyWaitsToMinimizeStalling = false, a play() issued
          // before the item was ready is a no-op and AVPlayer will NOT auto-start
          // when data arrives (the default true does). Kick it now that the item
          // is playable — gated on our intent so a pre-ready pause still holds.
          if self.isPlaying, item === self.player.currentItem {
            self.player.play()
          }
        default:
          break
        }
      }
    }.store(in: &itemCancellables)

    NotificationCenter.default.publisher(for: .AVPlayerItemPlaybackStalled, object: item)
      .sink { [weak self, weak item] _ in
        Task { @MainActor in
          guard let self, let item else { return }
          self.log.warning("AVPlayerItemPlaybackStalled notification")
          self.logDiagnostics(item: item, reason: "playback-stalled-notification")
        }
      }
      .store(in: &itemCancellables)

    NotificationCenter.default.publisher(for: .AVPlayerItemFailedToPlayToEndTime, object: item)
      .sink { [weak self, weak item] note in
        Task { @MainActor in
          guard let self else { return }
          let error = note.userInfo?[AVPlayerItemFailedToPlayToEndTimeErrorKey] as? Error
          self.reportFailure(item: item, error: error, fallback: "playback ended unexpectedly")
        }
      }
      .store(in: &itemCancellables)

    NotificationCenter.default.publisher(for: .AVPlayerItemNewErrorLogEntry, object: item)
      .sink { [weak self, weak item] _ in
        Task { @MainActor in
          guard let self, let item, let event = item.errorLog()?.events.last else { return }
          self.log.error(
            "errorLog status=\(event.errorStatusCode) domain=\(event.errorDomain, privacy: .public) comment=\(event.errorComment ?? "", privacy: .public)"
          )
        }
      }
      .store(in: &itemCancellables)
  }

  private func startWatchdog(for item: AVPlayerItem) {
    watchdogTask?.cancel()
    stallDetector.reset(position: safeCurrentTime)
    watchdogTask = Task { [weak self, weak item] in
      while !Task.isCancelled {
        do {
          try await Task.sleep(for: .seconds(2))
        } catch {
          return
        }
        guard let self, let item, self.player.currentItem === item else { return }
        let position = self.safeCurrentTime
        guard self.stallDetector.evaluate(position: position, intendsToPlay: self.isPlaying) else {
          continue
        }

        self.log.error("playback watchdog: no progress for 15 seconds")
        self.logDiagnostics(item: item, reason: "no-progress-timeout")
        self.player.pause()
        self.isPlaying = false
        self.pushNowPlayingInfo()
        let message = position < 1 ? "Stream did not start." : "Playback stalled."
        self.onPlaybackStalled?(message)
        return
      }
    }
  }

  private var safeCurrentTime: Double {
    let seconds = player.currentTime().seconds
    return seconds.isFinite ? max(seconds, 0) : 0
  }

  private func reportFailure(
    item: AVPlayerItem?,
    error explicitError: Error? = nil,
    fallback: String
  ) {
    let error = explicitError ?? item?.error
    let detail = error.map(String.init(describing:)) ?? fallback
    log.error("player item failed: \(detail, privacy: .public)")
    if let item { logDiagnostics(item: item, reason: "item-failed") }
    isPlaying = false
    onPlaybackError?((error as NSError?)?.localizedDescription ?? fallback)
  }

  private func logDiagnostics(item: AVPlayerItem, reason: String) {
    let waiting = player.reasonForWaitingToPlay?.rawValue ?? "none"
    let loaded = Self.describeRanges(item.loadedTimeRanges)
    let seekable = Self.describeRanges(item.seekableTimeRanges)
    let itemError = item.error.map(String.init(describing:)) ?? "none"
    log.error(
      "diagnostic reason=\(reason, privacy: .public) item=\(Self.itemStatusName(item.status), privacy: .public) timeControl=\(Self.timeControlName(self.player.timeControlStatus), privacy: .public) waiting=\(waiting, privacy: .public) position=\(self.safeCurrentTime, privacy: .public) duration=\(item.duration.seconds, privacy: .public) empty=\(item.isPlaybackBufferEmpty) full=\(item.isPlaybackBufferFull) keepUp=\(item.isPlaybackLikelyToKeepUp) loaded=\(loaded, privacy: .public) seekable=\(seekable, privacy: .public) error=\(itemError, privacy: .public)"
    )
    if let event = item.accessLog()?.events.last {
      log.error(
        "accessLog requests=\(event.numberOfMediaRequests) bytes=\(event.numberOfBytesTransferred) transfer=\(event.transferDuration, privacy: .public) observedBitrate=\(event.observedBitrate, privacy: .public) indicatedBitrate=\(event.indicatedBitrate, privacy: .public)"
      )
    }
    if let event = item.errorLog()?.events.last {
      log.error(
        "latestError status=\(event.errorStatusCode) domain=\(event.errorDomain, privacy: .public) comment=\(event.errorComment ?? "", privacy: .public)"
      )
    }
  }

  private static func describeRanges(_ values: [NSValue]) -> String {
    values.map { value in
      let range = value.timeRangeValue
      return "\(range.start.seconds)-\(CMTimeAdd(range.start, range.duration).seconds)"
    }.joined(separator: ",")
  }

  private static func itemStatusName(_ status: AVPlayerItem.Status) -> String {
    switch status {
    case .unknown: return "unknown"
    case .readyToPlay: return "ready"
    case .failed: return "failed"
    @unknown default: return "future"
    }
  }

  private static func timeControlName(_ status: AVPlayer.TimeControlStatus) -> String {
    switch status {
    case .paused: return "paused"
    case .waitingToPlayAtSpecifiedRate: return "waiting"
    case .playing: return "playing"
    @unknown default: return "future"
    }
  }

  /// Pause on interruption-began; on interruption-ended, resume if we had been
  /// playing. We resume whenever playback was active (not only when the system
  /// sets `.shouldResume`) — for calls that flag is unreliable, and this is a
  /// music app where the user expects the stream to pick back up.
  private func handleInterruption(type: AVAudioSession.InterruptionType, options: AVAudioSession.InterruptionOptions) {
    switch type {
    case .began:
      wasPlayingBeforeInterruption = isPlaying
      isPlaying = false
      stallDetector.reset(position: safeCurrentTime)
      pushNowPlayingInfo()
    case .ended:
      guard wasPlayingBeforeInterruption else { return }
      wasPlayingBeforeInterruption = false
      resumeAfterInterruption(retries: 3)
    @unknown default:
      break
    }
  }

  /// Reactivating the session right after a phone call can transiently fail
  /// (AVAudioSession error 560557684 / `.cannotStartPlaying`); retry briefly.
  private func resumeAfterInterruption(retries: Int) {
    do {
      try AVAudioSession.sharedInstance().setActive(true)
      player.play()
      isPlaying = true
      stallDetector.reset(position: safeCurrentTime)
      pushNowPlayingInfo()
    } catch {
      guard retries > 0 else {
        log.error("resume after interruption failed: \(String(describing: error), privacy: .public)")
        return
      }
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { [weak self] in
        Task { @MainActor in self?.resumeAfterInterruption(retries: retries - 1) }
      }
    }
  }

  private func configureAudioSession() {
    let session = AVAudioSession.sharedInstance()
    do {
      try session.setCategory(.playback, mode: .default, options: [.allowAirPlay])
      try session.setActive(true)
    } catch {
      log.error("audio session activation failed: \(String(describing: error), privacy: .public)")
    }
  }

  private func setupRemoteCommands() {
    guard !commandsConfigured else { return }
    commandsConfigured = true

    let center = MPRemoteCommandCenter.shared()
    center.playCommand.isEnabled = true
    center.pauseCommand.isEnabled = true
    center.togglePlayPauseCommand.isEnabled = true
    center.skipBackwardCommand.isEnabled = true
    center.skipForwardCommand.isEnabled = true
    center.skipBackwardCommand.preferredIntervals = [15]
    center.skipForwardCommand.preferredIntervals = [15]
    center.changePlaybackPositionCommand.isEnabled = true
    center.nextTrackCommand.isEnabled = true
    center.previousTrackCommand.isEnabled = true

    center.playCommand.addTarget { [weak self] _ in
      guard let self else { return .commandFailed }
      if !self.isPlaying { self.togglePlayPause() }
      return .success
    }
    center.pauseCommand.addTarget { [weak self] _ in
      guard let self else { return .commandFailed }
      if self.isPlaying { self.togglePlayPause() }
      return .success
    }
    center.togglePlayPauseCommand.addTarget { [weak self] _ in
      self?.togglePlayPause()
      return .success
    }
    center.skipBackwardCommand.addTarget { [weak self] _ in
      self?.skip(-15)
      return .success
    }
    center.skipForwardCommand.addTarget { [weak self] _ in
      self?.skip(15)
      return .success
    }
    center.changePlaybackPositionCommand.addTarget { [weak self] event in
      guard let self, let event = event as? MPChangePlaybackPositionCommandEvent else { return .commandFailed }
      self.seek(to: event.positionTime)
      return .success
    }
    center.nextTrackCommand.addTarget { [weak self] _ in
      self?.onSkipRequested?(true)
      return .success
    }
    center.previousTrackCommand.addTarget { [weak self] _ in
      self?.onSkipRequested?(false)
      return .success
    }
  }

  private func loadArtwork(_ artworkURL: String?) {
    guard let artworkURL else { return }
    // Prefer full-res art on the island/lock screen; fall back to what we have.
    let candidates = [artworkURL.maxresThumbnail, artworkURL]
    Task.detached {
      for candidate in candidates {
        guard let url = URL(string: candidate),
              let (data, _) = try? await URLSession.shared.data(from: url),
              let img = UIImage(data: data),
              img.size.width > 200
        else { continue }
        let art = MPMediaItemArtwork(boundsSize: img.size) { _ in img }
        await MainActor.run { [weak self] in
          self?.nowPlayingArtwork = art
          self?.pushNowPlayingInfo()
        }
        return
      }
    }
  }

  private func pushNowPlayingInfo() {
    var info: [String: Any] = [
      MPMediaItemPropertyTitle: nowPlayingTitle,
      MPMediaItemPropertyArtist: nowPlayingArtist,
      MPNowPlayingInfoPropertyElapsedPlaybackTime: currentTime,
      MPNowPlayingInfoPropertyPlaybackRate: isPlaying ? 1.0 : 0.0,
      MPNowPlayingInfoPropertyIsLiveStream: !(duration > 0),
    ]
    if duration > 0 {
      info[MPMediaItemPropertyPlaybackDuration] = duration
    }
    if let nowPlayingArtwork {
      info[MPMediaItemPropertyArtwork] = nowPlayingArtwork
    }
    MPNowPlayingInfoCenter.default().nowPlayingInfo = info
  }
}
