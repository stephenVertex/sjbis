import Foundation
import SwiftUI

final class AppSettings: ObservableObject {
    static let shared = AppSettings()
    private let defaults = UserDefaults.standard
    private let daemonURLKey = "daemonURL"

    @Published var daemonURL: String {
        didSet { defaults.set(daemonURL, forKey: daemonURLKey) }
    }

    private init() {
        daemonURL = defaults.string(forKey: daemonURLKey) ?? "http://dertog.tailb4b58.ts.net:7878"
    }
}
