import AVKit
import SwiftUI

/// The "Marquee" stage: artwork dissolves into a blurred fill of itself,
/// serif display type, brass accent. Dual-mode — controls this phone's
/// player, or the Mac when playback is remote.
struct NowPlayingView: View {
  @EnvironmentObject var appModel: AppModel
  @Environment(\.dismiss) private var dismiss

  // Scope-driven like the mini player: the stage controls whichever player
  // the active scope refers to.
  private var isLocal: Bool { appModel.scope == .recent && appModel.nowPlaying != nil }

  private var title: String {
    if isLocal { return appModel.nowPlaying?.title ?? "" }
    return appModel.remotePlayerStatus?.title ?? "Nothing playing"
  }

  private var subtitle: String {
    if isLocal { return appModel.nowPlaying?.channel ?? "" }
    let s = appModel.remotePlayerStatus
    return (s?.playlistTitle.isEmpty == false ? s?.playlistTitle : nil) ?? "On the Mac"
  }

  private var artURL: String? {
    if isLocal { return appModel.displayArtwork }
    let thumb = appModel.remotePlayerStatus?.thumbnail
    return (thumb?.isEmpty == false) ? thumb : nil
  }

  private var kicker: String {
    if !isLocal { return "Playing on the Mac" }
    if appModel.queue.count > 1 {
      let name = appModel.queueTitle.isEmpty ? "" : " — \(appModel.queueTitle)"
      return "No. \(appModel.queueIndex + 1) of \(appModel.queue.count)\(name)"
    }
    return appModel.nowPlaying?.isLive == true ? "Live stream" : ""
  }

  private var statusLabel: String {
    if !appModel.statusMessage.isEmpty { return appModel.statusMessage }
    if isLocal { return appModel.audioPlayer.isPlaying ? "streaming" : "paused" }
    return appModel.remotePlayerStatus?.isPaused == true ? "paused" : "streaming"
  }

  private var position: Double {
    isLocal ? appModel.audioPlayer.currentTime : (appModel.remotePlayerStatus?.position ?? 0)
  }

  private var duration: Double {
    isLocal ? appModel.audioPlayer.duration : (appModel.remotePlayerStatus?.duration ?? 0)
  }

  private var isPaused: Bool {
    isLocal ? !appModel.audioPlayer.isPlaying : (appModel.remotePlayerStatus?.isPaused ?? false)
  }

  var body: some View {
    ZStack {
      StageBackdrop(artURL: artURL)

      VStack(spacing: 0) {
        topRow
        Spacer()
        copyBlock
        progressBlock
        controlsRow
      }
      .padding(.horizontal, 26)
      .padding(.top, 16)
      .padding(.bottom, 24)
    }
    .preferredColorScheme(.dark)
  }

  private var topRow: some View {
    HStack {
      chip {
        HStack(spacing: 6) {
          Circle().fill(Color(red: 0.6, green: 0.91, blue: 0.68)).frame(width: 5, height: 5)
          Text(statusLabel.uppercased())
        }
      }
      Spacer()
      if isLocal && appModel.queue.count > 1 {
        chip { Text("QUEUE · \(appModel.queue.count)") }
      }
      AirPlayRoutePicker()
        .frame(width: 20, height: 20)
        .padding(6)
        .background(Color.black.opacity(0.28), in: Circle())
        .overlay(Circle().stroke(Color.white.opacity(0.14), lineWidth: 1))
        .opacity(isLocal ? 1 : 0.3)
        .disabled(!isLocal)
      Button { dismiss() } label: {
        ghostIcon("square.grid.2x2")
      }
    }
  }

