import SwiftUI

@main
struct bkgrndApp: App {
  @StateObject private var appModel = AppModel()

  var body: some Scene {
    WindowGroup {
      RootView()
        .environmentObject(appModel)
    }
  }
}

