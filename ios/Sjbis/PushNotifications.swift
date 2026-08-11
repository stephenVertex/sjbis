import Foundation
import UIKit
import UserNotifications

final class PushNotifications: NSObject, UNUserNotificationCenterDelegate {
    static let shared = PushNotifications()
    private var deviceToken: String?

    func requestAuthorization() {
        UNUserNotificationCenter.current().delegate = self
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { granted, _ in
            if granted {
                DispatchQueue.main.async {
                    UIApplication.shared.registerForRemoteNotifications()
                }
            }
        }
    }

    func registerToken(_ token: String) {
        deviceToken = token
        let url = URL(string: "\(AppSettings.shared.daemonURL)/device/register")!
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")

        let deviceName = UIDevice.current.name
        let body: [String: Any] = [
            "token": token,
            "device_name": deviceName
        ]
        request.httpBody = try? JSONSerialization.data(withJSONObject: body)

        URLSession.shared.dataTask(with: request) { _, _, error in
            if let error {
                print("Failed to register device token: \(error)")
            } else {
                print("Device token registered with daemon")
            }
        }.resume()
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
