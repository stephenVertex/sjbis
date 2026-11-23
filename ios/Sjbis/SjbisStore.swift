import Foundation

enum ConnectionStatus: String {
    case connecting = "Connecting"
    case live = "Live"
    case reconnecting = "Reconnecting"
    case offline = "Offline"
}

@MainActor
final class SjbisStore: ObservableObject {
    @Published var notifications: [SjbisNotification] = []
    @Published var missed: [SjbisNotification] = []
    @Published var history: [SjbisNotification] = []
    @Published var agents: [String: Agent] = [:]
    @Published var rules: [Rule] = []
    @Published private(set) var connectionStatus: ConnectionStatus = .connecting
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
            guard !Task.isCancelled, generation == refreshGeneration else { return }
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
        connectionStatus = .connecting
        connectionError = nil
        sseTask = Task { await sseLoop() }
    }

    func stopSSE() {
        sseTask?.cancel()
        sseTask = nil
        connectionStatus = .offline
    }

    private func sseLoop() async {
        guard !Task.isCancelled else { return }
        guard let url = URL(string: "\(daemonURL)/events") else {
            connectionStatus = .offline
            connectionError = URLError(.badURL).localizedDescription
            return
        }
        var request = URLRequest(url: url)
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")

        while !Task.isCancelled {
            do {
                let (bytes, response) = try await URLSession.shared.bytes(for: request)
                guard !Task.isCancelled else { return }
                guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                    throw URLError(.badServerResponse)
                }
                connectionStatus = .live
                connectionError = nil
                await refresh()
                for try await line in bytes.lines {
                    guard !Task.isCancelled else { return }
                    if line.hasPrefix("data:") {
                        let json = String(line.dropFirst(5).trimmingCharacters(in: .whitespaces))
                        handleSSEEvent(json)
                    }
                }
                guard !Task.isCancelled else { return }
                connectionStatus = .reconnecting
            } catch {
                guard !Task.isCancelled else { return }
                connectionStatus = (error as? URLError)?.code == .notConnectedToInternet ? .offline : .reconnecting
                connectionError = error.localizedDescription
            }
            try? await Task.sleep(for: .seconds(3))
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
        case "notification_timed_out":
            Task { await refresh() }
        case "notification_missed_dismissed":
            if let id = event.id { removeMissed([id]) }
        case "notifications_missed_cleared":
            if let ids = event.ids { removeMissed(ids) }
        case "notification_answered", "notification_dismissed":
            if let env = event.envelope {
                if let idx = notifications.firstIndex(where: { $0.id == env.id }) {
                    var n = notifications[idx]
                    n.status = env.via == "timed_out" ? .timed_out :
                               env.via == "dismissed" ? .dismissed : .answered
                    n.answer = env.answer
                    n.answer_label = env.answer_label
                    n.answered_at = env.answered_at
                    n.note = env.note
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

    func approvalSignerIsPinned(_ identity: ApprovalSignerIdentity) async throws -> Bool {
        let base = try serverURL()
        let pinned = try await approvalSignerIsPinned(identity, at: base)
        guard base == (try? serverURL()) else { throw ApprovalError.serverChanged }
        return pinned
    }

    func submitApproval(_ submission: ApprovalSubmission, for ask: SjbisNotification,
                        identity: ApprovalSignerIdentity) async throws -> ApprovalGrant {
        let base = try serverURL()
        guard try await approvalSignerIsPinned(identity, at: base) else { throw ApprovalError.unenrolled }
        let askURL = base.appendingPathComponent("notification").appendingPathComponent(ask.id)
        let current: SjbisNotification = try await requestJSON(URLRequest(url: askURL))
        guard current.id == ask.id, current.question.utf8.elementsEqual(ask.question.utf8),
              current.agent_name == ask.agent_name, current.choices == ask.choices,
              current.detail == ask.detail, current.detail_markdown == ask.detail_markdown,
              current.renews == ask.renews, current.once == ask.once,
              current.ttl_secs == ask.ttl_secs else { throw ApprovalError.requestChanged }
        _ = try submission.record(for: current, signer: identity.id, now: Date())
        guard base == (try? serverURL()) else { throw ApprovalError.serverChanged }

        let grant = try await ApprovalKey.sign(submission, for: current, identity: identity)
        try Task.checkCancellation()
        guard base == (try? serverURL()) else { throw ApprovalError.serverChanged }
        let grantsURL = base.appendingPathComponent("grants")
        var request = URLRequest(url: grantsURL)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(grant)
        do {
            let result: ApprovalGrantResponse = try await requestJSON(request)
            guard result.envelope == grant else { throw ApprovalError.invalidResponse }
        } catch {
            // A dropped response can follow a successful commit. Look up this exact envelope.
            let lookup = URLRequest(url: grantsURL.appendingPathComponent(grant.id))
            guard let stored: ApprovalGrantResponse = try? await requestJSON(lookup),
                  stored.envelope == grant else { throw error }
        }

        if base == (try? serverURL()) {
            refreshGeneration += 1
            notifications.removeAll { $0.id == ask.id }
            if !history.contains(where: { $0.id == ask.id }) {
                var answered = current
                answered.status = .answered
                answered.answer = grant.record.decision.rawValue
                answered.answer_label = grant.record.answer
                answered.note = grant.record.note
                answered.answered_at = Date()
                history.insert(answered, at: 0)
            }
        }
        return grant
    }

    private func serverURL() throws -> URL {
        guard let base = URL(string: daemonURL), let scheme = base.scheme,
              ["http", "https"].contains(scheme), base.host != nil else { throw URLError(.badURL) }
        return base
    }

    private func approvalSignerIsPinned(_ identity: ApprovalSignerIdentity, at base: URL) async throws -> Bool {
        var request = URLRequest(url: base.appendingPathComponent("signers"))
        request.cachePolicy = .reloadIgnoringLocalCacheData
        let signers: [ApprovalSignerIdentity] = try await requestJSON(request)
        return signers.contains(identity)
    }

    private func requestJSON<T: Decodable>(_ request: URLRequest) async throws -> T {
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let response = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
        guard (200..<300).contains(response.statusCode) else {
            let body = try? decoder.decode(DaemonError.self, from: data)
            throw DaemonError(error: body?.error ?? "SJBIS rejected the request (HTTP \(response.statusCode)).")
        }
        return try decoder.decode(T.self, from: data)
    }

    func clearMissed(_ ids: [String]) async throws {
        let base = try serverURL()
        var request = URLRequest(url: base.appendingPathComponent("missed").appendingPathComponent("clear"))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(MissedIDs(ids: ids))
        let _: MissedIDs = try await requestJSON(request)
        guard base == (try? serverURL()) else { return }
        removeMissed(ids)
    }

    private func removeMissed(_ ids: [String]) {
        let cleared = Set(ids)
        refreshGeneration += 1
        missed.removeAll { cleared.contains($0.id) }
    }

    func fetchApprovalGrants(for id: String) async throws -> [ApprovalGrantResponse] {
        let url = try serverURL().appendingPathComponent("grants")
            .appending(queryItems: [URLQueryItem(name: "ask", value: id)])
        return try await requestJSON(URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData))
    }

    func fetchOlderHistory(before id: String, limit: Int) async throws -> [SjbisNotification] {
        let url = try serverURL().appendingPathComponent("history").appending(queryItems: [
            URLQueryItem(name: "before", value: id), URLQueryItem(name: "limit", value: String(limit)),
        ])
        return try await requestJSON(URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData))
    }

    func fetchActivity() async throws -> RequestActivity {
        let url = try serverURL().appendingPathComponent("activity")
        return try await requestJSON(URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData))
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

private struct DaemonError: Decodable, LocalizedError {
    var error: String
    var errorDescription: String? { error }
}

private struct MissedIDs: Codable {
    var ids: [String]
}

private struct SSEEventWrapper: Codable {
    var event: String
    var notification: SjbisNotification?
    var envelope: AnswerEnvelope?
    var id: String?
    var ids: [String]?
    var rule: Rule?
}
