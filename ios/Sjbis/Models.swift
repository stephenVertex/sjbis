import Foundation

enum QuestionType: String, Codable, CaseIterable {
    case yesno
    case multichoice
    case freetext
    case numeric
    case file
    case diff
    case ack
    case picklist
    case schedule
    case form
}

enum NotificationStatus: String, Codable {
    case open
    case answered
    case cancelled
    case muted
    case timed_out
    case dismissed
}

struct Choice: Codable, Identifiable, Hashable {
    var value: String
    var label: String
    var hint: String?

    var id: String { value }
}

struct DiffLine: Codable, Identifiable, Hashable {
    var kind: String
    var text: String
    var id: String { "\(kind)-\(text)" }

    var color: String {
        switch kind {
        case "add": return "green"
        case "del": return "red"
        case "meta": return "secondary"
        default: return "primary"
        }
    }
}

struct PickItem: Codable, Identifiable, Hashable {
    var id: String
    var title: String
    var meta: String
}

struct Slot: Codable, Identifiable, Hashable {
    var day: String
    var time: String
    var disabled: Bool
    var reason: String?
    var id: String { "\(day)-\(time)" }
}

struct SubQuestion: Codable, Identifiable, Hashable {
    var key: String
    var question: String
    var shape: String
    var detail: String?
    var choices: [Choice]?
    var min: Double?
    var max: Double?
    var step: Double?
    var unit: String?
    var default_value: Double?
    var ack_label: String?

    var id: String { key }
}

struct SjbisNotification: Codable, Identifiable, Hashable {
    var id: String
    var agent_name: String
    var instance: String?
    var sender: String
    var src: String
    var question: String
    var detail: String?
    var detail_markdown: String?
    var question_type: QuestionType
    var urgency: Int
    var blocking: Bool
    var deadline: Date?
    var status: NotificationStatus
    var created_at: Date
    var answered_at: Date?
    var answer: String?
    var answer_label: String?
    var choices: [Choice]?
    var yes_label: String?
    var no_label: String?
    var placeholder: String?
    var suggestions: [String]?
    var min: Double?
    var max: Double?
    var step: Double?
    var default_value: Double?
    var unit: String?
    var accept: String?
    var diff: [DiffLine]?
    var ack_label: String?
    var items: [PickItem]?
    var slots: [Slot]?
    var mute_key: String?
    var snooze_until: Date?
    var note: String?
    var sub_questions: [SubQuestion]?
}

struct Agent: Codable, Identifiable, Hashable {
    var name: String
    var glyph: String
    var color: String
    var kind: String
    var id: String { name }
}

struct Rule: Codable, Identifiable, Hashable {
    var id: String
    var text: String
    var active: Bool
    var priority: Int
    var mute: Bool
    var created_at: Date
    var expires_at: Date?
}

struct DashboardState: Codable {
    var notifications: [SjbisNotification]
    var history: [SjbisNotification]
    var rules: [Rule]
    var agents: [String: Agent]
    var version: String?
}

struct AnswerRequest: Codable {
    var answer: String
    var via: String?
    var note: String?
}

struct AnswerEnvelope: Codable {
    var id: String
    var answer: String?
    var answer_label: String?
    var answered_at: Date?
    var latency_ms: Int?
    var renderer: String
    var src: String
    var via: String
    var note: String?
}

struct SnoozeRequest: Codable {
    var minutes: Int
}
