import SwiftUI

struct YesNoRenderer: View {
    let notif: SjbisNotification
    let onAnswer: (String) -> Void

    var body: some View {
        HStack(spacing: 12) {
            AnswerButton(label: notif.no_label ?? "No") { onAnswer("No") }
            AnswerButton(label: notif.yes_label ?? "Yes", action: { onAnswer("Yes") }, prominent: true)
        }
    }
}

struct MultiChoiceRenderer: View {
    let notif: SjbisNotification
    let onAnswer: (String) -> Void

    var body: some View {
        VStack(spacing: 8) {
            if let choices = notif.choices {
                ForEach(Array(choices.enumerated()), id: \.element.id) { idx, choice in
                    Button { onAnswer(choice.value) } label: {
                        HStack {
                            Text("\(idx + 1)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .frame(width: 24)
                            VStack(alignment: .leading) {
                                Text(choice.label)
                                    .font(.body)
                                if let hint = choice.hint {
                                    Text(hint)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            Spacer()
                        }
                        .padding(12)
                        .background(Color(.tertiarySystemBackground))
                        .clipShape(RoundedRectangle(cornerRadius: 10))
                    }
                }
            }
        }
    }
}

struct FreeTextRenderer: View {
    let notif: SjbisNotification
    let onAnswer: (String) -> Void
    @State private var text = ""

    var body: some View {
        VStack(spacing: 12) {
            TextField(notif.placeholder ?? "Type your answer…", text: $text, axis: .vertical)
                .textFieldStyle(.roundedBorder)
                .lineLimit(3...10)

            if let suggestions = notif.suggestions, !suggestions.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack {
                        ForEach(suggestions, id: \.self) { s in
                            Button(s) { text = s }
                                .font(.caption)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 6)
                                .background(.ultraThinMaterial)
                                .clipShape(Capsule())
                        }
                    }
                }
            }

            AnswerButton(label: "Submit") { onAnswer(text) }
                .disabled(text.isEmpty)
        }
    }
}

struct NumericRenderer: View {
    let notif: SjbisNotification
    let onAnswer: (String) -> Void
    @State private var value: Double = 0

    var body: some View {
        VStack(spacing: 16) {
            let minVal = notif.min ?? 0
            let maxVal = notif.max ?? 100
            let stepVal = notif.step ?? 1

            HStack {
                Text(formatNum(value))
                    .font(.system(size: 36, weight: .bold, design: .rounded))
                if let unit = notif.unit {
                    Text(unit)
                        .font(.title3)
                        .foregroundStyle(.secondary)
                }
            }

            Slider(value: Binding(
                get: { value },
                set: { value = ($0 / stepVal).rounded() * stepVal }
            ), in: minVal...maxVal, step: stepVal)
            .tint(.accentColor)

            HStack {
                Text(formatNum(minVal))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Text(formatNum(maxVal))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            AnswerButton(label: "Submit") { onAnswer(formatNum(value)) }
        }
        .onAppear {
            value = notif.default_value ?? (notif.min ?? 0)
        }
    }

    private func formatNum(_ v: Double) -> String {
        let s = notif.step ?? 1
        return s == 1 ? String(Int(v)) : String(format: "%.1f", v)
    }
}

struct AckRenderer: View {
    let notif: SjbisNotification
    let onAnswer: (String) -> Void

    var body: some View {
        AnswerButton(label: notif.ack_label ?? "Acknowledge", action: { onAnswer("ack") }, prominent: true)
    }
}

struct DiffRenderer: View {
    let notif: SjbisNotification
    let onAnswer: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let lines = notif.diff {
                ScrollView {
                    VStack(alignment: .leading, spacing: 1) {
                        ForEach(lines) { line in
                            HStack(alignment: .top, spacing: 0) {
                                Text(prefixFor(line.kind))
                                    .font(.system(.caption, design: .monospaced))
                                    .foregroundStyle(colorFor(line.kind))
                                    .frame(width: 16)
                                Text(line.text)
                                    .font(.system(.caption, design: .monospaced))
                                    .foregroundStyle(colorFor(line.kind))
                            }
                        }
                    }
                    .padding(10)
                    .background(Color(.tertiarySystemBackground))
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                }
            }
            HStack(spacing: 12) {
                AnswerButton(label: "Reject") { onAnswer("Reject") }
                AnswerButton(label: "Approve", action: { onAnswer("Approve") }, prominent: true)
            }
        }
    }

    private func prefixFor(_ kind: String) -> String {
        switch kind { case "add": "+"; case "del": "-"; case "meta": "@"; default: " " }
    }
    private func colorFor(_ kind: String) -> Color {
        switch kind { case "add": .green; case "del": .red; case "meta": .secondary; default: .primary }
    }
}