  private var copyBlock: some View {
    VStack(alignment: .leading, spacing: 10) {
      Text(kicker)
        .font(.system(size: 15, design: .serif))
        .italic()
        .foregroundStyle(Theme.brass)
        .frame(minHeight: 19, alignment: .leading)
      Text(title)
        .font(.system(size: 34, weight: .medium, design: .serif))
        .foregroundStyle(Theme.text)
        .lineLimit(3)
        .minimumScaleFactor(0.7)
        .shadow(color: .black.opacity(0.6), radius: 17, y: 4)
      Text(subtitle.uppercased())
        .font(.system(size: 12, weight: .bold))
        .kerning(1.9)
        .foregroundStyle(Color.white.opacity(0.62))
        .lineLimit(1)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
  }

  private var progressBlock: some View {
    VStack(spacing: 2) {
      StageRail(
        position: position,
        duration: duration,
        onSeek: isLocal ? { appModel.audioPlayer.seek(to: $0) } : nil
      )
      HStack {
        Text(formatClock(position))
        Spacer()
        Text(duration > 0 ? formatClock(duration) : "--:--")
      }
      .font(.system(size: 11.5, weight: .semibold))
      .monospacedDigit()
      .foregroundStyle(Color.white.opacity(0.6))
    }
    .padding(.top, 26)
  }

  private var controlsRow: some View {
    HStack {
      Button {
        if isLocal { appModel.stopLocal(); dismiss() }
        else { Task { await appModel.stopRemote() } }
      } label: {
        Image(systemName: "stop.fill")
          .font(.system(size: 17, weight: .semibold))
          .foregroundStyle(Color(red: 1, green: 0.51, blue: 0.51).opacity(0.8))
          .frame(width: 46, height: 46)
      }

      Spacer()

      Button {
        if isLocal { appModel.audioPlayer.skip(-15) }
      } label: {
        stageButton("gobackward.15")
      }
      .opacity(isLocal ? 1 : 0.3)
      .disabled(!isLocal)

      Spacer()

      Button {
        if isLocal { appModel.audioPlayer.togglePlayPause() }
        else { Task { await appModel.toggleRemotePause() } }
      } label: {
        Image(systemName: isPaused ? "play.fill" : "pause.fill")
          .font(.system(size: 26, weight: .bold))
          .foregroundStyle(Color(red: 0.086, green: 0.075, blue: 0.102))
          .frame(width: 72, height: 72)
          .background(Theme.paper, in: Circle())
          .shadow(color: .black.opacity(0.5), radius: 20, y: 8)
      }

      Spacer()

      Button {
        if isLocal { appModel.audioPlayer.skip(15) }
      } label: {
        stageButton("goforward.15")
      }
      .opacity(isLocal ? 1 : 0.3)
      .disabled(!isLocal)

      Spacer()

      Button {
        if isLocal { appModel.toggleShuffle() }
      } label: {
        Image(systemName: "shuffle")
          .font(.system(size: 17, weight: .semibold))
          .foregroundStyle(appModel.shuffleEnabled ? Theme.brass : Color.white.opacity(0.55))
          .frame(width: 46, height: 46)
          .background(
            appModel.shuffleEnabled ? Theme.brass.opacity(0.16) : Color.clear,
            in: Circle()
          )
      }
      .opacity(isLocal ? 1 : 0.3)
      .disabled(!isLocal)
    }
    .padding(.top, 22)
  }

  private func chip<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
    content()
      .font(.system(size: 10, weight: .bold))
      .kerning(1.4)
      .foregroundStyle(Color.white.opacity(0.8))
      .padding(.horizontal, 10)
      .frame(height: 26)
      .background(Color.black.opacity(0.3), in: Capsule())
      .overlay(Capsule().stroke(Color.white.opacity(0.16), lineWidth: 1))
  }

  private func ghostIcon(_ systemName: String) -> some View {
    Image(systemName: systemName)
      .font(.system(size: 13, weight: .semibold))
      .foregroundStyle(Color.white.opacity(0.75))
      .frame(width: 32, height: 32)
      .background(Color.black.opacity(0.28), in: Circle())
      .overlay(Circle().stroke(Color.white.opacity(0.14), lineWidth: 1))
  }

  private func stageButton(_ systemName: String) -> some View {
    Image(systemName: systemName)
      .font(.system(size: 19, weight: .semibold))
      .foregroundStyle(Color.white.opacity(0.88))
      .frame(width: 46, height: 46)
      .background(Color.white.opacity(0.05), in: Circle())
      .overlay(Circle().stroke(Color.white.opacity(0.18), lineWidth: 1))
  }
}

/// Blurred self-fill behind a sharp masked crop of the same art.
private struct StageBackdrop: View {
  let artURL: String?

