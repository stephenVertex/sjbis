import SwiftUI

struct HistoryView: View {
    @EnvironmentObject var store: SjbisStore

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 8) {
                if store.history.isEmpty {
                    VStack(spacing: 8) {
                        Image(systemName: "clock")
                            .font(.system(size: 36))
                            .foregroundStyle(.secondary)
                        Text("No history yet")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.top, 60)
                }
                ForEach(store.history) { notif in
                    HistoryRowView(notif: notif)
                }
            }
            .padding(.horizontal)
            .padding(.top, 8)
        }
    }
}

struct HistoryRowView: View {
    let notif: SjbisNotification

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            statusIcon
            VStack(alignment: .leading, spacing: 3) {
                Text(notif.question)
                    .font(.subheadline)
                    .lineLimit(2)
                HStack(spacing: 6) {
                    Text(notif.agent_name)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    statusBadge
                }
                if let answer = notif.answer_label ?? notif.answer {
                    Text("→ \(answer)")
                        .font(.caption)
                        .foregroundStyle(answerColor)
                }
            }
            Spacer()
            if let at = notif.answered_at {
                Text(at.formatted(.relative(presentation: .named)))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(10)
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 10))
    }

    @ViewBuilder
    private var statusIcon: some View {
        switch notif.status {
        case .answered:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.green)
        case .timed_out:
            Image(systemName: "clock.badge.exclamationmark.fill")
                .foregroundStyle(.orange)
        case .dismissed:
            Image(systemName: "xmark.circle.fill")
                .foregroundStyle(.secondary)
        case .cancelled:
            Image(systemName: "slash.circle.fill")
                .foregroundStyle(.red)
        default:
            Image(systemName: "circle")
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var statusBadge: some View {
        switch notif.status {
        case .answered:
            Text("answered")
                .font(.caption2)
                .padding(.horizontal, 5)
                .padding(.vertical, 1)
                .background(.green.opacity(0.2))
                .clipShape(Capsule())
        case .timed_out:
            Text("timed out")
                .font(.caption2)
                .padding(.horizontal, 5)
                .padding(.vertical, 1)
                .background(.orange.opacity(0.2))
                .clipShape(Capsule())
        case .dismissed:
            Text("dismissed")
                .font(.caption2)
                .padding(.horizontal, 5)
                .padding(.vertical, 1)
                .background(.secondary.opacity(0.2))
                .clipShape(Capsule())
        case .cancelled:
            Text("cancelled")
                .font(.caption2)
                .padding(.horizontal, 5)
                .padding(.vertical, 1)
                .background(.red.opacity(0.2))
                .clipShape(Capsule())
        default:
            EmptyView()
        }
    }

    private var answerColor: Color {
        switch notif.status {
        case .timed_out: .orange
        case .dismissed: .secondary
        default: .primary
        }
    }
}
