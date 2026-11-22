import Foundation

@MainActor
final class SjbisStore: ObservableObject {
    @Published var notifications: [SjbisNotification] = []
    @Published var missed: [SjbisNotification] = []
    @Published var history: [SjbisNotification] = []
    @Published var agents: [String: Agent] = [:]
    @Published var rules: [Rule] = []
    @Published var isConnected = false
    @Published var connectionError: String?
    @Published var daemonVersion: String?

    private let settings = AppSettings.shared
    private var sseTask: Task<Void, Never>?
    private let decoder: JSONDecoder
    private let encoder: JSONEncoder
    private var refreshGeneration = 0

    init() {
        decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let container = try decoder.singleValueContainer()
            let timestamp = try container.decode(String.self)
            if let date = try? Date.ISO8601FormatStyle(includingFractionalSeconds: true).parse(timestamp) {
                return date
            }
            return try Date.ISO8601FormatStyle().parse(timestamp)
        }
        encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
    }

    var daemonURL: String {
        settings.daemonURL
    }

    func refresh() async {
        guard let url = URL(string: "\(daemonURL)/state") else { return }
        refreshGeneration += 1
        let generation = refreshGeneration
        do {
            let (data, _) = try await URLSession.shared.data(from: url)
            guard !Task.isCancelled, generation == refreshGeneration else { return }
            let state = try decoder.decode(DashboardState.self, from: data)
            notifications = state.notifications.sorted { $0.urgency > $1.urgency }
            missed = state.missed ?? []
            history = state.history
            agents = state.agents
            rules = state.rules
            daemonVersion = state.version
            connectionError = nil
        } catch {
            connectionError = error.localizedDescription
        }
    }

    func fetchNotification(_ id: String) async -> SjbisNotification? {
        guard let base = URL(string: daemonURL) else { return nil }
        let url = base.appendingPathComponent("notification").appendingPathComponent(id)
        do {
            let (data, response) = try await URLSession.shared.data(from: url)
            guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw URLError(.badServerResponse) }
            return try decoder.decode(SjbisNotification.self, from: data)
        } catch {
            connectionError = error.localizedDescription
            return nil
        }
    }

    func startSSE() {
        sseTask?.cancel()
        sseTask = Task { await sseLoop() }
    }

    func stopSSE() {
        sseTask?.cancel()
        sseTask = nil
        isConnected = false
    }

    private func sseLoop() async {
        while !Task.isCancelled {
            do {
                let url = URL(string: "\(daemonURL)/events")!
                var request = URLRequest(url: url)
                request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                let (bytes, response) = try await URLSession.shared.bytes(for: request)
                guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                    throw URLError(.badServerResponse)
                }
                await MainActor.run { isConnected = true; connectionError = nil }
                await refresh()
                for try await line in bytes.lines {
                    if Task.isCancelled { break }
                    if line.hasPrefix("data:") {
                        let json = String(line.dropFirst(5).trimmingCharacters(in: .whitespaces))
                        handleSSEEvent(json)
                    }
                }
            } catch {
                await MainActor.run {
                    isConnected = false
                    connectionError = error.localizedDescription
                }
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }

    private func handleSSEEvent(_ json: String) {
        guard let data = json.data(using: .utf8),
              let event = try? decoder.decode(SSEEventWrapper.self, from: data) else { return }
        switch event.event {
        case "notification_created":
            if let n = event.notification, !notifications.contains(where: { $0.id == n.id }) {
                notifications.insert(n, at: 0)
                notifications.sort { $0.urgency > $1.urgency }
            }
        case "notification_updated":
            if let n = event.notification {
                if let idx = notifications.firstIndex(where: { $0.id == n.id }) {
                    notifications[idx] = n
                }
            }
        case "notification_timed_out", "notification_missed_dismissed":
            Task { await refresh() }
        case "notification_answered", "notification_dismissed":
            if let env = event.envelope {
                if let idx = notifications.firstIndex(where: { $0.id == env.id }) {
                    var n = notifications[idx]
                    n.status = env.via == "timed_out" ? .timed_out :
                               env.via == "dismissed" ? .dismissed : .answered
                    n.answer = env.answer
                    n.answer_label = env.answer_label
                    n.answered_at = env.answered_at
                    history.insert(n, at: 0)
                    notifications.remove(at: idx)
                }
            }
        case "notification_cancelled":
            if let id = event.id {
                notifications.removeAll { $0.id == id }
            }
        case "rule_created":
            if let r = event.rule { rules.append(r) }
        case "rule_updated":
            if let r = event.rule {
                if let idx = rules.firstIndex(where: { $0.id == r.id }) { rules[idx] = r }
            }
        case "rule_deleted":
            if let id = event.id { rules.removeAll { $0.id == id } }
        default:
            break
        }
    }

    func answer(_ id: String, answer: String, via: String = "ios", note: String? = nil) async {
        guard let url = URL(string: "\(daemonURL)/answer/\(id)") else { return }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let body = AnswerRequest(answer: answer, via: via, note: note)
        request.httpBody = try? encoder.encode(body)
        _ = try? await URLSession.shared.data(for: request)
        if let idx = notifications.firstIndex(where: { $0.id == id }) {
            var n = notifications[idx]
            n.status = .answered
            n.answer = answer
            n.answered_at = Date()
            history.insert(n, at: 0)
            notifications.remove(at: idx)
        }
    }

    func dismiss(_ id: String) async {
        guard let url = URL(string: "\(daemonURL)/dismiss/\(id)") else { return }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw URLError(.badServerResponse) }
            refreshGeneration += 1
            missed.removeAll { $0.id == id }
            notifications.removeAll { $0.id == id }
        } catch {
            connectionError = error.localizedDescription
        }
    }

    func snooze(_ id: String, minutes: Int) async {
        guard let url = URL(string: "\(daemonURL)/snooze/\(id)") else { return }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? encoder.encode(SnoozeRequest(minutes: minutes))
        _ = try? await URLSession.shared.data(for: request)
        await refresh()
    }
}

private struct SSEEventWrapper: Codable {
    var event: String
    var notification: SjbisNotification?
    var envelope: AnswerEnvelope?
    var id: String?
    var rule: Rule?
}
