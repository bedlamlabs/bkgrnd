import SwiftUI

struct SearchView: View {
  @EnvironmentObject var appModel: AppModel
  @Environment(\.dismiss) private var dismiss
  @FocusState private var searchFocused: Bool

  var body: some View {
    ZStack {
      Theme.bg.ignoresSafeArea()

      VStack(alignment: .leading, spacing: 14) {
        HStack(spacing: 12) {
          Button { dismiss() } label: {
            Image(systemName: "chevron.left")
              .font(.system(size: 14, weight: .semibold))
              .foregroundStyle(Theme.muted)
              .frame(width: 34, height: 34)
              .background(Theme.card, in: Circle())
              .overlay(Circle().stroke(Theme.stroke, lineWidth: 1))
          }
          VStack(alignment: .leading, spacing: 2) {
            Text("Search")
              .font(.system(size: 16, weight: .bold))
              .foregroundStyle(Theme.text)
            Text(appModel.scope.rawValue)
              .font(.system(size: 12))
              .foregroundStyle(Theme.muted2)
          }
          Spacer()
        }
        .padding(.top, 14)

        HStack(spacing: 9) {
          Image(systemName: "magnifyingglass")
            .font(.system(size: 13, weight: .medium))
            .foregroundStyle(Theme.muted2)
          TextField("Search, or paste a YouTube / Spotify URL", text: $appModel.searchQuery)
            .font(.system(size: 14, weight: .medium))
            .foregroundStyle(Theme.text)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled(true)
            .submitLabel(.search)
            .focused($searchFocused)
            .onSubmit {
              Task {
                await appModel.performSearch()
                // Pasted URLs start playing immediately; close the sheet.
                if appModel.searchResults.isEmpty && appModel.nowPlaying != nil {
                  dismiss()
                }
              }
            }
          if !appModel.searchQuery.isEmpty {
            Button {
              appModel.searchQuery = ""
              appModel.searchResults = []
            } label: {
              Image(systemName: "xmark")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Theme.muted)
                .frame(width: 28, height: 28)
                .background(Color.white.opacity(0.06), in: Circle())
            }
          }
        }
        .padding(.horizontal, 13)
        .frame(height: 44)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.strokeStrong, lineWidth: 1))

        if appModel.isSearching {
          HStack(spacing: 8) {
            ProgressView().tint(Theme.muted)
            Text("Searching…")
              .font(.system(size: 12.5))
              .foregroundStyle(Theme.muted)
          }
          .padding(.horizontal, 4)
        }

        ScrollView {
          VStack(spacing: 10) {
            ForEach(appModel.searchResults, id: \.id) { item in
              SearchRow(item: item) {
                Task {
                  await appModel.playItem(item)
                  dismiss()
                }
              }
            }
          }
          .padding(.bottom, 40)
        }
      }
      .padding(.horizontal, 20)
    }
    .preferredColorScheme(.dark)
    .onAppear { searchFocused = true }
  }
}

private struct SearchRow: View {
  let item: PlaylistItem
  let playAction: () -> Void

  var body: some View {
    Button(action: playAction) {
      HStack(spacing: 12) {
        AsyncImage(url: URL(string: item.thumbnail ?? "")) { phase in
          if case .success(let image) = phase {
            image.resizable().scaledToFill()
          } else {
            Rectangle().fill(Color.white.opacity(0.06))
          }
        }
        .frame(width: 92, height: 52)
        .clipShape(RoundedRectangle(cornerRadius: 9))

        VStack(alignment: .leading, spacing: 3) {
          Text(item.title)
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(Theme.text)
            .lineLimit(2)
            .multilineTextAlignment(.leading)
          HStack(spacing: 6) {
            Text(item.channel ?? "")
              .font(.system(size: 11))
              .foregroundStyle(Theme.muted2)
              .lineLimit(1)
            if let chip = item.durationChip {
              Text(chip)
                .font(.system(size: 10, weight: .bold))
                .monospacedDigit()
                .foregroundStyle(Theme.muted)
            }
          }
        }

        Spacer(minLength: 6)

        Image(systemName: "play.fill")
          .font(.system(size: 12))
          .foregroundStyle(Theme.muted)
          .frame(width: 30, height: 30)
          .overlay(Circle().stroke(Theme.stroke, lineWidth: 1))
      }
      .padding(8)
      .background(Theme.card, in: RoundedRectangle(cornerRadius: 14))
      .overlay(RoundedRectangle(cornerRadius: 14).stroke(Theme.stroke, lineWidth: 1))
    }
    .buttonStyle(.plain)
  }
}
