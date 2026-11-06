import CoreMedia
import ImageIO
import ReplayKit

/// ReplayKit broadcast upload extension: screen → hardware encoder → engine.
/// Mirrors ScreenSharingService.java (Android). All state lives on `queue`;
/// ReplayKit's sample callback hops onto it synchronously so frames are handled
/// one at a time and dropped (never queued) when the encoder is busy.
final class SampleHandler: RPBroadcastSampleHandler {
  private let queue = DispatchQueue(label: "computer.extend.broadcast")
  private var request: BroadcastRequest?
  private var preferences = SharingPreferences.defaults
  private var engine: EngineSender?
  private var encoder: BroadcastEncoder?
  private let transform = FrameTransform()
  private var timer: DispatchSourceTimer?

  private var phase = "connecting"
  private var failure = ""
  private var finished = false
  /// Upright size of the last captured frame.
  private var capture: (width: Int, height: Int)?
  private var started = Date()
  private var lastKeyframeRequest = Date.distantPast
  private var lastSample = Date.distantPast
  private var lastSubmitted: CMTime?
  /// Last encoder-sized frame, re-encoded when the receiver needs a keyframe
  /// while the screen is static (ReplayKit sends nothing for unchanged screens).
  private var latest: CVPixelBuffer?
  private var encoded = 0
  private var resizes = 0
  private var pendingUpdate: (id: String, preferences: SharingPreferences)?
  private var published: [String: Any] = [:]
  private var lastPublished = Date.distantPast

  // MARK: ReplayKit lifecycle

