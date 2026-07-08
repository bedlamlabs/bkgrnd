import CarPlay
import UIKit

/// Scene delegate for the CarPlay screen. Declared in Info.plist under the
/// `CPTemplateApplicationSceneSessionRoleApplication` scene role; the SwiftUI
/// `WindowGroup` continues to own the phone window scene independently.
///
/// NOTE: A CarPlay *audio* app also requires the `com.apple.developer.carplay-audio`
/// entitlement, which Apple grants on request via the developer portal. Until
/// that entitlement is granted and wired (see project.yml / bkgrnd.entitlements),
/// this scene will not activate in a real vehicle or the CarPlay Simulator.
final class CarPlaySceneDelegate: UIResponder, CPTemplateApplicationSceneDelegate {
  func templateApplicationScene(
    _ scene: CPTemplateApplicationScene,
    didConnect interfaceController: CPInterfaceController
  ) {
    Task { @MainActor in
      CarPlayController.shared.connect(interfaceController)
    }
  }

  func templateApplicationScene(
    _ scene: CPTemplateApplicationScene,
    didDisconnectInterfaceController interfaceController: CPInterfaceController
  ) {
    Task { @MainActor in
      CarPlayController.shared.disconnect()
    }
  }
}
