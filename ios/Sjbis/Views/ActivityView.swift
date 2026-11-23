import Charts
import SwiftUI

struct ActivityView: View {
    @EnvironmentObject private var store: SjbisStore
    @State private var activity: RequestActivity?
    @State private var error: String?
    @State private var selectedTime: Date?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if let activity, let first = activity.hours.first {
                    let total = activity.hours.reduce(0) { $0 + $1.count }
                    HStack(spacing: 12) {
                        metric("Last hour", (activity.hours.last?.count ?? 0).formatted())
                        metric("Last 24 hours", total.formatted())
                    }
                    VStack(alignment: .leading, spacing: 12) {
                        Text("Requests per hour").font(.headline)
                        hourlyChart(activity, start: first.start)
                        if let selectedTime, let hour = activity.hours.first(where: { $0.start <= selectedTime && selectedTime < $0.end }) {
                            LabeledContent("\(hour.start.formatted(date: .omitted, time: .shortened)) – \(hour.end.formatted(date: .omitted, time: .shortened))", value: hour.count.formatted())
                        } else {
                            LabeledContent("Average per hour", value: (Double(total) / Double(activity.hours.count)).formatted(.number.precision(.fractionLength(0...1))))
                        }
                    }
                    .font(.subheadline)
                } else if error == nil {
                    ProgressView("Loading activity…")
                }
                if let error {
                    ContentUnavailableView {
                        Label("Activity unavailable", systemImage: "chart.bar.xaxis")
                    } description: {
                        Text(error)
                    } actions: {
                        Button("Retry") { Task { await loadActivity() } }
                    }
                }
            }
            .padding()
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .task(id: store.daemonURL) {
            activity = nil
            selectedTime = nil
            await loadActivity()
        }
        .refreshable { await loadActivity() }
    }

    private func hourlyChart(_ activity: RequestActivity, start: Date) -> some View {
        Chart(activity.hours) { hour in
            RectangleMark(
                xStart: .value("Hour start", hour.start.addingTimeInterval(240)),
                xEnd: .value("Hour end", hour.end.addingTimeInterval(-240)),
                yStart: .value("Zero", 0),
                yEnd: .value("Requests", hour.count)
            )
            .foregroundStyle(Color.accentColor.opacity(selectedTime.map { hour.start <= $0 && $0 < hour.end ? 1 : 0.35 } ?? 1))
            .accessibilityLabel(hour.start.formatted(date: .omitted, time: .shortened))
            .accessibilityValue("\(hour.count) requests")
        }
        .chartXScale(domain: start...activity.as_of)
        .chartYScale(domain: 0...max(1, activity.hours.map(\.count).max() ?? 0))
        .chartXAxis {
            AxisMarks(values: .stride(by: .hour, count: 6)) {
                AxisTick()
                AxisValueLabel(format: .dateTime.hour())
            }
        }
        .chartYAxis { AxisMarks(position: .leading, values: .automatic(desiredCount: 4)) }
        .chartXSelection(value: $selectedTime)
        .frame(height: 200)
    }

    private func metric(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(value).font(.title.bold()).monospacedDigit()
            Text(title).font(.subheadline).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 14))
    }

    private func loadActivity() async {
        let server = store.daemonURL
        error = nil
        do {
            let result = try await store.fetchActivity()
            if !Task.isCancelled, store.daemonURL == server { activity = result }
        } catch {
            if !Task.isCancelled, store.daemonURL == server { self.error = error.localizedDescription }
        }
    }
}
