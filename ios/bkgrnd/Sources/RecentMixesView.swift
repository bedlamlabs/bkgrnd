import SwiftUI

struct RecentMixesView: View {
  @EnvironmentObject var appModel: AppModel
  @State private var showSettings = false
  @State private var selectedFilter: String = "Recent"

  private let filters = ["Recent", "Live", "Mixes"]
  private let columns = [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)]

  var body: some View {
    NavigationStack {
      ZStack {
        Color.black.ignoresSafeArea()

        ScrollView {
          VStack(alignment: .leading, spacing: 12) {
            Text("RECENT MIXES")
              .font(.caption.weight(.semibold))
              .foregroundStyle(.white.opacity(0.35))
              .padding(.horizontal, 16)

            ScrollView(.horizontal, showsIndicators: false) {
              HStack(spacing: 8) {
                ForEach(filters, id: \.self) { f in
                  Button {
                    selectedFilter = f
                  } label: {
                    Text(f)
                      .font(.subheadline)
                      .foregroundStyle(.white.opacity(selectedFilter == f ? 0.9 : 0.7))
                      .padding(.horizontal, 12)
                      .padding(.vertical, 6)
                      .background(.white.opacity(0.06))
                      .overlay(
                        RoundedRectangle(cornerRadius: 18)
                          .stroke(.white.opacity(0.12), lineWidth: 1)
                      )
                      .clipShape(RoundedRectangle(cornerRadius: 18))
                  }
                }
              }
              .padding(.horizontal, 16)
            }

            LazyVGrid(columns: columns, spacing: 12) {
              ForEach((appModel.recentMixes?.items ?? []), id: \.id) { item in
                MixTile(item: item)
                  .onTapGesture {
                    Task { await appModel.play(item) }
                  }
              }
            }
            .padding(.horizontal, 16)

            if let err = appModel.lastSyncError {
              Text("Offline — using cached playlists (\(err))")
                .font(.footnote)
                .foregroundStyle(.white.opacity(0.45))
                .padding(.horizontal, 16)
                .padding(.top, 8)
            }
          }
          .padding(.vertical, 12)
        }
      }
      .navigationTitle("Recent Mixes")
      .toolbar {
        ToolbarItem(placement: .topBarTrailing) {
          Button {
            showSettings = true
          } label: {
            Image(systemName: "gearshape")
          }
        }
      }
      .sheet(isPresented: $showSettings) {
        SettingsView()
      }
      .refreshable {
        await appModel.refreshPlaylists()
      }
    }
  }
}

private struct MixTile: View {
  let item: PlaylistItem

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      AsyncImage(url: URL(string: item.thumbnail ?? "")) { phase in
        switch phase {
        case .success(let image):
          image.resizable().scaledToFill()
        default:
          Rectangle().fill(.white.opacity(0.06))
        }
      }
      .frame(height: 110)
      .clipShape(RoundedRectangle(cornerRadius: 14))

      Text(item.title)
        .font(.subheadline.weight(.semibold))
        .foregroundStyle(.white.opacity(0.9))
        .lineLimit(2)

      Text(item.channel ?? "")
        .font(.caption)
        .foregroundStyle(.white.opacity(0.55))
        .lineLimit(1)

      Text("PLAY")
        .font(.caption.weight(.semibold))
        .foregroundStyle(.white.opacity(0.55))
        .padding(.top, 2)
    }
    .padding(12)
    .background(.white.opacity(0.05))
    .overlay(
      RoundedRectangle(cornerRadius: 18)
        .stroke(.white.opacity(0.10), lineWidth: 1)
    )
    .clipShape(RoundedRectangle(cornerRadius: 18))
  }
}