  override func broadcastStarted(withSetupInfo setupInfo: [String: NSObject]?) {
    queue.sync {
      do {
        guard BroadcastGroup.defaults != nil else { throw SharingError("Screen sharing isn't set up on this iPhone") }
        guard let value = BroadcastGroup.read(BroadcastGroup.requestKey) else { throw SharingError("Start sharing from extend.computer") }
        let request = try BroadcastRequest(value)
        guard Date().timeIntervalSince(request.created) < BroadcastGroup.requestLifetime,
              BroadcastGroup.defaults?.string(forKey: BroadcastGroup.consumedKey) != request.session else {
          throw SharingError("Start sharing from extend.computer")
        }
        BroadcastGroup.defaults?.set(request.session, forKey: BroadcastGroup.consumedKey)
        self.request = request
        preferences = request.preferences
        engine = try BroadcastEngine.make()
        started = Date()
        BroadcastGroup.observe(.stop) { [weak self] in self?.queue.async { self?.fail(SharingError("Sharing stopped from extend.computer"), stopped: true) } }
        BroadcastGroup.observe(.update) { [weak self] in self?.queue.async { self?.applyUpdate() } }
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: .milliseconds(100))
        timer.setEventHandler { [weak self] in self?.tick() }
        timer.resume()
        self.timer = timer
        publish(force: true)
      } catch {
        fail(error)
      }
    }
  }

  override func broadcastFinished() {
    queue.sync {
      if !finished { phase = "closed"; failure = "" }
      teardown()
    }
  }

  override func processSampleBuffer(_ sampleBuffer: CMSampleBuffer, with sampleBufferType: RPSampleBufferType) {
    guard sampleBufferType == .video else { return }
    queue.sync { handle(sampleBuffer) }
  }

  // MARK: Capture

  private func handle(_ sample: CMSampleBuffer) {
    guard !finished, let engine = self.engine, let request, let source = CMSampleBufferGetImageBuffer(sample) else { return }
    do {
      lastSample = Date()
      let raw = (CMGetAttachment(sample, key: RPVideoSampleOrientationKey as CFString, attachmentModeOut: nil) as? NSNumber)?.uint32Value
      let orientation = raw.flatMap(CGImagePropertyOrientation.init(rawValue:)) ?? .up
      let upright = FrameTransform.uprightSize(width: CVPixelBufferGetWidth(source), height: CVPixelBufferGetHeight(source), orientation: orientation)
      let changed = capture.map { $0 != upright } ?? false
      capture = upright
      if !startedEngine {
        // The first frame tells us the real capture size; start the session with it.
        try engine.start(peer: request.peer, address: request.address,
                         config: SharingConfig(preferences, screenWidth: upright.width, screenHeight: upright.height))
        startedEngine = true
        started = Date()
        publish(force: true)
        return
      }
      guard let encoder else { return }
      if changed {
        // Rotation: renegotiate the new shape at the current rate and bitrate.
        var next = try SharingConfig(preferences, screenWidth: upright.width, screenHeight: upright.height)
        next.fps = encoder.config.fps; next.bitrate = encoder.config.bitrate
        if next.width != encoder.config.width || next.height != encoder.config.height {
          try reconfigure(next)
          resizes += 1
          return
        }
      }
      let timestamp = CMSampleBufferGetPresentationTimeStamp(sample)
      if let lastSubmitted, CMTimeGetSeconds(CMTimeSubtract(timestamp, lastSubmitted)) < 0.9 / Double(encoder.config.fps) {
        return // pace to the negotiated frame rate
      }
      guard let image = try transform.process(source, orientation: orientation, width: encoder.config.width, height: encoder.config.height) else { return }
      if try encoder.encode(image, timestamp: timestamp) { lastSubmitted = timestamp }
      latest = image
    } catch {
      fail(error)
    }
  }
  private var startedEngine = false
  private func reconfigure(_ config: SharingConfig) throws {
    encoder?.invalidate(); encoder = nil; latest = nil
    try engine?.configure(config)
    phase = "connecting"; started = Date()
    publish(force: true)
  }

  // MARK: Engine supervision (every 100 ms, like Android's pump)

  private func tick() {
    guard !finished, let engine else { return }
    do {
      let state = engine.state()
      if startedEngine && (state.state == "failed" || state.state == "closed" || state.state == "idle") {
        throw SharingError(state.error.isEmpty ? "Screen connection ended" : state.error)
      }
      if encoder == nil {
        if startedEngine && state.state == "streaming" {
          let config = try SharingConfig(engine: state.config)
          encoder = try BroadcastEncoder(config: config, output: { [weak self] frame in self?.deliver(frame) },
                                         failure: { [weak self] error in self?.queue.async { self?.fail(error) } })
          lastSubmitted = nil
          phase = "streaming"
          if let update = pendingUpdate {
            preferences = update.preferences; pendingUpdate = nil
            answer(update.id, applied: true, error: "")
          }
          publish(force: true)
        } else if Date().timeIntervalSince(started) > 15 {
          throw SharingError("Screen connection timed out")
        }
      } else if let encoder, state.keyframe, Date().timeIntervalSince(lastKeyframeRequest) > 0.3 {
        encoder.requestKeyframe()
        lastKeyframeRequest = Date()
        if Date().timeIntervalSince(lastSample) > 0.1, let latest {
          // Static screen: no new sample will arrive to carry the keyframe.
          let now = CMClockGetTime(CMClockGetHostTimeClock())
          if try encoder.encode(latest, timestamp: now) { lastSubmitted = now }
        }
      }
      publish(force: false)
    } catch {
      fail(error)
    }
  }

  /// VideoToolbox callback thread. The engine queue is bounded and thread-safe.
  private func deliver(_ frame: EncodedFrame) {
    do {
      guard frame.timestamp > 0 else { return }
      try engine?.push(frame: frame.bytes, timestamp: frame.timestamp, keyframe: frame.keyframe)
      queue.async { self.encoded += 1 }
    } catch {
      queue.async { self.fail(error) }
    }
  }

  // MARK: Settings changes from the app

  private func applyUpdate() {
    guard !finished, let value = BroadcastGroup.read(BroadcastGroup.updateKey), let id = value["id"] as? String else { return }
    if let answered = BroadcastGroup.read(BroadcastGroup.updateResultKey), answered["id"] as? String == id { return }
    do {
      guard let raw = value["preferences"] as? [String: Any] else { throw SharingError("Invalid screen sharing settings") }
      let requested = try SharingPreferences(raw)
      guard phase == "streaming", pendingUpdate == nil, let encoder, let capture else {
        throw SharingError("Wait for screen sharing to connect before changing settings")
      }
      guard requested.codec == preferences.codec else { throw SharingError("Stop sharing to change the video codec") }
      let next = try SharingConfig(requested, screenWidth: capture.width, screenHeight: capture.height)
      if next == encoder.config {
        preferences = requested
        answer(id, applied: true, error: "")
        return
      }
      pendingUpdate = (id, requested)
      try reconfigure(next)
    } catch {
      answer(id, applied: false, error: "\(error)")
    }
  }

  private func answer(_ id: String, applied: Bool, error: String) {
    BroadcastGroup.write(BroadcastGroup.updateResultKey,
      ["id": id, "applied": applied, "error": error, "preferences": preferences.dictionary])
    BroadcastGroup.post(.state)
  }

  // MARK: State for the app

  private func publish(force: Bool) {
    var value: [String: Any] = [
      "session": request?.session ?? "", "peer": request?.peer ?? "", "state": phase, "error": failure,
      "encoded": encoded, "resizes": resizes, "footprint": Self.footprint(),
    ]
    if let engine, startedEngine { value["transport"] = engine.state().raw }
    let changed = value["state"] as? String != published["state"] as? String
      || value["error"] as? String != published["error"] as? String
    guard force || changed || Date().timeIntervalSince(lastPublished) >= BroadcastGroup.heartbeat else { return }
    published = value
    lastPublished = Date()
    value["updated"] = lastPublished.timeIntervalSince1970
    BroadcastGroup.write(BroadcastGroup.stateKey, value)
    BroadcastGroup.post(.state)
  }

  /// Physical footprint in bytes: what jetsam compares to the extension's
  /// ~50 MB limit. Published so the app (and on-device checks) can watch it.
  private static func footprint() -> Int {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
    let status = withUnsafeMutablePointer(to: &info) {
      $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count) }
    }
    return status == KERN_SUCCESS ? Int(info.phys_footprint) : 0
  }

  private func fail(_ error: Error, stopped: Bool = false) {
    guard !finished else { return }
    phase = stopped ? "closed" : "failed"
    failure = stopped ? "" : "\(error)"
    if let update = pendingUpdate { pendingUpdate = nil; answer(update.id, applied: false, error: failure.isEmpty ? "Screen sharing stopped" : failure) }
    teardown()
    // The only way an upload extension can end its broadcast; iOS shows the
    // message in an alert.
    finishBroadcastWithError(NSError(domain: "computer.extend.broadcast", code: stopped ? 0 : 1,
      userInfo: [NSLocalizedDescriptionKey: "\(error)"]))
  }

  private func teardown() {
    finished = true
    timer?.cancel(); timer = nil
    BroadcastGroup.ignore(.stop); BroadcastGroup.ignore(.update)
    encoder?.invalidate(); encoder = nil
    latest = nil
    transform.invalidate()
    engine?.stop(); engine = nil
    publish(force: true)
  }
}
