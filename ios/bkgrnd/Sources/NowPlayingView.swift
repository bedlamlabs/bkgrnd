import AVKit
import SwiftUI

struct NowPlayingView: View {
  @EnvironmentObject var appModel: AppModel

  private var accent: Color { Color(red: 0.79, green: 0.36, blue: 0.36) }

  var body: some View {
    NavigationStack {
      ZStack {
        Color.black.ignoresSafeArea()

        VStack(spacing: 14) {
          AsyncImage(url: URL(string: appModel.nowPlaying?.thumbnail ?? "")) { phase in
            switch phase {
            case .success(let image):
              image.resizable().scaledToFill()
            default:
              Rectangle().fill(.white.opacity(0.06))
            }
          }
          .frame(height: 260)
          .clipShape(RoundedRectangle(cornerRadius: 22))
          .padding(.horizontal, 16)
          .onTapGesture {
            // Implicit: tap artwork opens source
            if let s = appModel.nowPlaying?.url, let url = URL(string: s) {
              UIApplication.shared.open(url)
            }
          }

          Text("Tap artwork to open source")
            .font(.footnote)
            .foregroundStyle(.white.opacity(0.45))

          Text(appModel.nowPlaying?.title ?? "")
            .font(.title3.weight(.semibold))
            .foregroundStyle(.white.opacity(0.92))
            .multilineTextAlignment(.center)
            .lineLimit(3)
            .padding(.horizontal, 18)

          Text(appModel.nowPlaying?.channel ?? "")
            .font(.subheadline)
            .foregroundStyle(.white.opacity(0.55))

          VStack(spacing: 10) {
            Slider(
              value: Binding(
                get: { appModel.audioPlayer.currentTime },
                set: { appModel.audioPlayer.seek(to: $0) }
              ),
              in: 0...(max(appModel.audioPlayer.duration, 1))
            )
            .tint(accent)
            .padding(.horizontal, 18)

            HStack {
              Text(formatTime(appModel.audioPlayer.currentTime))
              Spacer()
              Text(formatTime(appModel.audioPlayer.duration))
            }
            .font(.caption)
            .foregroundStyle(.white.opacity(0.45))
            .padding(.horizontal, 18)
          }

          HStack(spacing: 14) {
            CircleButton(icon: "gobackward.15", filled: false, accent: accent) {
              appModel.audioPlayer.seek(to: max(appModel.audioPlayer.currentTime - 15, 0))
            }
            CircleButton(icon: "backward.fill", filled: false, accent: accent) {}
            CircleButton(icon: appModel.audioPlayer.isPlaying ? "pause.fill" : "play.fill", filled: true, accent: accent) {
              appModel.audioPlayer.togglePlayPause()
            }
            CircleButton(icon: "forward.fill", filled: false, accent: accent) {}
            CircleButton(icon: "goforward.15", filled: false, accent: accent) {
              appModel.audioPlayer.seek(to: appModel.audioPlayer.currentTime + 15)
            }
          }
          .padding(.top, 4)

          HStack {
            Image(systemName: "speaker.wave.2")
              .foregroundStyle(.white.opacity(0.55))
            Slider(value: .constant(0.6))
              .tint(accent.opacity(0.55))
            AirPlayRoutePicker()
              .frame(width: 44, height: 32)
          }
          .padding(.horizontal, 16)
          .padding(.top, 6)

          Spacer(minLength: 8)
        }
        .padding(.top, 10)
      }
      .navigationTitle("Now Playing")
      .toolbar {
        ToolbarItem(placement: .topBarTrailing) {
          Button {
            // placeholder menu
          } label: {
            Image(systemName: "ellipsis")
          }
        }
      }
    }
  }

  private func formatTime(_ t: Double) -> String {
    if !t.isFinite || t < 0 { return "0:00" }
    let s = Int(t)
    let m = s / 60
    let r = s % 60
    return "\(m):" + String(format: "%02d", r)
  }
}

private struct CircleButton: View {
  let icon: String
  let filled: Bool
  let accent: Color
  let action: () -> Void

  var body: some View {
    Button(action: action) {
      ZStack {
        RoundedRectangle(cornerRadius: 14)
          .fill(filled ? accent : .white.opacity(0.06))
          .overlay(
            RoundedRectangle(cornerRadius: 14)
              .stroke(.white.opacity(0.12), lineWidth: 1)
          )
        Image(systemName: icon)
          .foregroundStyle(filled ? .white : .white.opacity(0.8))
          .font(.system(size: 16, weight: .semibold))
      }
      .frame(width: filled ? 64 : 44, height: 44)
    }
    .buttonStyle(.plain)
  }
}

private struct AirPlayRoutePicker: UIViewRepresentable {
  func makeUIView(context: Context) -> AVRoutePickerView {
    let v = AVRoutePickerView()
    v.activeTintColor = UIColor.white
    v.tintColor = UIColor(white: 1.0, alpha: 0.55)
    v.prioritizesVideoDevices = false
    return v
  }

  func updateUIView(_ uiView: AVRoutePickerView, context: Context) {}
}

