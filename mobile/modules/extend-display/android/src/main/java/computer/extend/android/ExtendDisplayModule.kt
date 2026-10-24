package computer.extend.android

import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class ExtendDisplayModule : Module() {
  private var nearby: NearbyDiscovery? = null
  private val incoming = IncomingPairing()
  private val visual = VisualPairing()
  private val manual = ManualPairing()
  private fun deviceName(): String {
    return NativeEngine.localName(requireNotNull(appContext.reactContext))
  }
  private fun localDeviceInfo(item: org.json.JSONObject): Map<String, Any> = mapOf(
    "name" to item.getString("name"), "defaultName" to item.getString("defaultName"), "customName" to item.getBoolean("customName"),
    "fingerprint" to item.getString("fingerprint"), "version" to item.getString("version"), "build" to item.getString("build")
  )
  private fun peerDeviceInfo(item: org.json.JSONObject): Map<String, Any?> = mapOf(
    "peer" to item.getString("peer"), "name" to item.getString("name"), "address" to item.getString("address"), "trustSource" to item.optString("trustSource", "manual"),
    "defaultName" to item.optString("defaultName", item.getString("name")),
    "authenticatedName" to if (item.isNull("authenticatedName")) null else item.getString("authenticatedName"),
    "label" to if (item.isNull("label")) null else item.getString("label")
  )
  private fun discovery(): NearbyDiscovery {
    return nearby ?: NearbyDiscovery(requireNotNull(appContext.reactContext)).also { nearby = it }
  }
  private fun connectionActive(): Boolean {
    val sharing = ScreenSharingService.state().optString("state")
    val media = org.json.JSONObject(NativeEngine.displayState()).optString("state")
    val control = org.json.JSONObject(NativeEngine.controlState()).optString("state")
    return sharing in listOf("consent", "connecting", "streaming") ||
      media in listOf("offered", "connecting", "configuring", "streaming") || control in listOf("connecting", "connected")
  }
  override fun definition() = ModuleDefinition {
    Name("ExtendDisplay")
    AsyncFunction("getAccountState") { Accounts.get(requireNotNull(appContext.reactContext)).state() }
    AsyncFunction("resumeAccount") { Accounts.get(requireNotNull(appContext.reactContext)).resume() }
    AsyncFunction("refreshAccount") { Accounts.get(requireNotNull(appContext.reactContext)).refresh() }
    AsyncFunction("startAccountBrowser") {
      check(appContext.currentActivity?.hasWindowFocus() == true) { "Return to the app to sign in" }
      Accounts.get(requireNotNull(appContext.reactContext)).browser()
    }
    AsyncFunction("startAccountPassword") {
      val account = Accounts.get(requireNotNull(appContext.reactContext))
      account.prepare()
      check(appContext.currentActivity?.hasWindowFocus() == true) { "Return to the app to sign in" }
      AccountLoginDialog.show(requireNotNull(appContext.currentActivity), account)
    }
    AsyncFunction("cancelAccountSignIn") { Accounts.get(requireNotNull(appContext.reactContext)).cancel() }
    AsyncFunction("logoutAccount") { Accounts.get(requireNotNull(appContext.reactContext)).logout() }
    AsyncFunction("configureAccount") { server: String -> Accounts.get(requireNotNull(appContext.reactContext)).configure(requireNotNull(appContext.reactContext), server) }
    AsyncFunction("removeAccountDevice") { id: String -> Accounts.get(requireNotNull(appContext.reactContext)).remove(id) }
    AsyncFunction("openAccountPage") { page: String -> Accounts.get(requireNotNull(appContext.reactContext)).openPage(page) }
    Function("hasActiveConnection") { connectionActive() }

    Function("startDiscovery") { discovery().start() }
    Function("stopDiscovery") { nearby?.stop() }
    Function("getNearbyDevices") {
      val snapshot = discovery().snapshot()
      val items = snapshot.getJSONArray("candidates")
      mapOf("state" to snapshot.getString("state"), "candidates" to (0 until items.length()).map { index ->
        val item = items.getJSONObject(index)
        val addresses = item.getJSONArray("addresses")
        mapOf("id" to item.getString("id"), "name" to item.getString("name"), "pairing" to item.getBoolean("pairing"),
          "addresses" to (0 until addresses.length()).map { addresses.getString(it) })
      })
    }
    AsyncFunction("startVisualPairing") { address: String, name: String ->
      check(appContext.currentActivity?.hasWindowFocus() == true) { "Return to the app to pair a device" }
      check(!connectionActive()) { "Stop the current connection before pairing" }
      visual.start(requireNotNull(appContext.reactContext), address, name, requireNotNull(appContext.currentActivity))
    }
    Function("getVisualPairing") { id: String -> visual.state(requireNotNull(appContext.reactContext), id) }
    Function("respondVisualPairing") { id: String, request: String, allow: Boolean ->
      check(appContext.currentActivity?.hasWindowFocus() == true) { "Return to the app to confirm pairing" }
      visual.respond(id, request, allow)
    }
    Function("stopVisualPairing") { id: String -> visual.stop(id) }
    Function("startIncomingPairing") {
      check(!connectionActive()) { "Stop the current connection before pairing" }
      incoming.start(requireNotNull(appContext.reactContext), deviceName())
    }
    Function("getIncomingPairing") { incoming.state() }
    Function("respondIncomingPairing") { request: String, allow: Boolean ->
      check(appContext.currentActivity?.hasWindowFocus() == true) { "Return to the app to confirm pairing" }
      NativeEngine.respondIncoming(request, allow); Unit
    }
    Function("stopIncomingPairing") { incoming.stop() }
    Function("setReceivingEnabled") { enabled: Boolean -> ReceivingService.enable(requireNotNull(appContext.reactContext), enabled) }
    Function("getInputReceivingState") { InputReceivingService.state(requireNotNull(appContext.reactContext)).toString() }
    Function("setInputReceivingEnabled") { enabled: Boolean -> InputReceivingService.enable(requireNotNull(appContext.reactContext),enabled) }
    Function("openInputReceivingSettings") { InputReceivingService.openSettings(requireNotNull(appContext.reactContext)) }
    Function("getReceivingState") { ReceivingService.state(requireNotNull(appContext.reactContext)).toString() }
    AsyncFunction("startScreenSharing") { peer: String ->
      try {
        val context = requireNotNull(appContext.reactContext)
        ScreenSharingOptions.encoder(ScreenSharingOptions.config(context, ScreenSharingOptions.read(context,peer)))
        val checked = NativeEngine.check(context, peer)
        val address = if (checked.optString("route") == "usb") "usb:${checked.getString("carrier_id")}" else {
          check(checked.optString("route") != "internet") { "Screen sharing needs a local connection" }
          val metadata = context.getSharedPreferences("engine-peers", 0).getString(peer, null)
            ?: error("Device address unavailable")
          val saved = org.json.JSONObject(metadata).getString("address")
          "${saved.substringBeforeLast(':')}:48179"
        }
        check(appContext.currentActivity?.hasWindowFocus() == true) { "Return to the app to start screen sharing" }
        ScreenSharingService.request(context, peer, address)
        mapOf("started" to true, "error" to "")
      } catch (error: ScreenSharingOptions.Unsupported) {
        mapOf("started" to false, "error" to (error.message ?: "These screen sharing settings aren't supported"))
      } catch (error: Exception) {
        mapOf("started" to false, "error" to "Couldn't start sharing. Check the other device is available and try again.")
      }
    }
    Function("getScreenSharingPreferences") { peer: String -> ScreenSharingOptions.read(requireNotNull(appContext.reactContext),peer).toString() }
    Function("setScreenSharingPreferences") { peer: String, value: String -> ScreenSharingOptions.save(requireNotNull(appContext.reactContext),peer,value) }
    Function("getScreenSharingState") { ScreenSharingService.state().toString() }
    AsyncFunction("updateScreenSharingPreferences") { peer: String, value: String ->
      try {
        val updated = ScreenSharingService.update(requireNotNull(appContext.reactContext),peer,value)
        mapOf("applied" to true,"error" to "","preferences" to updated.toString())
      } catch (error: Exception) {
        mapOf("applied" to false,"error" to (error.message ?: "Couldn't update screen sharing settings"),"preferences" to "")
      }
    }
    Function("stopScreenSharing") { ScreenSharingService.stop(requireNotNull(appContext.reactContext)) }
    Function("resumeReceiving") { ReceivingService.resume(requireNotNull(appContext.reactContext)) }
    OnActivityEntersBackground { nearby?.suspend(); incoming.stop(); visual.stop(); manual.stop() }
    OnActivityEntersForeground { nearby?.resume(); appContext.reactContext?.let { ReceivingService.resume(it); try { Accounts.get(it).resume() } catch (_: Exception) {} } }
    OnDestroy { nearby?.stop(); nearby = null; incoming.stop(); visual.stop(); manual.stop() }

    AsyncFunction("getEngineIdentity") {
      val context = requireNotNull(appContext.reactContext) { "Android app context unavailable" }
      val identity = NativeEngine.identity(context)
      mapOf("fingerprint" to identity.getString("fingerprint"), "publicKey" to identity.getString("public_key"))
    }
    AsyncFunction("getLocalDeviceInfo") { localDeviceInfo(DeviceSettings.localInfo(requireNotNull(appContext.reactContext))) }
    AsyncFunction("setLocalDeviceName") { name: String ->
      check(appContext.currentActivity?.hasWindowFocus() == true) { "Return to the app to change device name" }
      localDeviceInfo(DeviceSettings.setLocal(requireNotNull(appContext.reactContext), name))
    }
    AsyncFunction("getPeerDeviceInfo") { peer: String -> peerDeviceInfo(DeviceSettings.peerInfo(requireNotNull(appContext.reactContext), peer)) }
    AsyncFunction("setPeerDeviceLabel") { peer: String, label: String ->
      check(appContext.currentActivity?.hasWindowFocus() == true) { "Return to the app to change device label" }
      peerDeviceInfo(DeviceSettings.setPeerLabel(requireNotNull(appContext.reactContext), peer, label))
    }
    AsyncFunction("setPeerDeviceAddress") { peer: String, address: String ->
      check(appContext.currentActivity?.hasWindowFocus() == true) { "Return to the app to change device address" }
      peerDeviceInfo(NativeEngine.setPeerAddress(requireNotNull(appContext.reactContext), peer, address))
    }

    AsyncFunction("getPairedDevices") {
      val context = requireNotNull(appContext.reactContext)
      val devices = NativeEngine.devices(context)
      (0 until devices.length()).map { index ->
        val item = devices.getJSONObject(index)
        peerDeviceInfo(item)
      }
    }
    Function("reserveManualPairing") {
      check(!connectionActive()) { "Stop the current connection before pairing" }
      manual.reserve(requireNotNull(appContext.currentActivity))
    }
    AsyncFunction("startManualPairing") { id: String, address: String, code: String, name: String ->
      check(!connectionActive()) { "Stop the current connection before pairing" }
      manual.start(requireNotNull(appContext.reactContext), id, address, code, name)
    }
    AsyncFunction("getManualPairing") { id: String -> manual.state(requireNotNull(appContext.reactContext), id) }
    Function("stopManualPairing") { id: String -> manual.stop(id) }
    AsyncFunction("pairDevice") { address: String, code: String, name: String ->
      check(!connectionActive()) { "Stop the current connection before pairing" }
      val item = NativeEngine.pair(requireNotNull(appContext.reactContext), address, code, name)
      mapOf("peer" to item.getString("peer"), "name" to item.getString("name"), "address" to item.getString("address"))
    }
    AsyncFunction("startControl") { peer: String ->
      val context = requireNotNull(appContext.reactContext)
      val checked = NativeEngine.check(context, peer)
      val address = when (checked.optString("route")) {
        "usb" -> "usb:${checked.getString("carrier_id")}"
        "internet" -> ""
        else -> context.getSharedPreferences("engine-peers", 0).getString(peer, null)
          ?.let { org.json.JSONObject(it).getString("address") } ?: error("Device address unavailable")
      }
      NativeEngine.startControl(peer, address)
    }
    Function("stopControl") { id: String -> NativeEngine.stopControl(id); Unit }
    Function("controlEvent") { id: String, event: String -> NativeEngine.controlEvent(id, event); Unit }
    AsyncFunction("getDeviceAvailability") { peer: String ->
      if (appContext.currentActivity?.hasWindowFocus() != true)
        return@AsyncFunction mapOf("state" to "unavailable", "reason" to "Return to the app to check availability")
      val result = DeviceAvailability.read(requireNotNull(appContext.reactContext), peer)
      mapOf("state" to result.getString("state"), "reason" to result.optString("reason", ""), "route" to result.optString("route", "lan"))
    }
    AsyncFunction("checkDevice") { peer: String ->
      check(!connectionActive()) { "Stop the current connection before probing another device" }
      val result = NativeEngine.check(requireNotNull(appContext.reactContext), peer)
      mapOf("rttMs" to result.getDouble("rtt_ms"))
    }
    AsyncFunction("getRemovedPeers") {
      val context = requireNotNull(appContext.reactContext)
      NativeEngine.identity(context)
      val removed = org.json.JSONObject(NativeEngine.removedPeers())
      removed.keys().asSequence().map { peer -> mapOf("peer" to peer, "name" to removed.getString(peer)) }.toList()
    }
    AsyncFunction("dismissRemovedPeer") { peer: String ->
      NativeEngine.identity(requireNotNull(appContext.reactContext))
      NativeEngine.dismissRemovedPeer(peer)
    }
    AsyncFunction("forgetDevice") { peer: String ->
      check(!connectionActive()) { "Stop the current connection before forgetting a device" }
      NativeEngine.forget(requireNotNull(appContext.reactContext), peer)
    }

    Function("getAppearance") {
      appContext.reactContext?.getSharedPreferences("appearance", 0)?.getString("value", "system") ?: "system"
    }
    Function("setAppearance") { value: String ->
      require(value in listOf("system", "light", "dark"))
      appContext.reactContext?.getSharedPreferences("appearance", 0)?.edit()?.putString("value", value)?.apply()
    }

    Function("getDeviceName") {
      deviceName()
    }
    Function("getMediaState") { NativeEngine.displayState() }
    AsyncFunction("getDisplayReceiverSettings") { id: String -> NativeEngine.displayReceiverSettings(id) }
    AsyncFunction("requestDisplayReceiverSettings") { id: String, revision: Long, settings: String ->
      check(appContext.currentActivity?.hasWindowFocus() == true) { "Return to the display to change settings" }
      NativeEngine.requestDisplayReceiverSettings(id, revision, settings)
    }
    Function("getLaunchSession") {
      val media = org.json.JSONObject(NativeEngine.displayState())
      if (media.optString("state") in listOf("offered", "connecting", "streaming")) {
        val peer = media.getString("peer")
        val saved = appContext.reactContext?.getSharedPreferences("engine-peers", 0)?.getString(peer, null)
        val name = saved?.let { org.json.JSONObject(it).optString("name", "") }?.takeIf { it.isNotEmpty() }
          ?: org.json.JSONObject(NativeEngine.peerNames()).optString(peer, "Paired device")
        return@Function mapOf("available" to true, "host" to name,
          "mode" to media.getJSONObject("config").getString("mode"), "id" to media.getString("id"))
      }
      val intent = appContext.currentActivity?.intent
      val port = intent?.getIntExtra("port", 0) ?: 0
      val valid = port in 1024..65535 && intent?.getStringExtra("token")?.length == 64
      mapOf("available" to valid, "host" to (intent?.getStringExtra("peer") ?: "Mac"),
        "mode" to (intent?.getStringExtra("mode") ?: "extend"))
    }
    Function("clearLaunchSession") { id: String? ->
      id?.let { NativeEngine.cancelDisplay(it) }
      appContext.currentActivity?.intent?.let { it.removeExtra("port"); it.removeExtra("token") }
    }
    Function("setImmersive") { enabled: Boolean ->
      appContext.currentActivity?.let { activity ->
        activity.runOnUiThread {
          val controller = WindowCompat.getInsetsController(activity.window, activity.window.decorView)
          if (enabled) {
            controller.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller.hide(WindowInsetsCompat.Type.systemBars())
          } else controller.show(WindowInsetsCompat.Type.systemBars())
        }
      }
    }
    View(ExtendDisplayView::class) {
      Events("onStatus", "onInputStatus", "onKeyboardStatus")
      AsyncFunction("editingShortcut") { view: ExtendDisplayView, token: String, action: String -> view.editingShortcut(token, action) }
      Prop("touchEnabled") { view: ExtendDisplayView, enabled: Boolean -> view.setTouchEnabled(enabled) }
      Prop("keyboardEnabled") { view: ExtendDisplayView, enabled: Boolean -> view.setKeyboardEnabled(enabled) }
      Prop("sessionId") { view: ExtendDisplayView, id: String? -> view.setSession(id) }
    }
    View(ExtendControlView::class) {
      Name("Control")
      Events("onStatus")
      AsyncFunction("editingShortcut") { view: ExtendControlView, token: String, action: String -> view.editingShortcut(token, action) }
      Prop("sessionId") { view: ExtendControlView, id: String? -> view.setSession(id) }
      Prop("keyboard") { view: ExtendControlView, enabled: Boolean -> view.setKeyboard(enabled) }
    }
  }
}
