import SwiftUI

struct ContentView: View {
    @EnvironmentObject var store: SjbisStore
    @State private var showingSettings = false
    @State private var selectedTab = 0

    var body: some View {
        TabView(selection: $selectedTab) {
            NavigationStack {
                NotificationListView()
                    .navigationTitle("SJBIS")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .topBarLeading) {
                            ConnectionIndicator(connected: store.isConnected)
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
        .task {
            await store.refresh()
            store.startSSE()
        }
        .onDisappear { store.stopSSE() }
        .refreshable { await store.refresh() }
    }
}

struct ConnectionIndicator: View {
    let connected: Bool
    var body: some View {
        HStack(spacing: 4) {
            Circle()
                .fill(connected ? Color.green : Color.red)
                .frame(width: 8, height: 8)
            Text(connected ? "Live" : "Offline")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
    }
}

#Preview {
    ContentView()
        .environmentObject(SjbisStore())
}
