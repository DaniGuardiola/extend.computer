import Foundation

@main struct DisplayInputTests {
    static func require(_ value: Bool) { precondition(value) }
    static func reject(_ action: () throws -> Void) {
        do { try action(); fatalError("Invalid display input accepted") } catch { }
    }
    static func target(_ bounds: String = #"{"x":-2000,"y":-100,"width":2000,"height":1000}"#,
                       _ content: String = #"{"x":0,"y":150,"width":1000,"height":500}"#) throws -> DisplayInputTarget {
        try DisplayInputTarget.decode(Data("{\"session\":\"test-session\",\"epoch\":9,\"revision\":3,\"display_id\":42,\"bounds\":\(bounds),\"canvas\":{\"width\":1000,\"height\":800},\"content\":\(content)}".utf8))
    }
    static func command(_ event: String, sequence: UInt64 = 1, epoch: UInt64 = 9,
                        revision: UInt64 = 3, session: String = "test-session") throws -> DisplayInputEnvelope {
        try DisplayInputEnvelope.decode(Data("{\"session\":\"\(session)\",\"epoch\":\(epoch),\"revision\":\(revision),\"sequence\":\(sequence),\"event\":\(event)}".utf8))
    }
    static func main() throws {
        let text = try command(#"{"kind":"text","value":"é🙂中文"}"#)
        var textBinding = DisplayInputBinding(target: try target())
        require(try textBinding.accept(text) == nil)
        var textState = InputState()
        require(try textState.apply(text.event.packet!) == [.text("é🙂中文")])
        reject { _ = try textBinding.accept(text) }
        reject { _ = try command(#"{"kind":"text","value":"é","x":0.5,"y":0.5}"#) }
        reject { _ = try command(#"{"kind":"text","value":"\ud800"}"#) }
        let mirror = try target()
        require(try mirror.point(x: 0.25, y: 0.65625) == CGPoint(x: -1500, y: 650))
        require(try mirror.point(x: 0, y: 0.1875) == CGPoint(x: -2000, y: -100))
        require(try mirror.point(x: 1, y: 0.8125) == CGPoint(x: -1, y: 899))
        reject { _ = try mirror.point(x: 0.25, y: 0.125) }
        reject { _ = try mirror.point(x: -0.001, y: 0.5) }
        reject { _ = try mirror.point(x: .nan, y: 0.5) }
        let portrait = try target(#"{"x":100,"y":-200,"width":1000,"height":2000}"#,
                                  #"{"x":300,"y":0,"width":400,"height":800}"#)
        require(try portrait.point(x: 0.5, y: 0.5) == CGPoint(x: 600, y: 800))
        reject { _ = try portrait.point(x: 0.2, y: 0.5) }
        let extend = try target(#"{"x":1920,"y":0,"width":1000,"height":800}"#,
                                #"{"x":0,"y":0,"width":1000,"height":800}"#)
        require(try extend.point(x: 0.5, y: 0.5) == CGPoint(x: 2420, y: 400))
        reject { _ = try target(#"{"x":0,"y":0,"width":0,"height":800}"#) }
        reject { _ = try target(#"{"x":-2000,"y":-100,"width":2000,"height":1000}"#, #"{"x":0,"y":300,"width":1000,"height":501}"#) }
        let move = #"{"kind":"move","x":0.25,"y":0.65625}"#
        var binding = DisplayInputBinding(target: mirror)
        require(try binding.accept(command(move)) == CGPoint(x: -1500, y: 650))
        reject { _ = try binding.accept(command(move)) }
        reject { _ = try binding.accept(command(move, sequence: 2, epoch: 8)) }
        reject { _ = try binding.accept(command(move, sequence: 2, revision: 2)) }
        reject { _ = try binding.accept(command(move, sequence: 2, session: "old")) }
        reject { _ = try binding.accept(command(#"{"kind":"move","x":0.25,"y":0.1}"#, sequence: 2)) }
        precondition(binding.sequence == 1)
        var owned = InputState()
        let down = try command(#"{"kind":"button","x":0.5,"y":0.5,"button":0,"down":true,"clicks":1}"#, sequence: 2)
        require(try binding.accept(down) == CGPoint(x: -1000, y: 400))
        require(try owned.apply(down.event.packet!) == [.button(0, true, 1, 0)])
        let scroll = try command(#"{"kind":"scroll","x":0.25,"y":0.65625,"dx":-42,"dy":15}"#, sequence: 3)
        require(try binding.accept(scroll) == CGPoint(x: -1500, y: 650))
        require(try owned.apply(scroll.event.packet!) == [.scroll(-42, 15, 0)])
        let release = try command(#"{"kind":"release"}"#, sequence: 4)
        require(try binding.accept(release) == nil)
        require(try owned.apply(release.event.packet!) == [.button(0, false, 1, 0)])
        precondition(owned.release().isEmpty)
        let modifiers = try command(#"{"kind":"modifiers","mask":9}"#, sequence: 5)
        require(try binding.accept(modifiers) == nil)
        require(try owned.apply(modifiers.event.packet!) == [.key(56, true, false, 1), .key(55, true, false, 9)])
        let key = try command(#"{"kind":"mac_key","code":0,"down":true,"repeat":false}"#, sequence: 6)
        require(try binding.accept(key) == nil)
        require(try owned.apply(key.event.packet!) == [.key(0, true, false, 9)])
        let repeating = try command(#"{"kind":"mac_key","code":0,"down":true,"repeat":true}"#, sequence: 7)
        require(try binding.accept(repeating) == nil)
        require(try owned.apply(repeating.event.packet!) == [.key(0, true, true, 9)])
        reject { _ = try binding.accept(command(#"{"kind":"mac_key","code":0,"down":false,"repeat":false}"#, sequence: 6)) }
        require(binding.sequence == 7 && owned.keys == [0] && owned.modifiers == 9)
        let keyboardRelease = try command(#"{"kind":"release"}"#, sequence: 8)
        require(try binding.accept(keyboardRelease) == nil)
        require(try owned.apply(keyboardRelease.event.packet!) == [.key(0, false, false, 9), .key(56, false, false, 8), .key(55, false, false, 0)])
        require(owned.keys.isEmpty && owned.modifiers == 0 && owned.release().isEmpty)
        for event in [#"{"kind":"mac_key","code":0,"down":false,"repeat":true}"#,
                      #"{"kind":"mac_key","code":128,"down":true,"repeat":false}"#,
                      #"{"kind":"mac_key","code":56,"down":true,"repeat":false}"#,
                      #"{"kind":"mac_key","code":0,"down":true,"repeat":false,"x":0.5}"#,
                      #"{"kind":"modifiers","mask":16}"#,
                      #"{"kind":"modifiers","mask":1,"y":0.5}"#,
                      #"{"kind":"mac_key","code":0}"#,
                      #"{"kind":"alive","x":0}"#,
                      #"{"kind":"move","x":0.5,"y":0.5,"extra":1}"#,
                      #"{"kind":"button","x":0.5,"y":0.5,"button":3,"down":true,"clicks":1}"#,
                      #"{"kind":"scroll","x":0.5,"y":0.5,"dx":0,"dy":4097}"#] {
            reject { _ = try command(event) }
        }
        reject { _ = try DisplayInputEnvelope.decode(Data(repeating: 32, count: 1025)) }
        print("PASS display fit, bars, negative origins, portrait, edge bounds, session/epoch/revision/sequence, atomic event positions, strict parser, bound keyboard/modifier actions and owned releases")
    }
}
