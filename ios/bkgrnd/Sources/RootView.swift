import SwiftUI

/// Browse surface per the final direction: wordmark header, segmented
/// Recent/Remote scope, search row, card grid, mini player. The Marquee
/// stage presents as a full-screen cover.
struct RootView: View {
  @EnvironmentObject var appModel: AppModel
  @Environment(\.scenePhase) private var scenePhase
  @State private var showSettings = false
  @State private var showSearch = false

  var body: some View {
    ZStack(alignment: .bottom) {
      Theme.bg.ignoresSafeArea()

      ScrollView {
        VStack(alignment: .leading, spacing: 0) {
          header
          scopeSegment
          searchRow
          if appModel.scope == .remote && appModel.remoteStatus?.online != true {
            statusCard("Mac is unavailable. Open bkgrnd on the Mac to enable Remote.")
          }
          Text("Streams")
            .font(.system(size: 16, weight: .bold))
            .foregroundStyle(Theme.text)
            .padding(.horizontal, 20)
            .padding(.bottom, 10)
          BrowseGrid()
            .padding(.horizontal, 20)
          if let err = appModel.lastSyncError {
            statusCard("Offline — \(err)")
              .padding(.top, 12)
          }
          Color.clear.frame(height: 110) // room for the mini player
        }
      }
      .refreshable {
        await appModel.refreshPlaylists()
        await appModel.refreshRemoteStatus()
      }

      MiniPlayerBar()
        .padding(.horizontal, 12)
        .padding(.bottom, 10)
    }
    .preferredColorScheme(.dark)
    .sheet(isPresented: $showSettings) { SettingsView() }
    .sheet(isPresented: $showSearch) { SearchView() }
    .fullScreenCover(isPresented: $appModel.showStage) { NowPlayingView() }
    .task {
      await appModel.refreshPlaylists()
      await appModel.refreshRemoteStatus()
    }
    .onChange(of: scenePhase) { _, phase in
      if phase == .active {
        Task {
          await appModel.refreshPlaylists()
          await appModel.refreshRemoteStatus()
        }
      }
    }
    .task {
      // Poll the Mac's status so the Remote dot and mini player stay live.
      while !Task.isCancelled {
        try? await Task.sleep(nanoseconds: 3_000_000_000)
        await appModel.refreshRemoteStatus()
      }
    }
  }

  private var header: some View {
    HStack {
      (Text("b").foregroundStyle(Theme.coral) + Text("kgrnd").foregroundStyle(Theme.text))
        .font(.system(size: 22, weight: .heavy))
        .kerning(-0.5)
      Spacer()
      Button { showSettings = true } label: {
        Image(systemName: "gearshape")
          .font(.system(size: 14, weight: .semibold))
          .foregroundStyle(Theme.muted)
          .frame(width: 34, height: 34)
          .background(Theme.card, in: Circle())
          .overlay(Circle().stroke(Theme.stroke, lineWidth: 1))
      }
    }
    .padding(.horizontal, 20)
    .padding(.top, 6)
    .padding(.bottom, 12)
  }

  private var scopeSegment: some View {
    HStack(spacing: 0) {
      ForEach(PlaybackScope.allCases, id: \.self) { scope in
        Button {
          appModel.scope = scope
        } label: {
          HStack(spacing: 6) {
            Text(scope.rawValue)
              .font(.system(size: 13, weight: .semibold))
            if scope == .remote && appModel.remoteIsPlaying {
              Circle().fill(Color(red: 0.5, green: 0.88, blue: 0.63)).frame(width: 6, height: 6)
            }
          }
          .foregroundStyle(appModel.scope == scope ? Theme.text : Theme.muted)
          .frame(maxWidth: .infinity)
          .frame(height: 32)
          .background(
            appModel.scope == scope ? Color.white.opacity(0.10) : .clear,
            in: RoundedRectangle(cornerRadius: 9)
          )
        }
      }
    }
    .padding(3)
    .background(Theme.card, in: RoundedRectangle(cornerRadius: 12))
    .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.stroke, lineWidth: 1))
    .padding(.horizontal, 20)
    .padding(.bottom, 14)
  }

  private var searchRow: some View {
    Button { showSearch = true } label: {
      HStack(spacing: 9) {
        Image(systemName: "magnifyingglass")
          .font(.system(size: 13, weight: .medium))
        Text("Search, or paste a YouTube / Spotify URL")
          .font(.system(size: 13.5, weight: .medium))
        Spacer()
      }
      .foregroundStyle(Theme.muted2)
      .padding(.horizontal, 13)
      .frame(height: 40)
      .background(Theme.card, in: RoundedRectangle(cornerRadius: 12))
      .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.stroke, lineWidth: 1))
    }
    .padding(.horizontal, 20)
    .padding(.bottom, 18)
  }

  private func statusCard(_ message: String) -> some View {
    Text(message)
      .font(.system(size: 12.5))
      .foregroundStyle(Theme.muted)
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding(12)
      .background(Theme.card, in: RoundedRectangle(cornerRadius: 12))
      .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.stroke, lineWidth: 1))
      .padding(.horizontal, 20)
      .padding(.bottom, 14)
  }
}
