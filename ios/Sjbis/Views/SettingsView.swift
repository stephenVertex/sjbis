import SwiftUI

struct SettingsView: View {
    @EnvironmentObject var store: SjbisStore
    @State private var daemonURL = AppSettings.shared.daemonURL
    @AppStorage(AppSettings.mazelTovModeKey) private var mazelTovMode = false
    @Environment(\.dismiss) var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section("Daemon") {
                    TextField("Daemon URL", text: $daemonURL)
                        .keyboardType(.URL)
                        .autocorrectionDisabled()
                        .textInputAutocapitalization(.never)
                    Button("Save & Reconnect") {
                        if daemonURL == AppSettings.shared.daemonURL {
                            store.startSSE()
                        } else {
                            AppSettings.shared.daemonURL = daemonURL
                        }
                        dismiss()
                    }
                    .disabled(daemonURL.isEmpty)
                }
                Section("Status") {
                    LabeledContent("Connection", value: store.connectionStatus.rawValue)
                    if let err = store.connectionError {
                        LabeledContent("Error", value: err)
                    }
                    if let v = store.daemonVersion {
                        LabeledContent("Version", value: v)
                    }
                }
                Section("Notifications") {
                    LabeledContent("Open", value: "\(store.notifications.count)")
                    LabeledContent("History", value: "\(store.history.count)")
                }
                Section("Celebrations") {
                    Toggle("Mazel tov mode", isOn: $mazelTovMode)
                }
                ApprovalSetupSection()
                Section("About") {
                    LabeledContent("App", value: "SJBIS iOS 0.1.0")
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}
