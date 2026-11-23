import SwiftUI

struct MazelTovBanner: View {
    private let gold = Color(red: 0.90, green: 0.75, blue: 0.48)
    private let cream = Color(red: 1, green: 0.94, blue: 0.78)

    var body: some View {
        VStack(spacing: 12) {
            HStack(spacing: 22) {
                StarOfDavid().stroke(gold, style: StrokeStyle(lineWidth: 1.4, lineJoin: .round))
                    .frame(width: 25, height: 25)
                GabledRoofline().stroke(gold, style: StrokeStyle(lineWidth: 1.25, lineJoin: .round))
                    .frame(width: 108, height: 48)
                StarOfDavid().stroke(gold, style: StrokeStyle(lineWidth: 1.4, lineJoin: .round))
                    .frame(width: 25, height: 25)
            }
            .accessibilityHidden(true)

            Text("Mazel tov")
                .font(.system(.largeTitle, design: .serif, weight: .bold))
                .foregroundStyle(cream)
                .multilineTextAlignment(.center)
                .accessibilityAddTraits(.isHeader)
        }
        .padding(.vertical, 23)
        .padding(.horizontal, 14)
        .frame(maxWidth: .infinity)
        .background {
            RoundedRectangle(cornerRadius: 20)
                .fill(RadialGradient(
                    colors: [Color(red: 0.15, green: 0.29, blue: 0.47), Color(red: 0.05, green: 0.10, blue: 0.21)],
                    center: .top, startRadius: 2, endRadius: 225
                ))
        }
        .overlay {
            ZStack {
                RoundedRectangle(cornerRadius: 20).strokeBorder(gold.opacity(0.8), lineWidth: 1)
                RoundedRectangle(cornerRadius: 15).strokeBorder(gold.opacity(0.35), lineWidth: 1).padding(6)
            }
        }
    }
}

private struct StarOfDavid: Shape {
    func path(in rect: CGRect) -> Path {
        let radius = min(rect.width, rect.height) / 2 - 1
        return Path { path in
            for rotation in [-Double.pi / 2, Double.pi / 2] {
                let vertices = (0..<3).map { vertex in
                    let angle = rotation + Double(vertex) * 2 * .pi / 3
                    return CGPoint(x: rect.midX + radius * cos(angle), y: rect.midY + radius * sin(angle))
                }
                path.addLines(vertices)
                path.closeSubpath()
            }
        }
    }
}

private struct GabledRoofline: Shape {
    func path(in rect: CGRect) -> Path {
        let gap = rect.width * 0.04
        let width = (rect.width - 2 * gap) / 3
        let outline: [(CGFloat, CGFloat)] = [
            (0, 1), (0, 0.48), (0.13, 0.48), (0.13, 0.34), (0.25, 0.34),
            (0.25, 0.21), (0.37, 0.21), (0.37, 0.10), (0.50, 0), (0.63, 0.10),
            (0.63, 0.21), (0.75, 0.21), (0.75, 0.34), (0.87, 0.34), (0.87, 0.48), (1, 0.48), (1, 1),
        ]
        return Path { path in
            for index in 0..<3 {
                let left = rect.minX + CGFloat(index) * (width + gap)
                let height = rect.height * (index == 1 ? 1 : 0.79)
                let top = rect.maxY - height
                path.addLines(outline.map { x, y in CGPoint(x: left + x * width, y: top + y * height) })
                path.closeSubpath()

                path.move(to: CGPoint(x: left + 0.36 * width, y: rect.maxY - 0.12 * height))
                path.addLine(to: CGPoint(x: left + 0.36 * width, y: rect.maxY - 0.37 * height))
                path.addQuadCurve(to: CGPoint(x: left + 0.64 * width, y: rect.maxY - 0.37 * height),
                                  control: CGPoint(x: left + 0.50 * width, y: rect.maxY - 0.65 * height))
                path.addLine(to: CGPoint(x: left + 0.64 * width, y: rect.maxY - 0.12 * height))
                path.closeSubpath()
            }
        }
    }
}
