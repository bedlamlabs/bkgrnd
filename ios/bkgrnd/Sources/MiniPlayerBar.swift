import SwiftUI

struct MiniPlayerBar: View {
  @EnvironmentObject var appModel: AppModel
  @State private var showNowPlaying = false

  var body: some View {
    Button {
      showNowPlaying = true
    } label: {
      HStack(spacing: 10) {
        AsyncImage(url: URL(string: appModel.nowPlaying?.thumbnail ?? "")) { phase in
          switch phase {
          case .success(let image):
            image.resizable().scaledToFill()
          default:
            Rectangle().fill(.white.opacity(0.08))
          }
        }
        .frame(width: 36, height: 36)
        .clipShape(RoundedRectangle(cornerRadius: 8))

        VStack(alignment: .leading, spacing: 2) {
          Text(appModel.nowPlaying?.title ?? "")
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(.white.opacity(0.9))
            .lineLimit(1)
          Text(appModel.nowPlaying?.channel ?? "")
            .font(.caption)
            .foregroundStyle(.white.opacity(0.55))
            .lineLimit(1)
        }
        Spacer()

        Button {
          appModel.audioPlayer.togglePlayPause()
        } label: {
          ZStack {
            RoundedRectangle(cornerRadius: 10)
              .fill(Color(red: 0.79, green: 0.36, blue: 0.36)) // subtle cherry accent
            Image(systemName: appModel.audioPlayer.isPlaying ? "pause.fill" : "play.fill")
              .foregroundStyle(.white)
          }
          .frame(width: 44, height: 36)
        }
        .buttonStyle(.plain)
      }
      .padding(10)
      .background(.black.opacity(0.65))
      .overlay(
        RoundedRectangle(cornerRadius: 16)
          .stroke(.white.opacity(0.12), lineWidth: 1)
      )
      .clipShape(RoundedRectangle(cornerRadius: 16))
    }
    .buttonStyle(.plain)
    .sheet(isPresented: $showNowPlaying) {
      NowPlayingView()
    }
  }
}

