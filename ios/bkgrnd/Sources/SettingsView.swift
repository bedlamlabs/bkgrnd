import SwiftUI

struct SettingsView: View {
  @EnvironmentObject var appModel: AppModel
  @Environment(\.dismiss) var dismiss

  @State private var baseURL: String = ""
  @State private var token: String = ""

  var body: some View {
    NavigationStack {
      ZStack {
        Color.black.ignoresSafeArea()
        Form {
          Section("WOPR") {
            TextField("Base URL", text: $baseURL)
              .textInputAutocapitalization(.never)
              .autocorrectionDisabled(true)
            SecureField("Bearer Token (optional)", text: $token)
          }

          Section {
            Button("Save") {
              appModel.updateServerSettings(baseURL: baseURL, token: token)
              dismiss()
            }
          }
        }
        .scrollContentBackground(.hidden)
      }
      .navigationTitle("Settings")
      .toolbar {
        ToolbarItem(placement: .topBarLeading) {
          Button("Close") { dismiss() }
        }
      }
      .onAppear {
        baseURL = appModel.baseURLString
        token = appModel.bearerToken
      }
    }
  }
}

