import CarPlay
import Combine
import OSLog
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
  private var rootList: CPListTemplate?
  private var lastRenderedKeys: [String] = []
  private var cancellables = Set<AnyCancellable>()
  private let log = Logger(subsystem: "com.bedlamlabs.bkgrnd.ios", category: "carplay")

  private init() {}

  func connect(_ interfaceController: CPInterfaceController) {
    self.interfaceController = interfaceController
    let list = makeListTemplate()
    rootList = list
    interfaceController.setRootTemplate(list, animated: false, completion: nil)

    // Rebuild rows when the library loads/changes. Use DispatchQueue.main, not
    // RunLoop.main — RunLoop.main delivery can be dropped while CarPlay is
    // interacting, which left the list blank if data arrived after connect.
    model.$recentPlaylist
      .receive(on: DispatchQueue.main)
      .sink { [weak self] _ in self?.reloadRootTemplate() }
      .store(in: &cancellables)

    // Always pull the latest library on (re)connect, then render whatever we
    // have now (covers both "already loaded" and "loads a moment later").
    Task { [weak self] in
      await self?.model.refreshPlaylists()
      self?.reloadRootTemplate()
    }
    reloadRootTemplate()
  }

  func disconnect() {
    cancellables.removeAll()
    interfaceController = nil
    rootList = nil
    lastRenderedKeys = []
  }

  // MARK: - Templates

  private func makeListTemplate() -> CPListTemplate {
    let section = CPListSection(items: model.gridItems.map(makeListItem))
    let template = CPListTemplate(title: "bkgrnd", sections: [section])
    template.emptyViewSubtitleVariants = ["Open bkgrnd on your phone to sign in"]
    return template
  }

  private func reloadRootTemplate() {
    guard let rootList else { return }
    let items = model.gridItems
    let keys = items.map(\.url)
    guard keys != lastRenderedKeys else { return }
    lastRenderedKeys = keys
    log.info("carplay list render: \(items.count, privacy: .public) items")
    rootList.updateSections([CPListSection(items: items.map(makeListItem))])
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
