import Foundation
import XCTest
@testable import Sjbis

final class ChoiceDecodingTests: XCTestCase {
    private let reportedStrings = [
        "Legacy first choice",
        "Legacy second choice",
    ]

    private let triageChoices = [
        Choice(value: "schedule", label: "Accept suggestion", hint: nil),
        Choice(value: "delete", label: "Keep → plan it", hint: nil),
        Choice(value: "needs_replan", label: "Wontfix / obsolete", hint: nil),
        Choice(value: "merge_into", label: "Duplicate", hint: "Select a target item"),
        Choice(value: "leave_captured", label: "Needs discussion", hint: nil),
    ]

    func testNotificationAndSubQuestionDecodeBothChoiceForms() throws {
        let notification = try decodeNotification()
        let expected = reportedStrings.map {
            Choice(value: $0, label: $0, hint: nil)
        } + triageChoices

        let topLevelChoices = try XCTUnwrap(notification.choices)
        XCTAssertEqual(topLevelChoices, expected)
        XCTAssertEqual(topLevelChoices.map(\.id), expected.map(\.value))

        let subQuestion = try XCTUnwrap(notification.sub_questions?.first)
        XCTAssertEqual(try XCTUnwrap(subQuestion.choices), expected)
    }

    func testChoiceEncodingAlwaysUsesCanonicalObjects() throws {
        let notification = try decodeNotification()
        let choices = try XCTUnwrap(notification.choices)
        let data = try JSONEncoder().encode(choices)
        let encoded = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        )

        XCTAssertEqual(encoded.count, choices.count)
        for (object, choice) in zip(encoded, choices) {
            XCTAssertEqual(object["value"] as? String, choice.value)
            XCTAssertEqual(object["label"] as? String, choice.label)
            XCTAssertEqual(object["hint"] as? String, choice.hint)
        }
    }

    func testCanonicalChoiceRequiresValueAndLabel() throws {
        for invalidChoice in [
            ["label": "Missing value"],
            ["value": "missing-label"],
        ] {
            let data = try JSONSerialization.data(withJSONObject: invalidChoice)
            XCTAssertThrowsError(try JSONDecoder().decode(Choice.self, from: data))
        }
    }

    private func decodeNotification() throws -> SjbisNotification {
        var choices = reportedStrings.map { $0 as Any }
        choices.append(contentsOf: triageChoices.map { choice in
            var object: [String: Any] = [
                "value": choice.value,
                "label": choice.label,
            ]
            if let hint = choice.hint {
                object["hint"] = hint
            }
            return object
        })
        let payload: [String: Any] = [
            "id": "sjbis-bZ66aqXF",
            "agent_name": "yesod-triage",
            "sender": "yesod-triage",
            "src": "yesod-triage · yesod backlog triage 2026-09-22",
            "question": "Choose a triage verdict",
            "question_type": "multichoice",
            "urgency": 1,
            "blocking": false,
            "status": "open",
            "created_at": "2026-09-22T18:20:17Z",
            "choices": choices,
            "sub_questions": [
                [
                    "key": "verdict",
                    "question": "Which verdict should be applied?",
                    "shape": "multichoice",
                    "choices": choices,
                ],
            ],
        ]

        let data = try JSONSerialization.data(withJSONObject: payload)
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(SjbisNotification.self, from: data)
    }
}
