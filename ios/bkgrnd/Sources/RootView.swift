import SwiftUI

struct RootView: View {
  @EnvironmentObject var appModel: AppModel

  var body: some View {
    TabView {
      RecentView()
        .tabItem {
          Image(systemName: "square.grid.2x2")
          Text("Recent")
        }

      RemoteView()
        .tabItem {
          Image(systemName: "macbook")
          Text("Remote")
        }

      SearchView()
        .tabItem {
          Image(systemName: "magnifyingglass")
          Text("Search")
        }
    }
    .overlay(alignment: .bottom) {
      if appModel.nowPlaying != nil {
        MiniPlayerBar()
          .padding(.horizontal, 12)
          .padding(.bottom, 8)
      }
    }
    .task {
      await appModel.refreshPlaylists()
      await appModel.refreshRemoteStatus()
    }
  }
}

struct RemoteView: View {
  @EnvironmentObject var appModel: AppModel
  @State private var showSettings = false

  private var status: WOPRClient.LocalPlayerStatus? {
    appModel.remoteStatus?.status
  }

  var body: some View {
    NavigationStack {
      ZStack {
        Color.black.ignoresSafeArea()

        ScrollView {
          VStack(alignment: .leading, spacing: 16) {
            Text("REMOTE")
              .font(.caption.weight(.semibold))
              .foregroundStyle(.white.opacity(0.35))
              .padding(.horizontal, 16)

            VStack(alignment: .leading, spacing: 12) {
              Text(appModel.remoteStatus?.online == true ? "Mac connected" : "Mac unavailable")
                .font(.headline)
                .foregroundStyle(.white.opacity(0.92))

              if let status, status.isPlaying {
                Text(status.title.isEmpty ? "Playing on Mac" : status.title)
                  .font(.title3.weight(.semibold))
                  .foregroundStyle(.white.opacity(0.9))
                  .lineLimit(3)

                Text(status.playlistTitle.isEmpty ? "Remote" : status.playlistTitle)
                  .font(.subheadline)
                  .foregroundStyle(.white.opacity(0.55))

                HStack(spacing: 12) {
                  Button {
                    Task { await appModel.toggleRemotePause() }
                  } label: {
                    Label(status.isPaused ? "Play" : "Pause", systemImage: status.isPaused ? "play.fill" : "pause.fill")
                  }

                  Button(role: .destructive) {
                    Task { await appModel.stopRemote() }
                  } label: {
                    Label("Stop", systemImage: "stop.fill")
                  }
                }
                .buttonStyle(.bordered)
              } else {
                Text("Choose an item below to start playback on the Mac.")
                  .font(.subheadline)
                  .foregroundStyle(.white.opacity(0.55))
              }

              if let err = appModel.lastRemoteError {
                Text(err)
                  .font(.footnote)
                  .foregroundStyle(.white.opacity(0.45))
              }
            }
            .padding(16)
            .background(.white.opacity(0.05))
            .overlay(
              RoundedRectangle(cornerRadius: 16)
                .stroke(.white.opacity(0.10), lineWidth: 1)
            )
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .padding(.horizontal, 16)

            VStack(spacing: 10) {
              ForEach((appModel.recentPlaylist?.items ?? []), id: \.id) { item in
                Button {
                  Task { await appModel.playRemote(item) }
                } label: {
                  HStack(spacing: 10) {
                    AsyncImage(url: URL(string: item.thumbnail ?? "")) { phase in
                      switch phase {
                      case .success(let image):
                        image.resizable().scaledToFill()
                      default:
                        Rectangle().fill(.white.opacity(0.06))
                      }
                    }
                    .frame(width: 56, height: 42)
                    .clipShape(RoundedRectangle(cornerRadius: 10))

                    VStack(alignment: .leading, spacing: 3) {
                      Text(item.title)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.white.opacity(0.9))
                        .lineLimit(2)
                      Text(item.channel ?? "")
                        .font(.caption)
                        .foregroundStyle(.white.opacity(0.55))
                        .lineLimit(1)
                    }
                    Spacer()
                    Image(systemName: "play.fill")
                      .foregroundStyle(.white.opacity(0.7))
                  }
                  .padding(10)
                  .background(.white.opacity(0.05))
                  .overlay(
                    RoundedRectangle(cornerRadius: 16)
                      .stroke(.white.opacity(0.10), lineWidth: 1)
                  )
                  .clipShape(RoundedRectangle(cornerRadius: 16))
                }
                .buttonStyle(.plain)
              }
            }
            .padding(.horizontal, 16)
          }
          .padding(.vertical, 12)
        }
      }
      .navigationTitle("Remote")
      .toolbar {
        ToolbarItem(placement: .topBarTrailing) {
          Button { showSettings = true } label: {
            Image(systemName: "gearshape")
          }
        }
      }
      .sheet(isPresented: $showSettings) { SettingsView() }
      .refreshable {
        await appModel.refreshRemoteStatus()
      }
    }
  }
}
