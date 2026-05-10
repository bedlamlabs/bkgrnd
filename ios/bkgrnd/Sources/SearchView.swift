import SwiftUI

struct SearchView: View {
  @EnvironmentObject var appModel: AppModel
  @State private var showSettings = false

  var body: some View {
    NavigationStack {
      ZStack {
        Color.black.ignoresSafeArea()

        VStack(alignment: .leading, spacing: 10) {
          TextField("Search", text: $appModel.searchQuery)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled(true)
            .submitLabel(.search)
            .onSubmit {
              Task { await appModel.performSearch() }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(.white.opacity(0.06))
            .overlay(
              RoundedRectangle(cornerRadius: 12)
                .stroke(.white.opacity(0.12), lineWidth: 1)
            )
            .clipShape(RoundedRectangle(cornerRadius: 12))
            .padding(.horizontal, 16)
            .padding(.top, 6)

          Text("RESULTS")
            .font(.caption.weight(.semibold))
            .foregroundStyle(.white.opacity(0.35))
            .padding(.horizontal, 16)

          ScrollView {
            VStack(spacing: 10) {
              ForEach(appModel.searchResults, id: \.id) { item in
                SearchRow(item: item) {
                  Task { await appModel.play(item) }
                } addAction: {
                  // TODO: add to playlist via WOPR; stub for now
                }
              }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 80)
          }
        }
      }
      .navigationTitle("Search")
      .toolbar {
        ToolbarItem(placement: .topBarTrailing) {
          Button { showSettings = true } label: {
            Image(systemName: "gearshape")
          }
        }
      }
      .sheet(isPresented: $showSettings) { SettingsView() }
    }
  }
}

private struct SearchRow: View {
  let item: PlaylistItem
  let playAction: () -> Void
  let addAction: () -> Void

  var body: some View {
    HStack(spacing: 10) {
      AsyncImage(url: URL(string: item.thumbnail ?? "")) { phase in
        switch phase {
        case .success(let image):
          image.resizable().scaledToFill()
        default:
          Rectangle().fill(.white.opacity(0.06))
        }
      }
      .frame(width: 52, height: 38)
      .clipShape(RoundedRectangle(cornerRadius: 10))
      .onTapGesture { playAction() }

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

      Button(action: addAction) {
        Image(systemName: "plus")
          .foregroundStyle(.white.opacity(0.75))
          .frame(width: 32, height: 32)
          .background(.white.opacity(0.06))
          .overlay(
            RoundedRectangle(cornerRadius: 10)
              .stroke(.white.opacity(0.12), lineWidth: 1)
          )
          .clipShape(RoundedRectangle(cornerRadius: 10))
      }
      .buttonStyle(.plain)
    }
    .padding(10)
    .background(.white.opacity(0.05))
    .overlay(
      RoundedRectangle(cornerRadius: 16)
        .stroke(.white.opacity(0.10), lineWidth: 1)
    )
    .clipShape(RoundedRectangle(cornerRadius: 16))
  }
}
