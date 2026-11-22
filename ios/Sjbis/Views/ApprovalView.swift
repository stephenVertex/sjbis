import SwiftUI

struct ApprovalView: View {
    let ask: SjbisNotification
    let status: NotificationStatus
    @EnvironmentObject private var store: SjbisStore
    @Environment(\.scenePhase) private var scenePhase
    @State private var identity: ApprovalSignerIdentity?
    @State private var pinned = false
    @State private var checking = true
    @State private var signing = false
    @State private var showingSetup = false
    @State private var receipt: ApprovalGrant?
    @State private var error: String?
    @State private var selection: String?
    @State private var ttl: Int?
    @State private var note = ""
    @State private var once = false

    private var choices: [Choice] { ask.choices ?? [] }
    private var defaultTTL: Int { ask.ttl_secs.flatMap { $0 > 0 ? $0 : nil } ?? 3600 }
    private var ttlOptions: [Int] { Set([defaultTTL, 600, 3600, 86400]).sorted() }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let receipt {
                Label(receipt.record.decision == .approve ? "Approval signed" : "Denial signed", systemImage: "checkmark.seal.fill")
                    .font(.headline)
                if let answer = receipt.record.answer {
                    Text(choices.first { $0.value == answer }?.label ?? answer)
                }
                LabeledContent("Grant") { Text(receipt.id).monospaced().textSelection(.enabled) }
                if let expiration = try? Date.ISO8601FormatStyle().parse(receipt.record.expires_at) {
                    LabeledContent("Expires", value: expiration.formatted(date: .abbreviated, time: .shortened))
                }
            } else if signing {
                ProgressView("Signing decision…")
            } else if status != .open {
                Text(status == .timed_out ? "This approval expired." : "This approval is closed.")
                    .foregroundStyle(.secondary)
            } else if checking {
                ProgressView("Checking signing key…")
            } else if let identity, pinned {
                controls(identity)
            } else {
                Label(identity == nil ? "Set up Face ID to sign approvals." : "This signing key is awaiting enrollment.", systemImage: "faceid")
                Button("Approval Signing") { showingSetup = true }
                    .buttonStyle(.bordered)
                if identity != nil {
                    Button("Check Enrollment") { Task { await checkEnrollment() } }
                }
            }
            if let error {
                Text(error)
                    .font(.callout)
                    .foregroundStyle(.red)
            }
        }
        .task(id: store.daemonURL) { await checkEnrollment() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active, !signing { Task { await checkEnrollment() } }
        }
        .sheet(isPresented: $showingSetup, onDismiss: { Task { await checkEnrollment() } }) {
            ApprovalSetupSheet()
        }
    }

    private func controls(_ identity: ApprovalSignerIdentity) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            if !choices.isEmpty {
                Picker("Option", selection: $selection) {
                    Text("Choose an option").tag(String?.none)
                    ForEach(choices) { choice in
                        Text(choice.label).tag(Optional(choice.value))
                    }
                }
            }
            Picker("Grant lifetime", selection: Binding(get: { ttl ?? defaultTTL }, set: { ttl = $0 })) {
                ForEach(ttlOptions, id: \.self) { seconds in
                    Text(Duration.seconds(seconds).formatted(.units(width: .abbreviated))).tag(seconds)
                }
            }
            if ask.once == true {
                Label("Single-use grant", systemImage: "1.circle")
                    .foregroundStyle(.secondary)
            } else {
                Toggle("Single-use grant", isOn: $once)
            }
            TextField("Optional note", text: $note, axis: .vertical)
                .textFieldStyle(.roundedBorder)
                .lineLimit(2...5)
            HStack {
                Button(role: .destructive) { submit(.deny, identity: identity) } label: {
                    Label("Deny", systemImage: "faceid")
                }
                .buttonStyle(.bordered)
                .accessibilityHint("Confirms with Face ID.")
                Button { submit(.approve, identity: identity) } label: {
                    Label("Approve", systemImage: "faceid")
                }
                .buttonStyle(.borderedProminent)
                .accessibilityHint("Confirms with Face ID.")
                .disabled(!choices.isEmpty && selection == nil)
            }
        }
    }

    private func checkEnrollment() async {
        guard status == .open else { return }
        let server = store.daemonURL
        checking = true
        defer { if !Task.isCancelled, store.daemonURL == server { checking = false } }
        do {
            let currentIdentity = try ApprovalKey.identity()
            identity = currentIdentity
            let enrolled = if let currentIdentity { try await store.approvalSignerIsPinned(currentIdentity) } else { false }
            guard !Task.isCancelled, store.daemonURL == server else { return }
            pinned = enrolled
            error = nil
        } catch {
            guard !Task.isCancelled, store.daemonURL == server else { return }
            pinned = false
            self.error = error.localizedDescription
        }
    }

    private func submit(_ decision: ApprovalDecision, identity: ApprovalSignerIdentity) {
        let input = ApprovalSubmission(decision: decision, answer: selection, note: note, ttl: ttl ?? defaultTTL, once: once)
        signing = true
        error = nil
        Task {
            defer { signing = false }
            do {
                receipt = try await store.submitApproval(input, for: ask, identity: identity)
            } catch {
                self.error = error.localizedDescription
                await store.refresh()
            }
        }
    }
}

