import SwiftUI

/// The 2-column stream card grid. Cards route through AppModel.playItem,
/// which respects the active scope (play here vs. play on the Mac).
struct BrowseGrid: View {
  @EnvironmentObject var appModel: AppModel

  private let columns = [GridItem(.flexible(), spacing: 13), GridItem(.flexible(), spacing: 13)]

  var body: some View {
    LazyVGrid(columns: columns, spacing: 13) {
      ForEach(appModel.gridItems, id: \.id) { item in
        MixCard(item: item)
          .onTapGesture {
            Task { await appModel.playItem(item) }
          }
      }
    }
  }
}

struct MixCard: View {
  let item: PlaylistItem

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      ZStack {
        Rectangle()
          .fill(LinearGradient(colors: [Color(white: 0.14), Color(white: 0.07)], startPoint: .topLeading, endPoint: .bottomTrailing))
        AsyncImage(url: URL(string: item.thumbnail ?? "")) { phase in
          if case .success(let image) = phase {
            image.resizable().scaledToFill()
          } else {
            Text("b")
              .font(.system(size: 30, weight: .black))
              .foregroundStyle(Color.white.opacity(0.16))
          }
        }
      }
      .aspectRatio(16 / 9, contentMode: .fit)
      .clipped()
      .overlay(alignment: .topLeading) {
        if item.isLive {
          Text("LIVE")
            .font(.system(size: 9, weight: .heavy))
            .kerning(0.6)
            .foregroundStyle(Color(red: 1, green: 0.70, blue: 0.70))
            .padding(.horizontal, 6)
            .frame(height: 18)
            .background(Color.black.opacity(0.55), in: RoundedRectangle(cornerRadius: 5))
            .padding(7)
        }
      }
      .overlay(alignment: .bottomTrailing) {
        if !item.isLive, let chip = item.durationChip {
          Text(chip)
            .font(.system(size: 10, weight: .bold))
            .monospacedDigit()
            .foregroundStyle(Color.white.opacity(0.85))
            .padding(.horizontal, 6)
            .frame(height: 18)
            .background(Color.black.opacity(0.55), in: RoundedRectangle(cornerRadius: 5))
            .padding(7)
        }
      }

      VStack(alignment: .leading, spacing: 3) {
        Text(item.title)
          .font(.system(size: 12.5, weight: .semibold))
          .foregroundStyle(Theme.text)
          .lineLimit(2)
          .frame(minHeight: 32, alignment: .topLeading)
        Text(item.channel ?? " ")
          .font(.system(size: 11))
          .foregroundStyle(Theme.muted2)
          .lineLimit(1)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding(EdgeInsets(top: 9, leading: 11, bottom: 11, trailing: 11))
    }
    .background(Theme.card)
    .clipShape(RoundedRectangle(cornerRadius: 14))
    .overlay(RoundedRectangle(cornerRadius: 14).stroke(Theme.stroke, lineWidth: 1))
    .contentShape(RoundedRectangle(cornerRadius: 14))
  }
}
