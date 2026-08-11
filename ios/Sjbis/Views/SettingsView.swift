import SwiftUI

struct SettingsView: View {
    @EnvironmentObject var store: SjbisStore
    @State private var daemonURL = AppSettings.shared.daemonURL
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
                        AppSettings.shared.daemonURL = daemonURL
                        store.stopSSE()
                        Task {
                            await store.refresh()
                            store.startSSE()
                        }
                        dismiss()
                    }
                    .disabled(daemonURL.isEmpty)
                }
                Section("Status") {
                    LabeledContent("Connected", value: store.isConnected ? "Yes" : "No")
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
