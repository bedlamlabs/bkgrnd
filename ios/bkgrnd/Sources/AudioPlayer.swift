import AVFoundation
import MediaPlayer
import UIKit

@MainActor
final class AudioPlayer: ObservableObject {
  @Published private(set) var isPlaying = false
  @Published private(set) var currentTime: Double = 0
  @Published private(set) var duration: Double = 0

  private var player: AVPlayer?
  private var timeObserver: Any?

  func play(url: URL, headers: [String: String], title: String, artist: String, artworkURL: String?) async {
    configureAudioSession()

    let asset = AVURLAsset(url: url, options: headers.isEmpty ? nil : [AVURLAssetHTTPHeaderFieldsKey: headers])
    let item = AVPlayerItem(asset: asset)
    let p = AVPlayer(playerItem: item)
    player = p

    if let timeObserver {
      p.removeTimeObserver(timeObserver)
      self.timeObserver = nil
    }

    timeObserver = p.addPeriodicTimeObserver(forInterval: CMTime(seconds: 0.5, preferredTimescale: 600), queue: .main) { [weak self] t in
      guard let self else { return }
      self.currentTime = t.seconds
      if let d = p.currentItem?.duration.seconds, d.isFinite {
        self.duration = d
      }
    }

    updateNowPlaying(title: title, artist: artist, elapsed: 0, duration: 0, isPlaying: true, artworkURL: artworkURL)
    setupRemoteCommands()

    p.play()
    isPlaying = true
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
    updatePlaybackState()
  }

  func seek(to seconds: Double) {
    guard let p = player else { return }
    p.seek(to: CMTime(seconds: seconds, preferredTimescale: 600))
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
    let center = MPRemoteCommandCenter.shared()
    center.playCommand.isEnabled = true
    center.pauseCommand.isEnabled = true
    center.togglePlayPauseCommand.isEnabled = true

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
  }

  private func updateNowPlaying(title: String, artist: String, elapsed: Double, duration: Double, isPlaying: Bool, artworkURL: String?) {
    var info: [String: Any] = [
      MPMediaItemPropertyTitle: title,
      MPMediaItemPropertyArtist: artist,
      MPNowPlayingInfoPropertyElapsedPlaybackTime: elapsed,
      MPMediaItemPropertyPlaybackDuration: duration,
      MPNowPlayingInfoPropertyPlaybackRate: isPlaying ? 1.0 : 0.0,
    ]

    if let artworkURL, let url = URL(string: artworkURL) {
      Task.detached {
        if let data = try? Data(contentsOf: url), let img = UIImage(data: data) {
          let art = MPMediaItemArtwork(boundsSize: img.size) { _ in img }
          await MainActor.run {
            info[MPMediaItemPropertyArtwork] = art
            MPNowPlayingInfoCenter.default().nowPlayingInfo = info
          }
        } else {
          await MainActor.run {
            MPNowPlayingInfoCenter.default().nowPlayingInfo = info
          }
        }
      }
    } else {
      MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }
  }

  private func updatePlaybackState() {
    guard var info = MPNowPlayingInfoCenter.default().nowPlayingInfo else { return }
    info[MPNowPlayingInfoPropertyElapsedPlaybackTime] = currentTime
    info[MPNowPlayingInfoPropertyPlaybackRate] = isPlaying ? 1.0 : 0.0
    if duration > 0 {
      info[MPMediaItemPropertyPlaybackDuration] = duration
    }
    MPNowPlayingInfoCenter.default().nowPlayingInfo = info
  }
}
