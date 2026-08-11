import SwiftUI

struct NotificationDetailView: View {
    let notif: SjbisNotification
    @EnvironmentObject var store: SjbisStore
    @Environment(\.dismiss) var dismiss
    @State private var showNote = false
    @State private var note = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header
                if let md = notif.detail_markdown {
                    if let attr = try? AttributedString(markdown: md) {
                        Text(attr)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    } else {
                        Text(md)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                } else if let detail = notif.detail {
                    Text(detail)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                Divider()
                renderer
                Divider()
                noteSection
            }
            .padding()
        }
        .navigationTitle(notif.agent_name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Button("Snooze 5 min") { Task { await store.snooze(notif.id, minutes: 5) } }
                    Button("Snooze 15 min") { Task { await store.snooze(notif.id, minutes: 15) } }
                    Button("Snooze 30 min") { Task { await store.snooze(notif.id, minutes: 30) } }
                    Divider()
                    Button("Dismiss", role: .destructive) { Task { await store.dismiss(notif.id); dismiss() } }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
            }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(notif.question)
                .font(.title3)
                .fontWeight(.semibold)
            HStack {
                if let inst = notif.instance {
                    Text("\(notif.agent_name) · \(inst)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Text(notif.agent_name)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                UrgencyBadge(urgency: notif.urgency)
            }
        }
    }

    @ViewBuilder
    private var noteSection: some View {
        if showNote {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text("Note for agent")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button { showNote = false; note = "" } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(.secondary)
                    }
                }
                TextField("Optional context for the agent…", text: $note, axis: .vertical)
                    .textFieldStyle(.roundedBorder)
                    .lineLimit(2...5)
            }
        } else {
            Button { showNote = true } label: {
                Label("Add note", systemImage: "square.and.pencil")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func doAnswer(_ value: String) {
        let trimmed = note.trimmingCharacters(in: .whitespacesAndNewlines)
        Task { await store.answer(notif.id, answer: value, note: trimmed.isEmpty ? nil : trimmed); dismiss() }
    }

    private func doDismiss() {
        Task { await store.dismiss(notif.id); dismiss() }
    }

    @ViewBuilder
    private var renderer: some View {
        switch notif.question_type {
        case .yesno:
            YesNoRenderer(notif: notif, onAnswer: doAnswer)
        case .multichoice:
            MultiChoiceRenderer(notif: notif, onAnswer: doAnswer)
        case .freetext:
            FreeTextRenderer(notif: notif, onAnswer: doAnswer)
        case .numeric:
            NumericRenderer(notif: notif, onAnswer: doAnswer)
        case .ack:
            AckRenderer(notif: notif, onAnswer: doAnswer)
        case .diff:
            DiffRenderer(notif: notif, onAnswer: doAnswer)
        case .picklist:
            PickListRenderer(notif: notif, onAnswer: doAnswer)
        case .schedule:
            ScheduleRenderer(notif: notif, onAnswer: doAnswer)
        case .form:
            FormRenderer(notif: notif, onAnswer: doAnswer)
        case .file:
            FileRenderer(notif: notif, onAnswer: doAnswer)
        }
    }
}

struct UrgencyBadge: View {
    let urgency: Int
    var body: some View {
        HStack(spacing: 2) {
            ForEach(0..<5) { i in
                Image(systemName: i < urgency ? "bolt.fill" : "bolt")
                    .font(.caption2)
                    .foregroundStyle(urgency >= 4 ? .red : (urgency >= 2 ? .orange : .blue))
            }
        }
    }
}

struct AnswerButton: View {
    let label: String
    let action: () -> Void
    var prominent = false

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.body)
                .fontWeight(prominent ? .bold : .regular)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 14)
                .background(prominent ? Color.accentColor : Color(.tertiarySystemBackground))
                .foregroundStyle(prominent ? .white : .primary)
                .clipShape(RoundedRectangle(cornerRadius: 10))
        }
    }
}
