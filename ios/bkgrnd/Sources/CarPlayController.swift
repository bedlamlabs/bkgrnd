import CarPlay
import Combine
import UIKit

/// Drives the CarPlay interface from the shared `AppModel`. Presents the Recent
/// grid as a browsable list; tapping a row starts playback on the same engine
/// the phone UI uses and pushes CarPlay's system Now Playing screen (which reads
/// the MPNowPlayingInfoCenter / MPRemoteCommandCenter wiring AudioPlayer already
/// sets up, so transport controls work for free).
@MainActor
final class CarPlayController {
  static let shared = CarPlayController()

  private let model = AppModel.shared
  private var interfaceController: CPInterfaceController?
  private var cancellables = Set<AnyCancellable>()

  private init() {}

  func connect(_ interfaceController: CPInterfaceController) {
    self.interfaceController = interfaceController
    interfaceController.setRootTemplate(makeListTemplate(), animated: false, completion: nil)

    // Rebuild rows whenever the Recent playlist loads or re-sorts.
    model.$recentPlaylist
      .receive(on: RunLoop.main)
      .sink { [weak self] _ in self?.reloadRootTemplate() }
      .store(in: &cancellables)
    model.objectWillChange
      .receive(on: RunLoop.main)
      .sink { [weak self] _ in self?.reloadRootTemplate() }
      .store(in: &cancellables)

    if model.recentPlaylist == nil {
      Task { await model.refreshPlaylists() }
    }
  }

  func disconnect() {
    cancellables.removeAll()
    interfaceController = nil
  }

  // MARK: - Templates

  private func makeListTemplate() -> CPListTemplate {
    let section = CPListSection(items: model.gridItems.map(makeListItem))
    let template = CPListTemplate(title: "bkgrnd", sections: [section])
    template.emptyViewSubtitleVariants = ["Open bkgrnd on your phone to sign in"]
    return template
  }

  private func reloadRootTemplate() {
    guard let rootList = interfaceController?.rootTemplate as? CPListTemplate else { return }
    let section = CPListSection(items: model.gridItems.map(makeListItem))
    rootList.updateSections([section])
  }

  private func makeListItem(_ item: PlaylistItem) -> CPListItem {
    let listItem = CPListItem(text: item.title, detailText: item.channel)
    listItem.handler = { [weak self] _, completion in
      guard let self else { completion(); return }
      Task { @MainActor in
        await self.model.playItem(item)
        self.pushNowPlaying()
        completion()
      }
    }
    loadArtwork(item.thumbnail) { [weak listItem] image in
      listItem?.setImage(image)
    }
    return listItem
  }

  private func pushNowPlaying() {
    guard let interfaceController else { return }
    // Avoid stacking duplicate Now Playing templates on repeat taps.
    if interfaceController.topTemplate !== CPNowPlayingTemplate.shared {
      interfaceController.pushTemplate(CPNowPlayingTemplate.shared, animated: true, completion: nil)
    }
  }

  // MARK: - Artwork

  private func loadArtwork(_ urlString: String?, completion: @escaping @MainActor (UIImage) -> Void) {
    guard let urlString, let url = URL(string: urlString) else { return }
    Task.detached {
      guard
        let (data, _) = try? await URLSession.shared.data(from: url),
        let image = UIImage(data: data)
      else { return }
      // CarPlay list thumbnails are small; downscale to keep memory sane.
      let scaled = image.scaledToFit(maxDimension: 88)
      await MainActor.run { completion(scaled) }
    }
  }
}

private extension UIImage {
  func scaledToFit(maxDimension: CGFloat) -> UIImage {
    let longest = max(size.width, size.height)
    guard longest > maxDimension else { return self }
    let scale = maxDimension / longest
    let target = CGSize(width: size.width * scale, height: size.height * scale)
    let renderer = UIGraphicsImageRenderer(size: target)
    return renderer.image { _ in draw(in: CGRect(origin: .zero, size: target)) }
  }
}
