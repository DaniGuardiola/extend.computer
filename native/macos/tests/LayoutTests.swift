import Foundation
import CoreGraphics

extension EdgeSide: Decodable {}

// native/shared/fixtures/edge-layout.json; native/windows/input/src/edge.rs
// replays the same file through its port.
struct EdgeFixture: Decodable {
    struct Layout: Decodable {
        let local: [Double]
        let remote: [Double]
        let side: EdgeSide
        let offset_y: Double
        func make() throws -> EdgeLayout {
            try EdgeLayout(localWidth: local[0], localHeight: local[1], remoteWidth: remote[0],
                           remoteHeight: remote[1], side: side, offsetY: offset_y)
        }
    }
    struct Step: Decodable {
        let motion: [Double]?
        let exposed: Bool?
        let allow_return: Bool?
        let set_side: EdgeSide?
        let cancel: Bool?
        let expect: String?
        let at: [Double]?
        let side: EdgeSide?
    }
    struct Case: Decodable {
        let name: String
        let layout: Layout
        let steps: [Step]
    }
    let invalid_layouts: [Layout]
    let cases: [Case]
}

func matches(_ outcome: EdgeMotion, _ step: EdgeFixture.Step) -> Bool {
    let (kind, point): (String, EdgePoint?) = switch outcome {
    case .local: ("local", nil)
    case .remote(let point): ("remote", point)
    case .returned(let point): ("returned", point)
    }
    guard kind == step.expect else { return false }
    guard let point, let at = step.at else { return point == nil && step.at == nil }
    return abs(point.x - at[0]) < 1e-9 && abs(point.y - at[1]) < 1e-9
}

@main struct LayoutTests {
    static func main() throws {
        // Main external display above/right of the built-in laptop display.
        // The laptop's left edge must hand off even though it is not main.
        let external = EdgeDisplay(id: 5, bounds: CGRect(x: 0, y: 0, width: 2560, height: 1440))
        let laptop = EdgeDisplay(id: 1, bounds: CGRect(x: -1728, y: 773, width: 1728, height: 1117))
        let displays = [external, laptop]
        precondition(edgeDisplay(at: CGPoint(x: -1728, y: 900), in: displays) == laptop)
        precondition(edgeIsExposed(display: laptop, side: .left, y: 127, in: displays))
        precondition(!edgeIsExposed(display: external, side: .left, y: 900, in: displays))
        precondition(edgeIsExposed(display: external, side: .left, y: 200, in: displays))
        precondition(!edgeIsExposed(display: laptop, side: .right, y: 127, in: displays))
        precondition(edgeIsExposed(display: laptop, side: .right, y: 1000, in: displays))
        print("PASS secondary display selection, external/laptop seams, exposed outer edges")

        let decoder = JSONDecoder()
        decoder.nonConformingFloatDecodingStrategy = .convertFromString(positiveInfinity: "inf", negativeInfinity: "-inf", nan: "nan")
        let fixture = try decoder.decode(EdgeFixture.self, from: Data(contentsOf: URL(fileURLWithPath: "native/shared/fixtures/edge-layout.json")))
        for layout in fixture.invalid_layouts {
            do { _ = try layout.make(); fatalError("Invalid layout accepted: \(layout)") } catch {}
        }
        for test in fixture.cases {
            var layout = try test.layout.make()
            for (index, step) in test.steps.enumerated() {
                let outcome: EdgeMotion?
                if let side = step.set_side {
                    layout.setSide(side)
                    outcome = nil
                } else if step.cancel == true {
                    outcome = layout.cancel().map(EdgeMotion.returned) ?? .local
                } else {
                    let m = step.motion!
                    outcome = layout.motion(local: EdgePoint(x: m[0], y: m[1]), dx: m[2], dy: m[3],
                                            exposed: step.exposed ?? true, allowReturn: step.allow_return ?? true)
                }
                if let outcome { precondition(matches(outcome, step), "\(test.name) step \(index): \(outcome)") }
                if let side = step.side { precondition(layout.side == side, "\(test.name) step \(index): \(layout.side)") }
            }
        }
        print("PASS \(fixture.cases.count) shared edge layout cases and \(fixture.invalid_layouts.count) invalid layouts")
    }
}
