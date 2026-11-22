import Foundation
import Combine
import UIKit
import UserNotifications

final class PushNotifications: NSObject, UNUserNotificationCenterDelegate, ObservableObject {
    static let shared = PushNotifications()
    @Published private(set) var openedID: String?
    private var deviceToken: String?

    func requestAuthorization() {
        UNUserNotificationCenter.current().delegate = self
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { granted, _ in
            if granted {
                DispatchQueue.main.async {
                    UIApplication.shared.registerForRemoteNotifications()
                }
            }
        }
    }

    func registerToken(_ token: String) {
        deviceToken = token
        syncRegistration()
    }

    func syncRegistration() {
        guard let token = deviceToken,
              let base = URL(string: AppSettings.shared.daemonURL) else { return }
        let url = base.appendingPathComponent("device").appendingPathComponent("register")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")

        let deviceName = UIDevice.current.name
        let body: [String: Any] = [
            "token": token,
            "device_name": deviceName
        ]
        request.httpBody = try? JSONSerialization.data(withJSONObject: body)

        URLSession.shared.dataTask(with: request) { _, response, error in
            if let error {
                print("Failed to register device token: \(error)")
            } else if let response = response as? HTTPURLResponse, response.statusCode != 200 {
                print("Device token registration returned HTTP \(response.statusCode)")
            } else {
                print("Device token registered with daemon")
            }
        }.resume()
    }

    func clearOpenedID(_ id: String) {
        if openedID == id { openedID = nil }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler completionHandler: @escaping () -> Void) {
        let id = response.notification.request.content.userInfo["notification_id"] as? String
        DispatchQueue.main.async {
            if response.actionIdentifier == UNNotificationDefaultActionIdentifier {
                self.openedID = id
            }
            completionHandler()
        }
    }

    func unregisterToken() {
        guard let token = deviceToken else { return }
        let url = URL(string: "\(AppSettings.shared.daemonURL)/device/unregister")!
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let body: [String: Any] = ["token": token]
        request.httpBody = try? JSONSerialization.data(withJSONObject: body)
        URLSession.shared.dataTask(with: request).resume()
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .sound])
    }
}