struct ApprovalSetupSection: View {
    @EnvironmentObject private var store: SjbisStore
    @Environment(\.scenePhase) private var scenePhase
    @State private var identity: ApprovalSignerIdentity?
    @State private var pinned: Bool?
    @State private var checking = true
    @State private var replacingKey = false
    @State private var error: String?

    var body: some View {
        Section("Approval Signing") {
            if checking {
                ProgressView("Checking signing key…")
            } else if let identity {
                LabeledContent("Face ID key", value: pinned == true ? "Enrolled" : "Setup required")
                if pinned != true {
                    Text("Pin this public key in the daemon’s signer registry and on the machines that verify your grants.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text(identity.id).font(.caption.monospaced()).textSelection(.enabled)
                    ShareLink(item: identity.enrollmentCommand) {
                        Label("Share Enrollment Command", systemImage: "square.and.arrow.up")
                    }
                    Button("Check Enrollment") { Task { await checkEnrollment() } }
                }
                Button("Replace Signing Key", role: .destructive) { replacingKey = true }
            } else {
                Button { createKey() } label: { Label("Set Up Face ID", systemImage: "faceid") }
            }
            if let error {
                Text(error).font(.callout).foregroundStyle(.red)
            }
        }
        .task(id: store.daemonURL) { await checkEnrollment() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active, pinned != true { Task { await checkEnrollment() } }
        }
        .confirmationDialog("Replace your signing key?", isPresented: $replacingKey, titleVisibility: .visible) {
            Button("Replace Signing Key", role: .destructive) { createKey() }
        } message: {
            Text("You’ll need to pin the replacement key before signing again.")
        }
    }

    private func createKey() {
        do {
            identity = try ApprovalKey.create()
            pinned = false
            error = nil
            Task { await checkEnrollment() }
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func checkEnrollment() async {
        let server = store.daemonURL
        checking = true
        defer { if !Task.isCancelled, store.daemonURL == server { checking = false } }
        do {
            let currentIdentity = try ApprovalKey.identity()
            identity = currentIdentity
            let enrolled = if let currentIdentity { try await store.approvalSignerIsPinned(currentIdentity) } else { false }
            guard !Task.isCancelled, store.daemonURL == server else { return }
            pinned = enrolled
            error = nil
        } catch {
            guard !Task.isCancelled, store.daemonURL == server else { return }
            pinned = nil
            self.error = error.localizedDescription
        }
    }
}

private struct ApprovalSetupSheet: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form { ApprovalSetupSection() }
                .navigationTitle("Approval Signing")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Done") { dismiss() }
                    }
                }
        }
    }
}
