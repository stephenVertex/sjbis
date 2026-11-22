import SwiftUI

struct ContentView: View {
    @EnvironmentObject var store: SjbisStore
    @Environment(\.scenePhase) private var scenePhase
    @ObservedObject private var push = PushNotifications.shared
    @ObservedObject private var settings = AppSettings.shared
    @State private var showingSettings = false
    @State private var selectedTab = 0
    @State private var queuePath: [SjbisNotification] = []

    var body: some View {
        TabView(selection: $selectedTab) {
            NavigationStack(path: $queuePath) {
                NotificationListView()
                    .navigationTitle("SJBIS")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .topBarLeading) {
                            ConnectionIndicator(status: store.connectionStatus)
                        }
                        ToolbarItem(placement: .topBarTrailing) {
                            Button { showingSettings = true } label: {
                                Image(systemName: "gearshape")
                            }
                        }
                    }
            }
            .tabItem {
                Label("Queue", systemImage: "tray")
            }
            .badge(store.notifications.count)
            .tag(0)

            NavigationStack {
                HistoryView()
                    .navigationTitle("History")
                    .navigationBarTitleDisplayMode(.inline)
            }
            .tabItem {
                Label("History", systemImage: "clock")
            }
            .tag(1)
        }
        .sheet(isPresented: $showingSettings) {
            SettingsView()
        }
        .task(id: scenePhase) {
            if scenePhase == .active {
                push.syncRegistration()
                store.startSSE()
            } else if scenePhase == .background {
                store.stopSSE()
            }
        }
        .task(id: push.openedID) {
            guard let id = push.openedID else { return }
            if let notification = await store.fetchNotification(id), !Task.isCancelled {
                showingSettings = false
                selectedTab = 0
                queuePath = [notification]
            }
            push.clearOpenedID(id)
        }
        .onChange(of: settings.daemonURL) {
            push.syncRegistration()
            store.startSSE()
        }
        .onDisappear { store.stopSSE() }
        .refreshable { await store.refresh() }
    }
}

struct ConnectionIndicator: View {
    let status: ConnectionStatus

    var body: some View {
        HStack(spacing: 4) {
            switch status {
            case .connecting, .reconnecting:
                ProgressView()
                    .controlSize(.mini)
                    .accessibilityHidden(true)
            case .live, .offline:
                Circle()
                    .fill(status == .live ? Color.green : Color.red)
                    .frame(width: 8, height: 8)
                    .accessibilityHidden(true)
            }
            Text(status.rawValue)
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .fixedSize()
    }
}

#Preview {
    ContentView()
        .environmentObject(SjbisStore())
}
