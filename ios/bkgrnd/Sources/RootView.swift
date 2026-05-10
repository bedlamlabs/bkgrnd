import SwiftUI

struct RootView: View {
  @EnvironmentObject var appModel: AppModel

  var body: some View {
    TabView {
      RecentMixesView()
        .tabItem {
          Image(systemName: "square.grid.2x2")
          Text("Recent")
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
    }
  }
}

