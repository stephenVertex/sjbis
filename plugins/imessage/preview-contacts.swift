import Contacts
import Foundation
import ImageIO
import UniformTypeIdentifiers

struct PreviewContact: Encodable {
    let name: String
    let handles: [String]
    let photo: String?
}

struct PreviewConfig: Encodable {
    let pi_ssh: String
    let contacts: [PreviewContact]
}

func previewPhoto(_ data: Data?) -> String? {
    guard let data, let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
    let options = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: 224,
    ] as CFDictionary
    guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options) else { return nil }
    let jpeg = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(jpeg, UTType.jpeg.identifier as CFString, 1, nil) else {
        return nil
    }
    CGImageDestinationAddImage(destination, image, [kCGImageDestinationLossyCompressionQuality: 0.8] as CFDictionary)
    return CGImageDestinationFinalize(destination) ? jpeg.base64EncodedString() : nil
}

func exportContacts() throws {
    let args = Array(CommandLine.arguments.dropFirst())
    guard args.count >= 2 else {
        throw NSError(domain: "Usage: preview-contacts <pi SSH host> <full contact name> […]", code: 1)
    }

    let store = CNContactStore()
    let formatter = CNContactFormatter()
    let keys: [CNKeyDescriptor] = [
        CNContactFormatter.descriptorForRequiredKeys(for: .fullName),
        CNContactGivenNameKey as NSString,
        CNContactPhoneNumbersKey as NSString,
        CNContactEmailAddressesKey as NSString,
        CNContactThumbnailImageDataKey as NSString,
    ]
    let contacts = try args.dropFirst().map { name in
        let predicate = CNContact.predicateForContacts(matchingName: name)
        let matches = try store.unifiedContacts(matching: predicate, keysToFetch: keys).filter {
            formatter.string(from: $0)?.caseInsensitiveCompare(name) == .orderedSame
        }
        guard matches.count == 1, let contact = matches.first else {
            throw NSError(domain: "Expected one exact Contacts match for \(name), found \(matches.count)", code: 2)
        }
        let handles = contact.phoneNumbers.map { $0.value.stringValue }
            + contact.emailAddresses.map { $0.value as String }
        guard !handles.isEmpty else {
            throw NSError(domain: "No phone number or email in Contacts for \(name)", code: 3)
        }
        return PreviewContact(
            name: contact.givenName.isEmpty ? name : contact.givenName,
            handles: handles,
            photo: previewPhoto(contact.thumbnailImageData)
        )
    }
    let config = PreviewConfig(pi_ssh: args[0], contacts: contacts)
    FileHandle.standardOutput.write(try JSONEncoder().encode(config))
    FileHandle.standardOutput.write(Data("\n".utf8))
}

do {
    try exportContacts()
} catch {
    FileHandle.standardError.write(Data("\(error)\n".utf8))
    exit(1)
}
