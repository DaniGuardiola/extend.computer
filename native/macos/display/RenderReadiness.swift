import Foundation

// GPU completion establishes a working decode/render pipeline. It is separate
// from compositor presentation, whose callback and timestamp may be absent.
final class RenderReadiness {
    private let lock = NSLock()
    private var ready = false
    private var handler: (() -> Void)?

    var callback: (() -> Void)? {
        get { lock.lock(); defer { lock.unlock() }; return handler }
        set { lock.lock(); handler = newValue; lock.unlock() }
    }

    @discardableResult
    func complete(success: Bool) -> Bool {
        lock.lock()
        guard success, !ready else { lock.unlock(); return false }
        ready = true
        let callback = handler
        lock.unlock()
        // Completion handlers run on Metal threads. Invoke outside the lock so
        // the framed stdout writer cannot hold up another completion callback.
        callback?()
        return true
    }
}
