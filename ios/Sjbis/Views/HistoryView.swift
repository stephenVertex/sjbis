import SwiftUI

struct HistoryView: View {
    @EnvironmentObject var store: SjbisStore
    @State private var olderRequests: [SjbisNotification] = []
    @State private var loadingOlder = false
    @State private var historyComplete = false
    @State private var historyError: String?
    private let pageSize = 50

    private var requests: [SjbisNotification] {
        let recentIDs = Set(store.history.map(\.id))
        return store.history + olderRequests.filter { !recentIDs.contains($0.id) }
    }

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 8) {
                if requests.isEmpty {
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
                ForEach(requests) { notif in
                    HistoryRowView(notif: notif)
                }
                if let historyError {
                    Text(historyError).font(.caption).foregroundStyle(.secondary)
                }
                if loadingOlder {
                    ProgressView("Loading older requests…")
                } else if !historyComplete, requests.count >= pageSize {
                    Button("Load Older Requests") { Task { await loadOlder() } }
                        .padding(.vertical, 8)
                }
            }
            .padding(.horizontal)
            .padding(.top, 8)
        }
        .onChange(of: store.daemonURL) {
            olderRequests = []
            historyComplete = false
            historyError = nil
        }
    }

    private func loadOlder() async {
        let displayed = requests
        guard let cursor = displayed.last?.id, !loadingOlder else { return }
        let server = store.daemonURL
        loadingOlder = true
        historyError = nil
        defer { loadingOlder = false }
        do {
            let page = try await store.fetchOlderHistory(before: cursor, limit: pageSize)
            guard store.daemonURL == server else { return }
            let displayedIDs = Set(displayed.map(\.id))
            let older = page.filter { !displayedIDs.contains($0.id) }
            olderRequests = displayed + older
            historyComplete = older.count < pageSize
        } catch {
            guard store.daemonURL == server else { return }
            historyError = error.localizedDescription
        }
    }
}

struct HistoryRowView: View {
    let notif: SjbisNotification
    @State private var expanded = false

    var body: some View {
        FoldableNotificationCard(isExpanded: $expanded, foldLabel: "request") { isExpanded in
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .top, spacing: 10) {
                    statusIcon
                    VStack(alignment: .leading, spacing: 3) {
                        Text(notif.question)
                            .font(.subheadline)
                            .lineLimit(isExpanded ? nil : 2)
                            .fixedSize(horizontal: false, vertical: true)
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
                        if let at = notif.answered_at ?? notif.timed_out_at {
                            Text(at.formatted(.relative(presentation: .named)))
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                if isExpanded {
                    HistoryRequestDetails(notif: notif)
                }
            }
        }
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

private struct HistoryRequestDetails: View {
    let notif: SjbisNotification

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Divider()
            LabeledContent("Requested", value: notif.created_at.formatted(date: .abbreviated, time: .shortened))
            if let markdown = notif.detail_markdown {
                if let formatted = try? AttributedString(markdown: markdown, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)) {
                    Text(formatted).textSelection(.enabled)
                } else {
                    Text(markdown).textSelection(.enabled)
                }
            } else if let detail = notif.detail {
                Text(detail).textSelection(.enabled)
            }
            if let note = notif.note, !note.isEmpty {
                Text("Note").font(.caption).foregroundStyle(.secondary)
                Text(note).textSelection(.enabled)
            }
            if notif.question_type == .approval, notif.status == .answered {
                HistoryApprovalGrants(askID: notif.id)
            }
        }
        .font(.callout)
    }
}

private struct HistoryApprovalGrants: View {
    let askID: String
    @EnvironmentObject private var store: SjbisStore
    @State private var grants: [ApprovalGrantResponse]?
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let grants {
                if grants.isEmpty {
                    Text("No signed grant recorded").foregroundStyle(.secondary)
                }
                ForEach(grants, id: \.envelope.id) { grant in
                    let record = grant.envelope.record
                    Divider()
                    LabeledContent("Grant") { Text(grant.envelope.id).monospaced().textSelection(.enabled) }
                    LabeledContent("Decision", value: record.decision.label)
                    LabeledContent("Signed by") { Text(record.signer).textSelection(.enabled) }
                    if let expires = try? Date.ISO8601FormatStyle().parse(record.expires_at) {
                        LabeledContent(grant.status?.expired == true ? "Expired" : "Expires",
                                       value: expires.formatted(date: .abbreviated, time: .shortened))
                    }
                    if let revokedBy = grant.status?.revoked_by {
                        LabeledContent("Revoked by", value: revokedBy)
                    }
                    if let claimedBy = grant.status?.claimed_by {
                        LabeledContent("Claimed by", value: claimedBy)
                    }
                }
            } else if let error {
                Text("Couldn’t load the signed grant. \(error)").foregroundStyle(.secondary)
                Button("Retry") { Task { await loadGrants() } }
            } else {
                ProgressView("Loading signed grant…")
            }
        }
        .task(id: store.daemonURL) { await loadGrants() }
    }

    private func loadGrants() async {
        error = nil
        do {
            let response = try await store.fetchApprovalGrants(for: askID)
            if !Task.isCancelled { grants = response }
        } catch {
            if !Task.isCancelled { self.error = error.localizedDescription }
        }
    }
}
