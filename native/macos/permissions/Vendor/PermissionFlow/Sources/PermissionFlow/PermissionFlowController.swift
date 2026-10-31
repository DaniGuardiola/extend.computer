#if os(macOS)
import AppKit
import Combine
import SystemSettingsKit
import SwiftUI

@available(macOS 13.0, *)
@MainActor
public final class PermissionFlowController: ObservableObject {
    /// The package exposes a single active floating panel at a time so opening
    /// a second permission flow closes the previous panel automatically.
    private static var activeController: PermissionFlowController?
    private let systemSettingsBundleIdentifier = "com.apple.systempreferences"

    /// Apps currently represented in the floating panel.
    @Published public private(set) var droppedApps: [URL]

    /// The permission pane currently being guided.
    @Published public private(set) var currentPane: PermissionFlowPane?

    /// Drives the visibility of the "reopen settings" action.
    @Published var isSettingsFrontmost = false

    /// Drives the header icon animation while the app card is being dragged.
    @Published var isDraggingApp = false

    /// Drives the locale environment used by the floating SwiftUI panel.
    @Published public private(set) var localeIdentifier: String?

    @Published var guidanceTitle: String?
    @Published var guidanceBody: String?
    @Published var pointerSide: PermissionGuidePointerSide = .none
    @Published var pointerY: CGFloat = 66
    @Published private(set) var targetFrame: CGRect?
    private var openGuidedSettings: (() -> Void)?
    private var guideTargetIdentifier: String?
    private var guidePaneTitles: Set<String> = []
    private var refreshTimer: Timer?
    private var presentation = PermissionGuidePresentation(now: ProcessInfo.processInfo.systemUptime)
    private var targetQueryRunning = false
    private var lastTargetQuery: TimeInterval = 0
    private var sessionID = UUID()
    private var emptyWindowCount = 0
    private var hasPresented = false

    /// A temporarily hidden guide remains active and must keep checking grants.
    public var isActive: Bool { panel != nil && Self.activeController === self }

    /// Guidance for settings toggles that do not accept dragged apps.
    public func guide(title: String, body: String, targetIdentifier: String? = nil, paneTitles: Set<String> = [], openSettings: @escaping () -> Void) {
        closeOtherActivePanelIfNeeded()
        closePanel()
        rememberPreviousFrontmostApplication()
        currentPane = nil
        guidanceTitle = title
        guidanceBody = body
        guideTargetIdentifier = targetIdentifier
        guidePaneTitles = paneTitles
        openGuidedSettings = openSettings
        openSettings()
        Self.activeController = self
        showPanel()
        tracker.startTracking(promptIfNeeded: false)
    }

    public var onDrop: ((URL) -> Void)?

    private let configuration: PermissionFlowConfiguration
    private let tracker = SettingsWindowTracker()

    private var panel: FloatingDropPanel?
    private var previousFrontmostApplicationPID: pid_t?
    private var previousFrontmostApplicationBundleIdentifier: String?
    private var cancellables = Set<AnyCancellable>()

    public init(configuration: PermissionFlowConfiguration = .init()) {
        self.configuration = configuration
        self.droppedApps = configuration.requiredAppURLs.uniqueAppURLs()
        self.localeIdentifier = configuration.localeIdentifier

        updateFrontmostAppState()
        bindTrackerCallbacks()
        observeFrontmostApplication()
    }

    /// Opens the requested privacy pane and starts the floating guidance flow.
    ///
    /// - Parameters:
    ///   - pane: The permission pane to open inside System Settings.
    ///   - suggestedAppURLs: Optional `.app` bundle URLs that should appear in
    ///     the floating panel as drag candidates. This parameter defaults to an
    ///     empty array, which means no explicit app list is injected here.
    ///     When this value is empty and no previously registered app is
    ///     available, the floating panel falls back to `Bundle.main.bundleURL`
    ///     if the current host bundle is itself an `.app`.
    ///   - sourceFrameInScreen: Retained for source compatibility. The guide now
    ///     waits for settled Settings geometry instead of flying from the source.
    public func authorize(
        pane: PermissionFlowPane,
        suggestedAppURLs: [URL] = [],
        sourceFrameInScreen: CGRect? = nil
    ) {
        closeOtherActivePanelIfNeeded()

        closePanel()
        rememberPreviousFrontmostApplication()
        guidanceTitle = nil
        guidanceBody = nil
        openGuidedSettings = nil
        currentPane = pane
        guideTargetIdentifier = nil
        guidePaneTitles = []
        mergeDroppedApps(with: suggestedAppURLs)
        SystemSettings.open(url: pane.settingsURL)

        guard pane.supportsFloatingAuthorizationPanel else { return }

        Self.activeController = self
        showPanel()
        tracker.startTracking(promptIfNeeded: configuration.promptForAccessibilityTrust)
    }

