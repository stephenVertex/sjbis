import CryptoKit
import XCTest
@testable import Sjbis

final class ApprovalGrantTests: XCTestCase {
    func testEnvelopeMatchesPhoneWebWireFormat() throws {
        let record = ApprovalGrantRecord(
            ask_id: "sjbis-native-compat", asked_by: "agent-x",
            question: "Review \"café\"?\n/path\t😀\u{0001}", decision: .approve,
            answer: "preview/ready", issued_at: "2026-11-22T22:00:00Z",
            expires_at: "2026-11-22T23:00:00Z", signer: "faceid@iphone-fixture",
            once: true, renews: "ta-3kp9ab"
        )
        let canonical = try record.canonicalBytes()
        let digest = SHA256.hash(data: canonical)
        let key = P256.Signing.PrivateKey()
        let grant = ApprovalGrant(record: record, digest: digest, signature: try key.signature(for: digest))

        XCTAssertEqual(grant.id, "ta-xkwp47")
        XCTAssertEqual(grant.record_sha256, "baacfe7dc6ac75394c4e2f1409d26bf9343bc684cab5449877c4e7acb4ba2e0b")
        let signature = try P256.Signing.ECDSASignature(derRepresentation: XCTUnwrap(Data(base64Encoded: grant.sig)))
        XCTAssertTrue(key.publicKey.isValidSignature(signature, for: canonical))
        XCTAssertEqual(try JSONDecoder().decode(ApprovalGrant.self, from: JSONEncoder().encode(grant)), grant)
    }

    func testDecisionPreservesApprovalConstraints() throws {
        let json = """
        {"id":"sjbis-approval","agent_name":"test-agent","sender":"test-agent","src":"test-agent",
         "question":"Which preview?","question_type":"approval","urgency":1,"blocking":false,
         "status":"open","created_at":"2026-11-22T22:00:00Z","deadline":"2026-11-22T23:00:00Z",
         "choices":[{"value":"draft","label":"Draft preview"}],"ttl_secs":600,"once":true,"renews":"ta-3kp9ab"}
        """
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let ask = try decoder.decode(SjbisNotification.self, from: Data(json.utf8))
        let now = try Date.ISO8601FormatStyle().parse("2026-11-22T22:00:04Z")
        var input = ApprovalSubmission(decision: .approve, answer: nil, note: "  test  ", ttl: 600, once: false)
        XCTAssertThrowsError(try input.record(for: ask, signer: "fixture", now: now))

        input.answer = "draft"
        let approval = try input.record(for: ask, signer: "fixture", now: now)
        XCTAssertEqual(approval.answer, "draft")
        XCTAssertEqual(approval.note, "test")
        XCTAssertEqual(approval.expires_at, "2026-11-22T22:10:04Z")
        XCTAssertEqual(approval.once, true)
        XCTAssertEqual(approval.renews, "ta-3kp9ab")

        input.decision = .deny
        let denial = try input.record(for: ask, signer: "fixture", now: now)
        XCTAssertNil(denial.answer)
        XCTAssertThrowsError(try input.record(for: ask, signer: "fixture", now: now.addingTimeInterval(3600)))
    }
}
