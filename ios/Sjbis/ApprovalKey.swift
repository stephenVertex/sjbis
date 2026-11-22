import CryptoKit
import Foundation
import LocalAuthentication
import Security

struct ApprovalSignerIdentity: Codable, Equatable {
    var id: String
    var pubkey: String

    var enrollmentCommand: String {
        "ta signers add --id \(id) --method faceid --device 'SJBIS native iPhone' --pubkey \(pubkey)"
    }
}

enum ApprovalKey {
    private struct StoredKey: Codable {
        var representation: Data
        var identity: ApprovalSignerIdentity
    }

    private static var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: "com.meshcrawler.sjbis.approvals",
         kSecAttrAccount as String: "face-id-v1"]
    }

    static func identity() throws -> ApprovalSignerIdentity? {
        try load()?.identity
    }

    static func create() throws -> ApprovalSignerIdentity {
        let context = try faceIDContext()
        defer { context.invalidate() }
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly,
            [.privateKeyUsage, .biometryCurrentSet], &error
        ) else {
            if let error { throw error.takeRetainedValue() }
            throw ApprovalError.faceIDRequired
        }
        let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access, authenticationContext: context)
        let publicKey = key.publicKey.x963Representation
        let fingerprint = SHA256.hash(data: publicKey).prefix(8).map { String(format: "%02x", Int($0)) }.joined()
        let identity = ApprovalSignerIdentity(id: "faceid@iphone-\(fingerprint)", pubkey: publicKey.base64EncodedString())
        let data = try JSONEncoder().encode(StoredKey(representation: key.dataRepresentation, identity: identity))

        let updated = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if updated == errSecItemNotFound {
            var item = query
            item[kSecAttrAccessible as String] = kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly
            item[kSecValueData as String] = data
            let added = SecItemAdd(item as CFDictionary, nil)
            guard added == errSecSuccess else { throw ApprovalError.keychain(added) }
        } else if updated != errSecSuccess {
            throw ApprovalError.keychain(updated)
        }
        #if DEBUG
        // A development device can pass its public enrollment data to its connected Mac.
        UserDefaults.standard.set(try JSONEncoder().encode(identity), forKey: "approvalSignerPublicIdentity")
        #endif
        return identity
    }

    static func sign(_ submission: ApprovalSubmission, for ask: SjbisNotification,
                     identity: ApprovalSignerIdentity) async throws -> ApprovalGrant {
        guard let stored = try load(), stored.identity == identity else { throw ApprovalError.keyMissing }
        let context = try faceIDContext()
        defer { context.invalidate() }
        context.localizedReason = "\(submission.decision.label) the request from \(ask.agent_name)."
        let authenticated = try await context.evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, localizedReason: context.localizedReason)
        guard authenticated else { throw LAError(.authenticationFailed) }
        try Task.checkCancellation()

        let key = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: stored.representation, authenticationContext: context)
        let record = try submission.record(for: ask, signer: identity.id, now: Date())
        let digest = SHA256.hash(data: try record.canonicalBytes())
        let signature = try key.signature(for: digest)
        return ApprovalGrant(record: record, digest: digest, signature: signature)
    }

    private static func faceIDContext() throws -> LAContext {
        let context = LAContext()
        context.localizedFallbackTitle = ""
        var error: NSError?
        let available = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
        guard SecureEnclave.isAvailable, context.biometryType == .faceID else { throw ApprovalError.faceIDRequired }
        guard available else { throw error ?? ApprovalError.faceIDRequired as NSError }
        return context
    }

    private static func load() throws -> StoredKey? {
        var request = query
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = item as? Data else { throw ApprovalError.keychain(status) }
        return try JSONDecoder().decode(StoredKey.self, from: data)
    }
}