    /// Create the guide without revealing it at a provisional screen position.
    public func showPanel() {
        guard panel == nil else { return }
        closeOtherActivePanelIfNeeded()
        Self.activeController = self
        hasPresented = false
        panel = FloatingDropPanel(controller: self)
        presentation = PermissionGuidePresentation(now: ProcessInfo.processInfo.systemUptime)
        sessionID = UUID()
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 0.2, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refreshGuide() }
        }
        refreshTimer?.tolerance = 0.04
        refreshGuide()
    }

    public func closePanel(returnToPreviousApp: Bool = false) {
        refreshTimer?.invalidate()
        refreshTimer = nil
        sessionID = UUID()
        targetQueryRunning = false
        lastTargetQuery = 0
        targetFrame = nil
        isDraggingApp = false
        emptyWindowCount = 0
        hasPresented = false
        tracker.stopTracking()
        panel?.close()
        panel = nil

        if Self.activeController === self {
            Self.activeController = nil
        }

        if returnToPreviousApp {
            reactivatePreviousFrontmostApplication()
        }
    }

    public func resetDroppedApps() {
        droppedApps = configuration.requiredAppURLs.uniqueAppURLs()
    }

    /// Updates the locale injected into the floating panel.
    public func setLocaleIdentifier(_ localeIdentifier: String?) {
        guard self.localeIdentifier != localeIdentifier else { return }
        self.localeIdentifier = localeIdentifier
        panel?.updateLocaleIdentifier(localeIdentifier)
    }

    /// Registers a unique `.app` bundle URL and notifies the host if needed.
    public func registerDroppedApp(_ url: URL) {
        guard url.pathExtension.lowercased() == "app" else { return }
        let normalizedURL = url.standardizedFileURL
        guard droppedApps.contains(normalizedURL) == false else { return }
        droppedApps.append(normalizedURL)
        onDrop?(normalizedURL)
    }

    /// The panel always renders a single primary app card. If the host has not
    /// supplied one yet, the host application's bundle becomes the fallback.
    var preferredAppURL: URL? {
        if let first = droppedApps.first {
            return first
        }
        let bundleURL = Bundle.main.bundleURL.standardizedFileURL
        return bundleURL.pathExtension.lowercased() == "app" ? bundleURL : nil
    }

    /// The panel becomes mouse-transparent while dragging so System Settings
    /// underneath can receive the drop.
    func setPanelDragging(_ isDragging: Bool) {
        isDraggingApp = isDragging
        panel?.setDraggingPassthrough(isDragging)
        if !isDragging { refreshGuide() }
    }

    /// Keeps System Settings visually present whenever the floating panel is
    /// clicked or momentarily considered for focus.
    func keepSettingsVisible() {
        SystemSettings.activate()
        refreshGuide()
    }

    func reopenCurrentSettingsPane() {
        if let openGuidedSettings { openGuidedSettings(); return }
        guard let currentPane else { return }
        SystemSettings.open(url: currentPane.settingsURL)
        refreshGuide()
    }

    /// Merges unique app bundle URLs into the current panel list.
    func mergeDroppedApps(with urls: [URL]) {
        for url in urls.uniqueAppURLs() {
            registerDroppedApp(url)
        }
    }

    private func bindTrackerCallbacks() {
        tracker.onFrameChange = { [weak self] frame in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.presentPanel(self.panel, for: frame)
            }
        }
        tracker.onTrackingEnded = { [weak self] in
            Task { @MainActor [weak self] in
                self?.closePanel()
            }
        }
    }

    private func observeFrontmostApplication() {
        NSWorkspace.shared.notificationCenter
            .publisher(for: NSWorkspace.didActivateApplicationNotification)
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in
                self?.updateFrontmostAppState()
                self?.refreshGuide()
            }
            .store(in: &cancellables)
    }

    private func closeOtherActivePanelIfNeeded() {
        if let activeController = Self.activeController, activeController !== self {
            activeController.closePanel()
        }
    }

    private func rememberPreviousFrontmostApplication() {
        let frontmostApplication = NSWorkspace.shared.frontmostApplication
        guard frontmostApplication?.bundleIdentifier != systemSettingsBundleIdentifier else { return }
        previousFrontmostApplicationPID = frontmostApplication?.processIdentifier
        previousFrontmostApplicationBundleIdentifier = frontmostApplication?.bundleIdentifier
    }

    private func reactivatePreviousFrontmostApplication() {
        defer {
            previousFrontmostApplicationPID = nil
            previousFrontmostApplicationBundleIdentifier = nil
        }

        if let previousFrontmostApplicationPID,
           let application = NSRunningApplication(processIdentifier: previousFrontmostApplicationPID) {
            application.activate(options: [.activateIgnoringOtherApps])
            return
        }

        guard let previousFrontmostApplicationBundleIdentifier else { return }
        NSRunningApplication.runningApplications(withBundleIdentifier: previousFrontmostApplicationBundleIdentifier)
            .first?
            .activate(options: [.activateIgnoringOtherApps])
    }

    private func presentPanel(_ panel: FloatingDropPanel?, for settingsFrame: CGRect) {
        guard !isDraggingApp else { return }
        let now = ProcessInfo.processInfo.systemUptime
        if let previous = presentation.frame, previous != settingsFrame {
            if previous.size == settingsFrame.size {
                targetFrame = targetFrame?.offsetBy(dx: settingsFrame.minX - previous.minX,
                                                  dy: settingsFrame.minY - previous.minY)
            } else { targetFrame = nil; lastTargetQuery = 0 }
        }
        presentation.observe(settingsFrame, now: now)
        panel?.snap(to: settingsFrame)
    }

    private func refreshGuide() {
        guard isActive, let panel else { return }
        // Check all spaces so a desktop switch never counts as closing Settings.
        if presentation.frame != nil, let hasWindow = settingsHasWindow() {
            emptyWindowCount = hasWindow ? 0 : emptyWindowCount + 1
            if emptyWindowCount >= 10 { closePanel(); return }
        }
        guard !isDraggingApp, let frame = tracker.currentFrame else { return }
        presentPanel(panel, for: frame)
        let front = NSWorkspace.shared.frontmostApplication?.bundleIdentifier
        let settingsForeground = front == systemSettingsBundleIdentifier
        let now = ProcessInfo.processInfo.systemUptime
        let visible = settingsForeground && (hasPresented || presentation.canReveal(settingsForeground: true, now: now))
        panel.setVisible(visible)
        if visible { hasPresented = true }
        guard settingsForeground, !isDraggingApp, !targetQueryRunning, now - lastTargetQuery >= 1 else { return }
        let identifier = guideTargetIdentifier ?? preferredAppURL.map { $0.lastPathComponent + "_Toggle" }
        let titles = currentPane.map { Set([$0.localizedTitle(localeIdentifier: localeIdentifier)]) } ?? guidePaneTitles
        guard let identifier, !titles.isEmpty else { return }
        targetQueryRunning = true
        lastTargetQuery = now
        let id = sessionID
        DispatchQueue.global(qos: .utility).async { [weak self] in
            let target = PermissionGuideTarget.find(identifier: identifier, paneTitles: titles)
            DispatchQueue.main.async {
                guard let self, self.sessionID == id, self.isActive else { return }
                self.targetQueryRunning = false
                guard !self.isDraggingApp, self.tracker.currentFrame == frame else { self.lastTargetQuery = 0; return }
                self.targetFrame = target.flatMap { PermissionGuideTarget.appKitFrame($0) }
                self.refreshGuide()
            }
        }
    }

    private func settingsHasWindow() -> Bool? {
        let pids = Set(NSRunningApplication.runningApplications(withBundleIdentifier: systemSettingsBundleIdentifier).map(\.processIdentifier))
        guard let windows = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { return nil }
        return windows.contains { window in
            guard let pid = window[kCGWindowOwnerPID as String] as? Int32, pids.contains(pid),
                  window[kCGWindowLayer as String] as? Int == 0,
                  let bounds = window[kCGWindowBounds as String] as? NSDictionary,
                  let frame = CGRect(dictionaryRepresentation: bounds) else { return false }
            return frame.width > 320 && frame.height > 240
        }
    }

    private func updateFrontmostAppState() {
        isSettingsFrontmost =
            NSWorkspace.shared.frontmostApplication?.bundleIdentifier == systemSettingsBundleIdentifier
    }
}

@available(macOS 13.0, *)
private extension Array where Element == URL {
    /// Normalizes and de-duplicates `.app` bundle URLs.
    func uniqueAppURLs() -> [URL] {
        var seen = Set<String>()
        return compactMap { url in
            let normalized = url.standardizedFileURL
            guard normalized.pathExtension.lowercased() == "app" else { return nil }
            return seen.insert(normalized.path).inserted ? normalized : nil
        }
    }

    /// Uses normalized file paths for containment because equivalent file URLs
    /// can differ in their string representation.
    func contains(_ url: URL) -> Bool {
        contains(where: { $0.standardizedFileURL.path == url.standardizedFileURL.path })
    }
}
#endif