struct PickListRenderer: View {
    let notif: SjbisNotification
    let onAnswer: (String) -> Void

    var body: some View {
        VStack(spacing: 8) {
            if let items = notif.items {
                ForEach(Array(items.enumerated()), id: \.element.id) { idx, item in
                    Button { onAnswer(item.id) } label: {
                        HStack {
                            Text("\(idx + 1)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .frame(width: 24)
                            VStack(alignment: .leading) {
                                Text(item.title)
                                    .font(.body)
                                Text(item.meta)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                        }
                        .padding(12)
                        .background(Color(.tertiarySystemBackground))
                        .clipShape(RoundedRectangle(cornerRadius: 10))
                    }
                }
            }
        }
    }
}

struct ScheduleRenderer: View {
    let notif: SjbisNotification
    let onAnswer: (String) -> Void

    var body: some View {
        VStack(spacing: 8) {
            if let slots = notif.slots {
                ForEach(Array(slots.enumerated()), id: \.element.id) { idx, slot in
                    Button {
                        guard !slot.disabled else { return }
                        onAnswer("\(slot.day) \(slot.time)")
                    } label: {
                        HStack {
                            Text("\(idx + 1)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .frame(width: 24)
                            VStack(alignment: .leading) {
                                Text("\(slot.day) at \(slot.time)")
                                    .font(.body)
                                if let reason = slot.reason {
                                    Text(reason)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            Spacer()
                            if slot.disabled {
                                Image(systemName: "xmark.circle")
                                    .foregroundStyle(.red)
                            }
                        }
                        .padding(12)
                        .background(slot.disabled ? Color(.tertiarySystemBackground).opacity(0.5) : Color(.tertiarySystemBackground))
                        .clipShape(RoundedRectangle(cornerRadius: 10))
                    }
                    .disabled(slot.disabled)
                }
            }
        }
    }
}

struct FormRenderer: View {
    let notif: SjbisNotification
    let onAnswer: (String) -> Void
    @State private var answers: [String: String] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let subs = notif.sub_questions {
                ForEach(subs) { sub in
                    VStack(alignment: .leading, spacing: 6) {
                        Text(sub.question)
                            .font(.subheadline)
                            .fontWeight(.medium)
                        if let detail = sub.detail {
                            Text(detail)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        subRenderer(sub)
                    }
                }
            }
            AnswerButton(label: "Submit") {
                let encoded = answers.map { "\($0.key)=\($0.value)" }.joined(separator: ";")
                onAnswer(encoded)
            }
        }
    }

    @ViewBuilder
    private func subRenderer(_ sub: SubQuestion) -> some View {
        switch sub.shape {
        case "yesno":
            HStack(spacing: 12) {
                AnswerButton(label: "No") { answers[sub.key] = "No" }
                AnswerButton(label: "Yes") { answers[sub.key] = "Yes" }
            }
        case "multichoice", "picklist":
            if let choices = sub.choices {
                ForEach(choices) { c in
                    Button {
                        answers[sub.key] = c.value
                    } label: {
                        Text(c.label)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(10)
                            .background(answers[sub.key] == c.value ? Color.accentColor.opacity(0.3) : Color(.tertiarySystemBackground))
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                    }
                }
            }
        case "freetext":
            TextField("Type…", text: Binding(
                get: { answers[sub.key] ?? "" },
                set: { answers[sub.key] = $0 }
            ))
            .textFieldStyle(.roundedBorder)
        case "numeric":
            HStack {
                Stepper(value: Binding(
                    get: { Double(answers[sub.key] ?? "0") ?? 0 },
                    set: { answers[sub.key] = String(Int($0)) }
                ), in: (sub.min ?? 0)...(sub.max ?? 100)) {
                    Text(answers[sub.key] ?? "\(Int(sub.default_value ?? 0))")
                        .font(.body)
                }
            }
        case "ack":
            AnswerButton(label: sub.ack_label ?? "Ack") { answers[sub.key] = "ack" }
        default:
            TextField("Type…", text: Binding(
                get: { answers[sub.key] ?? "" },
                set: { answers[sub.key] = $0 }
            ))
            .textFieldStyle(.roundedBorder)
        }
    }
}

struct FileRenderer: View {
    let notif: SjbisNotification
    let onAnswer: (String) -> Void

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "doc")
                .font(.system(size: 48))
                .foregroundStyle(.secondary)
            Text("File upload not supported on iOS yet")
                .font(.caption)
                .foregroundStyle(.secondary)
            AnswerButton(label: "Acknowledge") { onAnswer("ack") }
        }
    }
}
