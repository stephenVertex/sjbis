import CryptoKit
import Foundation
import Security

enum ApprovalDecision: String, Codable {
    case approve
    case deny

    var label: String { rawValue.capitalized }
}

struct ApprovalGrantRecord: Codable, Equatable {
    var v = 1
    var kind = "grant"
    var ask_id: String
    var asked_by: String
    var question: String
    var decision: ApprovalDecision
    var answer: String?
    var note: String?
    var issued_at: String
    var expires_at: String
    var signer: String
    var once: Bool?
    var renews: String?

    func canonicalBytes() throws -> Data {
        // The grant schema uses fixed ASCII keys and only strings, booleans and integers.
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return try encoder.encode(self)
    }
}

struct ApprovalGrant: Codable, Equatable {
    var id: String
    var record: ApprovalGrantRecord
    var record_sha256: String
    var alg: String
    var sig: String

    init(record: ApprovalGrantRecord, digest: SHA256.Digest, signature: P256.Signing.ECDSASignature) {
        let prefix = digest.prefix(4).reduce(UInt32.zero) { ($0 << 8) | UInt32($1) }
        let alphabet = Array("abcdefghijklmnopqrstuvwxyz234567")
        let suffix = [27, 22, 17, 12, 7, 2].map { alphabet[Int((prefix >> $0) & 31)] }
        id = "ta-" + String(suffix)
        self.record = record
        record_sha256 = digest.map { String(format: "%02x", Int($0)) }.joined()
        alg = "es256"
        sig = signature.derRepresentation.base64EncodedString()
    }
}

struct ApprovalSubmission {
    var decision: ApprovalDecision
    var answer: String?
    var note: String
    var ttl: Int
    var once: Bool

    func record(for ask: SjbisNotification, signer: String, now: Date) throws -> ApprovalGrantRecord {
        guard ask.question_type == .approval, ask.status == .open,
              ask.deadline.map({ $0 > now }) ?? true else { throw ApprovalError.closed }
        let choice = decision == .approve ? answer : nil
        if decision == .approve, let choices = ask.choices, !choices.isEmpty {
            guard let choice, choices.contains(where: { $0.value.utf8.elementsEqual(choice.utf8) }) else {
                throw ApprovalError.chooseOption
            }
        }
        guard ttl > 0 else { throw ApprovalError.invalidLifetime }

        let formatter = ISO8601DateFormatter()
        let trimmedNote = note.trimmingCharacters(in: .whitespacesAndNewlines)
        return ApprovalGrantRecord(
            ask_id: ask.id, asked_by: ask.agent_name, question: ask.question,
            decision: decision, answer: choice, note: trimmedNote.isEmpty ? nil : trimmedNote,
            issued_at: formatter.string(from: now),
            expires_at: formatter.string(from: now.addingTimeInterval(TimeInterval(ttl))),
            signer: signer, once: once || ask.once == true ? true : nil, renews: ask.renews
        )
    }
}

enum ApprovalError: LocalizedError {
    case faceIDRequired
    case keyMissing
    case unenrolled
    case closed
    case chooseOption
    case invalidLifetime
    case serverChanged
    case requestChanged
    case invalidResponse
    case keychain(OSStatus)
    case server(String)

    var errorDescription: String? {
        switch self {
        case .faceIDRequired: "Set up Face ID on this iPhone to sign approvals."
        case .keyMissing: "Set up your Face ID signing key first."
        case .unenrolled: "Pin this iPhone’s signing key on the server before signing."
        case .closed: "This approval is no longer open."
        case .chooseOption: "Choose an option to approve."
        case .invalidLifetime: "Choose a positive grant lifetime."
        case .serverChanged: "The server changed during signing. Reopen the approval and try again."
        case .requestChanged: "The approval changed. Review the latest request before signing."
        case .invalidResponse: "SJBIS couldn’t confirm this grant. Check the queue before trying again."
        case .keychain(let status): "Couldn’t access the signing key (Keychain error \(status))."
        case .server(let message): message
        }
    }
}
