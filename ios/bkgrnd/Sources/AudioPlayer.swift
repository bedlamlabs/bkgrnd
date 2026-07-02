import AVFoundation
import MediaPlayer
import UIKit

@MainActor
final class AudioPlayer: ObservableObject {
  @Published private(set) var isPlaying = false
  @Published private(set) var currentTime: Double = 0
  @Published private(set) var duration: Double = 0

  /// Fires when the current item plays to its end — drives queue auto-advance.
  var onItemEnded: (() -> Void)?
  /// Fires on lock-screen/island prev-next taps.
  var onSkipRequested: ((_ forward: Bool) -> Void)?

  private var player: AVPlayer?
  private var timeObserver: Any?
  private var endObserver: NSObjectProtocol?
  private var commandsConfigured = false

  private var nowPlayingTitle = ""
  private var nowPlayingArtist = ""
  private var nowPlayingArtwork: MPMediaItemArtwork?

  func play(url: URL, headers: [String: String], title: String, artist: String, artworkURL: String?) async {
    configureAudioSession()

    let assetOptions: [String: Any]? = headers.isEmpty ? nil : ["AVURLAssetHTTPHeaderFieldsKey": headers]
    let asset = AVURLAsset(url: url, options: assetOptions)
    let item = AVPlayerItem(asset: asset)
    let p = AVPlayer(playerItem: item)

    teardownObservers()
    player = p
    currentTime = 0
    duration = 0
    nowPlayingTitle = title
    nowPlayingArtist = artist
    nowPlayingArtwork = nil

    timeObserver = p.addPeriodicTimeObserver(
      forInterval: CMTime(seconds: 1.0, preferredTimescale: 600),
      queue: DispatchQueue.main
    ) { [weak self, weak p] (t: CMTime) in
      Task { @MainActor in
        guard let self else { return }
        self.currentTime = t.seconds
        if let d = p?.currentItem?.duration.seconds, d.isFinite {
          self.duration = d
        }
        // Keep the island/lock-screen scrubber honest.
        self.pushNowPlayingInfo()
      }
    }

    endObserver = NotificationCenter.default.addObserver(
      forName: .AVPlayerItemDidPlayToEndTime,
      object: item,
      queue: .main
    ) { [weak self] _ in
      Task { @MainActor in
        guard let self else { return }
        self.isPlaying = false
        self.onItemEnded?()
      }
    }

    setupRemoteCommands()
    loadArtwork(artworkURL)

    p.play()
    isPlaying = true
    pushNowPlayingInfo()
  }

  func stop() {
    teardownObservers()
    player?.pause()
    player = nil
    isPlaying = false
    currentTime = 0
    duration = 0
    MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
  }

  func togglePlayPause() {
    guard let p = player else { return }
    if isPlaying {
      p.pause()
      isPlaying = false
    } else {
      p.play()
      isPlaying = true
    }
    pushNowPlayingInfo()
  }

  func seek(to seconds: Double) {
    guard let p = player else { return }
    p.seek(to: CMTime(seconds: max(seconds, 0), preferredTimescale: 600))
    currentTime = max(seconds, 0)
    pushNowPlayingInfo()
  }

  func skip(_ delta: Double) {
    seek(to: currentTime + delta)
  }

  private func teardownObservers() {
    if let timeObserver {
      player?.removeTimeObserver(timeObserver)
      self.timeObserver = nil
    }
    if let endObserver {
      NotificationCenter.default.removeObserver(endObserver)
      self.endObserver = nil
    }
  }

  private func configureAudioSession() {
    let session = AVAudioSession.sharedInstance()
    do {
      try session.setCategory(.playback, mode: .default, options: [.allowAirPlay])
      try session.setActive(true)
    } catch {
      // ignore
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
