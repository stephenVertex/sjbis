import SwiftUI

struct NotificationListView: View {
    @EnvironmentObject var store: SjbisStore

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 12) {
                if store.notifications.isEmpty {
                    EmptyStateView(error: store.connectionError)
                }
                ForEach(store.notifications) { notif in
                    NavigationLink(value: notif) {
                        NotificationRowView(notif: notif)
                    }
                }
            }
            .padding(.horizontal)
            .padding(.top, 8)
        }
        .navigationDestination(for: SjbisNotification.self) { notif in
            NotificationDetailView(notif: notif)
        }
    }
}

struct EmptyStateView: View {
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

struct NotificationRowView: View {
    let notif: SjbisNotification

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            AgentBadge(agentName: notif.agent_name, urgency: notif.urgency)
            VStack(alignment: .leading, spacing: 4) {
                Text(notif.question)
                    .font(.body)
                    .lineLimit(3)
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
                    Text(notif.question_type.rawValue)
                        .font(.caption2)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.ultraThinMaterial)
                        .clipShape(Capsule())
                }
            }
        }
        .padding(14)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 12))
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
