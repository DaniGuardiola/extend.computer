import Foundation

@main struct LayoutTests {
    static func main() throws {
        var left = try EdgeLayout(localWidth: 1920, localHeight: 1080, remoteWidth: 1280, remoteHeight: 800, side: .left, offsetY: 100)
        precondition(left.motion(local: EdgePoint(x: 0, y: 50), dx: -2, dy: 0, exposed: true) == .local)
        precondition(left.motion(local: EdgePoint(x: 0, y: 150), dx: -2, dy: 0, exposed: false) == .local)
        precondition(left.motion(local: EdgePoint(x: 0, y: 150), dx: 2, dy: 0, exposed: true) == .local)
        precondition(left.motion(local: EdgePoint(x: 0, y: 150), dx: -2, dy: 0, exposed: true) == .remote(EdgePoint(x: 1, y: 50.0 / 799)))
        precondition(left.motion(local: EdgePoint(x: 0, y: 150), dx: -1279, dy: 0, exposed: true) == .remote(EdgePoint(x: 0, y: 50.0 / 799)))
        precondition(left.motion(local: EdgePoint(x: 0, y: 150), dx: -50, dy: -1000, exposed: true) == .remote(EdgePoint(x: 0, y: 0)))
        precondition(left.motion(local: EdgePoint(x: 0, y: 150), dx: 1280, dy: 60, exposed: true) == .returned(EdgePoint(x: 4, y: 160)))
        precondition(left.motion(local: EdgePoint(x: 0, y: 160), dx: -2, dy: 0, exposed: true) == .local)
        _ = left.motion(local: EdgePoint(x: 10, y: 160), dx: 10, dy: 0, exposed: true)
        precondition(left.motion(local: EdgePoint(x: 0, y: 160), dx: -10, dy: 0, exposed: true) == .remote(EdgePoint(x: 1, y: 60.0 / 799)))
        precondition(left.cancel() == EdgePoint(x: 4, y: 160))
        precondition(left.cancel() == nil)
        print("PASS left edge, unequal sizes, offset gap, exposed-edge check, direction, far-edge clamp, return height, hysteresis, cancel")

        var right = try EdgeLayout(localWidth: 1000, localHeight: 600, remoteWidth: 800, remoteHeight: 1200, side: .right, offsetY: -300)
        precondition(right.motion(local: EdgePoint(x: 999, y: 200), dx: 2, dy: 0, exposed: true) == .remote(EdgePoint(x: 0, y: 500.0 / 1199)))
        _ = right.motion(local: EdgePoint(x: 999, y: 200), dx: 10, dy: 600, exposed: true)
        precondition(right.motion(local: EdgePoint(x: 999, y: 200), dx: -20, dy: 0, exposed: true) == .remote(EdgePoint(x: 0, y: 1100.0 / 1199)))
        precondition(right.motion(local: EdgePoint(x: 999, y: 200), dx: -1, dy: -700, exposed: true) == .returned(EdgePoint(x: 995, y: 100)))
        print("PASS right edge, negative offset, blocked return through non-overlap, valid return")

        var live = try EdgeLayout(localWidth: 1000, localHeight: 600, remoteWidth: 800, remoteHeight: 600, side: .left, offsetY: 0)
        live.setSide(.right)
        precondition(live.side == .right)
        _ = live.motion(local: EdgePoint(x: 999, y: 200), dx: 2, dy: 0, exposed: true)
        let position = live.remotePosition
        live.setSide(.left)
        precondition(live.side == .right && live.remotePosition == position)
        // Returning still uses the old edge; only subsequent crossings use the new one.
        precondition(live.motion(local: EdgePoint(x: 999, y: 200), dx: -1, dy: 0, exposed: true) == .returned(EdgePoint(x: 995, y: 200)))
        precondition(live.side == .left && live.remotePosition == nil)
        _ = live.motion(local: EdgePoint(x: 990, y: 200), dx: -5, dy: 0, exposed: true)
        _ = live.motion(local: EdgePoint(x: 0, y: 200), dx: -5, dy: 0, exposed: true)
        precondition(live.remotePosition != nil)
        live.setSide(.right)
        live.setSide(.left)
        _ = live.motion(local: EdgePoint(x: 0, y: 200), dx: 1, dy: 0, exposed: true)
        precondition(live.side == .left && live.remotePosition == nil)
        print("PASS live edge changes, deferred remote edits, unchanged return route, latest edit wins")

        for sizes in [(0.0, 600.0, 800.0, 1200.0, 0.0), (1000, 600, 800, 1200, 600), (1000, 600, 800, 1200, -1200), (1000, 600, Double.nan, 1200, 0)] {
            do {
                _ = try EdgeLayout(localWidth: sizes.0, localHeight: sizes.1, remoteWidth: sizes.2, remoteHeight: sizes.3, side: .left, offsetY: sizes.4)
                fatalError("Invalid layout accepted")
            } catch {}
        }
        print("PASS invalid and nonoverlapping layouts rejected")
    }
}
