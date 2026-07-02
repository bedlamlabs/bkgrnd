import SwiftUI

/// Ticket-style mini player with a brass progress line along its bottom
/// edge. Shows this phone's playback, or the Mac's when in Remote scope.
struct MiniPlayerBar: View {
  @EnvironmentObject var appModel: AppModel

  // The ribbon follows the selected scope (PWA parity): Recent shows this
  // phone's playback, Remote shows the Mac's — never the other way around.
  private var isLocal: Bool { appModel.scope == .recent && appModel.nowPlaying != nil }
  private var showRemote: Bool { appModel.scope == .remote && appModel.remoteIsPlaying }

  private var title: String {
    if isLocal { return appModel.nowPlaying?.title ?? "" }
    return appModel.remotePlayerStatus?.title ?? ""
  }

  private var subtitle: String {
    if isLocal {
      let elapsed = formatClock(appModel.audioPlayer.currentTime)
      return appModel.statusMessage.isEmpty ? "\(elapsed) · streaming" : appModel.statusMessage
    }
    return "On the Mac"
  }

  private var artURL: String? {
    isLocal ? appModel.nowPlaying?.thumbnail : appModel.remotePlayerStatus?.thumbnail
  }

  private var isPaused: Bool {
    isLocal ? !appModel.audioPlayer.isPlaying : (appModel.remotePlayerStatus?.isPaused ?? false)
  }

  private var progress: Double {
    let position = isLocal ? appModel.audioPlayer.currentTime : (appModel.remotePlayerStatus?.position ?? 0)
    let duration = isLocal ? appModel.audioPlayer.duration : (appModel.remotePlayerStatus?.duration ?? 0)
    guard duration > 0 else { return 0 }
    return min(max(position / duration, 0), 1)
  }

  var body: some View {
    if isLocal || showRemote {
      Button {
        appModel.showStage = true
      } label: {
        HStack(spacing: 11) {
          AsyncImage(url: URL(string: artURL ?? "")) { phase in
            if case .success(let image) = phase {
              image.resizable().scaledToFill()
            } else {
              Rectangle().fill(Color.white.opacity(0.08))
            }
          }
          .frame(width: 44, height: 44)
          .clipShape(RoundedRectangle(cornerRadius: 10))

          VStack(alignment: .leading, spacing: 2) {
            Text(title)
              .font(.system(size: 12.5, weight: .bold))
              .foregroundStyle(Theme.text)
              .lineLimit(1)
            Text(subtitle)
              .font(.system(size: 11))
              .foregroundStyle(Theme.muted)
              .lineLimit(1)
          }
          Spacer(minLength: 8)

          Button {
            if isLocal { appModel.audioPlayer.togglePlayPause() }
            else { Task { await appModel.toggleRemotePause() } }
          } label: {
            Image(systemName: isPaused ? "play.fill" : "pause.fill")
              .font(.system(size: 16, weight: .semibold))
              .foregroundStyle(.white)
              .frame(width: 34, height: 34)
          }
          .buttonStyle(.plain)
        }
        .padding(EdgeInsets(top: 8, leading: 8, bottom: 8, trailing: 12))
        .background(Color(red: 0.133, green: 0.141, blue: 0.165).opacity(0.96))
        .overlay(alignment: .bottom) {
          GeometryReader { proxy in
            ZStack(alignment: .leading) {
              Capsule().fill(Color.white.opacity(0.14))
              Capsule().fill(Theme.brass).frame(width: proxy.size.width * progress)
            }
            .frame(height: 2)
            .padding(.horizontal, 8)
            .frame(maxHeight: .infinity, alignment: .bottom)
          }
        }
        .clipShape(RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).stroke(Color.white.opacity(0.12), lineWidth: 1))
        .shadow(color: .black.opacity(0.45), radius: 17, y: 7)
      }
      .buttonStyle(.plain)
    }
  }
}
