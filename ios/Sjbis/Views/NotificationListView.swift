import SwiftUI

struct NotificationListView: View {
    @EnvironmentObject var store: SjbisStore
    @State private var expandedIDs: Set<String> = []
    @State private var pendingMissedIDs: [String] = []
    @State private var confirmingClear = false
    @State private var clearingMissed = false
    @State private var clearError: String?

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 12) {
                if store.notifications.isEmpty && store.missed.isEmpty {
                    EmptyStateView(status: store.connectionStatus, error: store.connectionError)
                }
                ForEach(store.notifications) { notif in
                    FoldableNotificationCard(isExpanded: expanded(notif.id)) { isExpanded in
                        NavigationLink(value: notif) {
                            NotificationRowView(notif: notif, isExpanded: isExpanded)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                if !store.missed.isEmpty {
                    HStack {
                        Text("Missed while away")
                            .font(.headline)
                        Spacer()
                        if clearingMissed {
                            ProgressView()
                                .accessibilityLabel("Clearing missed requests")
                        } else {
                            Button("Clear All", role: .destructive) {
                                pendingMissedIDs = store.missed.map(\.id)
                                confirmingClear = true
                            }
                        }
                    }
                    ForEach(store.missed) { notif in
                        FoldableNotificationCard(isExpanded: expanded(notif.id)) { isExpanded in
                            VStack(alignment: .leading, spacing: 8) {
                                Text(notif.question_type == .celebrate ? "🎉 \(notif.agent_name)" : notif.agent_name)
                                    .font(.caption).foregroundStyle(.secondary)
                                Text(notif.question)
                                    .lineLimit(isExpanded ? nil : 2)
                                    .fixedSize(horizontal: false, vertical: true)
                                if let expired = notif.timed_out_at {
                                    Text("Expired \(expired.formatted(date: .abbreviated, time: .shortened))")
                                        .font(.caption).foregroundStyle(.secondary)
                                } else {
                                    Text("Expiry time unavailable").font(.caption).foregroundStyle(.secondary)
                                }
                                Button("Dismiss") { Task { await store.dismiss(notif.id) } }
                                    .buttonStyle(.bordered)
                            }
                        }
                    }
                }
            }
            .padding(.horizontal)
            .padding(.top, 8)
        }
        .navigationDestination(for: SjbisNotification.self) { notif in
            NotificationDetailView(notif: notif)
        }
        .confirmationDialog("Clear \(pendingMissedIDs.count) missed requests?", isPresented: $confirmingClear, titleVisibility: .visible) {
            Button("Clear Missed", role: .destructive) {
                let ids = pendingMissedIDs
                clearingMissed = true
                Task {
                    defer { clearingMissed = false }
                    do {
                        try await store.clearMissed(ids)
                        expandedIDs.subtract(ids)
                    } catch {
                        clearError = error.localizedDescription
                    }
                }
            }
        } message: {
            Text("You can still find these requests in History.")
        }
        .alert("Couldn’t clear missed requests", isPresented: Binding(
            get: { clearError != nil }, set: { if !$0 { clearError = nil } }
        )) {
            Button("OK", role: .cancel) { clearError = nil }
        } message: {
            Text(clearError ?? "")
        }
    }

    private func expanded(_ id: String) -> Binding<Bool> {
        Binding(
            get: { expandedIDs.contains(id) },
            set: { isExpanded in
                if isExpanded {
                    expandedIDs.insert(id)
                } else {
                    expandedIDs.remove(id)
                }
            }
        )
    }
}

struct FoldableNotificationCard<Content: View>: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Binding var isExpanded: Bool
    var foldLabel = "summary"
    @ViewBuilder var content: (Bool) -> Content

    var body: some View {
        HStack(alignment: .top, spacing: 4) {
            content(isExpanded)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button {
                withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.2)) {
                    isExpanded.toggle()
                }
            } label: {
                Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(isExpanded ? "Fold \(foldLabel)" : "Expand \(foldLabel)")
        }
        .padding(14)
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
    }
}

struct EmptyStateView: View {
    let status: ConnectionStatus
    let error: String?
    var body: some View {
        VStack(spacing: 12) {
            if let error {
                Image(systemName: "exclamationmark.triangle")
                    .font(.system(size: 48))
                    .foregroundStyle(.orange)
                Text("Connection Error")
                    .font(.headline)
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            } else if status == .connecting || status == .reconnecting {
                Text("Connecting to SJBIS…")
                    .font(.headline)
                    .foregroundStyle(.secondary)
            } else {
                Image(systemName: "checkmark.circle")
                    .font(.system(size: 48))
                    .foregroundStyle(.green)
                Text("No open notifications")
                    .font(.headline)
                .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 80)
    }
}

private struct NotificationRowView: View {
    let notif: SjbisNotification
    let isExpanded: Bool

    private var isCelebration: Bool { notif.question_type == .celebrate }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            if isCelebration {
                Text("🎉")
                    .font(.title2)
                    .frame(width: 36, height: 36)
                    .background(.purple.opacity(0.22), in: Circle())
                    .accessibilityHidden(true)
            } else {
                AgentBadge(agentName: notif.agent_name, urgency: notif.urgency)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(notif.question)
                    .font(.body)
                    .lineLimit(isExpanded ? nil : 2)
                    .fixedSize(horizontal: false, vertical: true)
                    .multilineTextAlignment(.leading)
                HStack(spacing: 6) {
                    Text(notif.agent_name)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if let inst = notif.instance {
                        Text("· \(inst)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Text(isCelebration ? "milestone" : notif.question_type.rawValue)
                        .font(.caption2)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.ultraThinMaterial)
                        .clipShape(Capsule())
                }
            }
        }
    }
}

struct AgentBadge: View {
    let agentName: String
    let urgency: Int

    private let hues: [Double] = [130, 175, 245, 295, 330, 25, 55, 90, 150, 210]
    private let lights: [Double] = [72, 78, 84]

    private var hash: UInt32 {
        agentName.unicodeScalars.reduce(UInt32(0)) { ($0 &* 31) &+ $1.value }
    }
    private var hue: Double { hues[Int(hash) % hues.count] }
    private var light: Double { lights[Int(hash >> 8) % lights.count] }
    private var color: Color {
        Color(hue: hue / 360, saturation: 0.55, brightness: light / 100)
    }

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 36, height: 36)
            .overlay(
                Text(agentName.prefix(2).uppercased())
                    .font(.caption2)
                    .fontWeight(.bold)
                    .foregroundStyle(.black)
            )
            .overlay(alignment: .topTrailing) {
                if urgency >= 4 {
                    Circle()
                        .fill(.red)
                        .frame(width: 10, height: 10)
                } else if urgency >= 2 {
                    Circle()
                        .fill(.orange)
                        .frame(width: 8, height: 8)
                }
            }
    }
}

#Preview {
    NotificationListView()
        .environmentObject(SjbisStore())
}