  var body: some View {
    GeometryReader { proxy in
      ZStack(alignment: .top) {
        Color(red: 0.043, green: 0.039, blue: 0.047)

        if let artURL {
          StageArtImage(urlString: artURL) { image in
            image.resizable().scaledToFill()
              .frame(width: proxy.size.width, height: proxy.size.height)
              .blur(radius: 52)
              .saturation(1.5)
              .brightness(-0.25)
              .clipped()
          }
          StageArtImage(urlString: artURL) { image in
            image.resizable().scaledToFill()
              .frame(width: proxy.size.width, height: proxy.size.height * 0.62, alignment: .top)
              .clipped()
              .mask(
                LinearGradient(
                  stops: [.init(color: .black, location: 0.62), .init(color: .clear, location: 1)],
                  startPoint: .top, endPoint: .bottom
                )
              )
          }
        } else {
          Text("b")
            .font(.system(size: 120, weight: .bold, design: .serif))
            .foregroundStyle(Color.white.opacity(0.10))
            .frame(width: proxy.size.width, height: proxy.size.height * 0.6)
        }

        LinearGradient(
          stops: [
            .init(color: Color(red: 0.03, green: 0.027, blue: 0.04).opacity(0.25), location: 0),
            .init(color: .clear, location: 0.26),
            .init(color: Color(red: 0.03, green: 0.027, blue: 0.04).opacity(0.42), location: 0.6),
            .init(color: Color(red: 0.03, green: 0.027, blue: 0.04).opacity(0.94), location: 1),
          ],
          startPoint: .top, endPoint: .bottom
        )
        .frame(width: proxy.size.width, height: proxy.size.height)
      }
    }
    .ignoresSafeArea()
  }
}

/// Loads maxres art with graceful fallback to the original thumbnail.
private struct StageArtImage<Content: View>: View {
  let urlString: String
  @ViewBuilder let content: (Image) -> Content
  @State private var maxresFailed = false

  var body: some View {
    let target = maxresFailed ? urlString : urlString.maxresThumbnail
    AsyncImage(url: URL(string: target)) { phase in
      switch phase {
      case .success(let image):
        content(image)
      case .failure:
        Color.clear.onAppear {
          if !maxresFailed { maxresFailed = true }
        }
      default:
        Color.clear
      }
    }
  }
}

/// Brass progress rail with live drag-to-seek: the knob follows the finger
/// and the seek commits on release.
private struct StageRail: View {
  let position: Double
  let duration: Double
  let onSeek: ((Double) -> Void)?

  @State private var dragFraction: Double?

  var body: some View {
    GeometryReader { proxy in
      let playFraction = duration > 0 ? min(max(position / duration, 0), 1) : 0
      let fraction = dragFraction ?? playFraction
      ZStack(alignment: .leading) {
        Capsule().fill(Color.white.opacity(0.22)).frame(height: 3)
        Capsule().fill(Theme.brass).frame(width: proxy.size.width * fraction, height: 3)
        Circle()
          .fill(Theme.brass)
          .frame(width: dragFraction != nil ? 17 : 13, height: dragFraction != nil ? 17 : 13)
          .background(Circle().fill(Theme.brass.opacity(0.22)).frame(width: 25, height: 25))
          .offset(x: proxy.size.width * fraction - 8)
      }
      .frame(maxHeight: .infinity)
      .contentShape(Rectangle())
      .highPriorityGesture(
        DragGesture(minimumDistance: 0)
          .onChanged { value in
            guard onSeek != nil, duration > 0 else { return }
            dragFraction = min(max(value.location.x / proxy.size.width, 0), 1)
          }
          .onEnded { value in
            guard let onSeek, duration > 0 else {
              dragFraction = nil
              return
            }
            let fraction = min(max(value.location.x / proxy.size.width, 0), 1)
            onSeek(duration * fraction)
            dragFraction = nil
          }
      )
    }
    .frame(height: 34)
  }
}

struct AirPlayRoutePicker: UIViewRepresentable {
  func makeUIView(context: Context) -> AVRoutePickerView {
    let v = AVRoutePickerView()
    v.activeTintColor = UIColor.white
    v.tintColor = UIColor(white: 1.0, alpha: 0.55)
    v.prioritizesVideoDevices = false
    return v
  }

  func updateUIView(_ uiView: AVRoutePickerView, context: Context) {}
}
