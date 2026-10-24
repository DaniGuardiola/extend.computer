package computer.extend.android

import android.content.Context
import android.graphics.Color
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.MotionEvent
import android.view.KeyEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.text.Editable
import android.text.SpannableStringBuilder
import android.text.InputType
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import org.json.JSONObject
import android.view.SurfaceHolder
import android.view.SurfaceView
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.views.ExpoView
import expo.modules.kotlin.viewevent.EventDispatcher
import java.net.InetSocketAddress
import java.net.Socket

class ExtendDisplayView(context: Context, appContext: AppContext) : ExpoView(context, appContext), SurfaceHolder.Callback {
  private val onStatus by EventDispatcher()
  private val onInputStatus by EventDispatcher()
  private val onKeyboardStatus by EventDispatcher()
  private val inputHandler = Handler(Looper.getMainLooper())
  private var touchEnabled = false
  private var keyboardEnabled = false
  private var imeVisible = false
  private var imeGeneration = 0
  private var keyboardBinding = ""
  private var keyboardError = ""
  private var textSupported = false
  private var shortcutPolicy: KeyboardShortcutPolicy? = null
  private val keyboard = DisplayKeyboard({ textSupported }, ::sendKeyboard)
  private var decoderRevision = -1L
  @Volatile private var frameSeen = false
  private var requestedRevision = -1L
  private var grantRevision = -1L
  private var grantBinding = ""
  private var inputStatus = "disabled"
  private var lastInputStatus = ""
  private var grantGeometry: JSONObject? = null
  private val viewConfiguration = ViewConfiguration.get(context)
  private val gestures = DisplayTouch(viewConfiguration.scaledTouchSlop.toDouble(),
    viewConfiguration.scaledDoubleTapSlop.toDouble(), ViewConfiguration.getLongPressTimeout().toLong(),
    ViewConfiguration.getDoubleTapTimeout().toLong(), ::sendTouch)
  private val inputPad = InputPad(context)
  private val hold = Runnable { if (canTouch()) gestures.hold(SystemClock.uptimeMillis()) }
  fun setTouchEnabled(enabled: Boolean) {
    if (touchEnabled == enabled) return
    touchEnabled = enabled
    if (keyboardEnabled) { gestures.cancel(); inputHandler.removeCallbacks(hold) } else suspendInput()
    inputHandler.removeCallbacks(pollInput)
    if (isAttachedToWindow) inputHandler.post(pollInput)
  }
  private fun canInput() = frameSeen && decoderRevision >= 0 && inputStatus == "granted"
    && grantRevision == decoderRevision && grantGeometry != null && trustedId != null && isShown && hasWindowFocus()
  private fun canTouch() = touchEnabled && canInput()
  private fun canKeyboard() = keyboardEnabled && canInput() && inputPad.hasFocus()
  private fun shortcutToken(): String? = if (keyboardEnabled && canInput() && isAttachedToWindow &&
    video.holder.surface.isValid && shortcutPolicy != null) "$trustedId:$grantBinding:$imeGeneration" else null
  fun editingShortcut(token: String, action: String): Boolean {
    val shortcut = EditingShortcut.fromWire(action) ?: throw IllegalArgumentException("Unknown shortcut")
    check(token == shortcutToken()) { "Keyboard input changed. Try again." }
    val policy = requireNotNull(shortcutPolicy)
    check(keyboard.canShortcut()) { "Release physical keys before using shortcuts." }
    if (keyboard.shortcut(shortcut, policy)) return true
    suspendInput()
    return false
  }
  fun setKeyboardEnabled(enabled: Boolean) {
    if (keyboardEnabled == enabled) return
    keyboard.release()
    keyboardEnabled = enabled
    keyboardError = ""
    hideKeyboard()
    if (!enabled && !touchEnabled) suspendInput()
    publishKeyboard()
    inputHandler.removeCallbacks(pollInput)
    if (isAttachedToWindow) inputHandler.post(pollInput)
  }
  private fun publishKeyboard() {
    onKeyboardStatus(mapOf("requested" to keyboardEnabled, "visible" to imeVisible, "error" to keyboardError))
  }
  private fun hideKeyboard() {
    imeGeneration++
    keyboardBinding = ""
    (context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager)
      .hideSoftInputFromWindow(inputPad.windowToken, 0)
  }
  private fun showKeyboard() {
    if (!keyboardEnabled || !canInput() || keyboardBinding == grantBinding) return
    keyboardBinding = grantBinding
    imeGeneration++
    inputPad.requestFocus()
    val manager = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
    manager.restartInput(inputPad)
    manager.showSoftInput(inputPad, InputMethodManager.SHOW_IMPLICIT)
    publishKeyboard()
  }
  private fun publishInput(state: String, reason: String = "") {
    val key = "$state:$reason:${shortcutPolicy?.wire}:${shortcutToken()}"
    if (lastInputStatus == key) return
    lastInputStatus = key
    onInputStatus(mapOf("state" to state, "reason" to reason,
      "shortcutPolicy" to (shortcutPolicy?.wire ?: ""), "shortcutToken" to (shortcutToken() ?: "")))
  }
  private fun suspendInput() {
    keyboard.release()
    hideKeyboard()
    inputHandler.removeCallbacks(hold)
    gestures.cancel()
    trustedId?.let { runCatching { NativeEngine.displayInputCommand(it, decoderRevision, "{\"kind\":\"stop\"}") }; runCatching { NativeEngine.setDisplayInputEnabled(it, false) } }
    requestedRevision = -1
    grantRevision = -1
    grantBinding = ""
    grantGeometry = null
    shortcutPolicy = null
    gestures.geometry = null
    inputStatus = if (touchEnabled || keyboardEnabled) "suspended" else "disabled"
    publishInput(inputStatus)
  }
  private val pollInput = object : Runnable {
    override fun run() {
      if (!isAttachedToWindow) return
      val id = trustedId
      try {
        if (id == null) { inputStatus = "unsupported"; publishInput("unsupported", "View-only display") }
        else if (!touchEnabled && !keyboardEnabled) { inputStatus = "disabled"; publishInput("disabled") }
        else if (frameSeen && decoderRevision >= 0 && isShown && hasWindowFocus() && video.holder.surface.isValid) {
          if (requestedRevision != decoderRevision) {
            // Mark before requesting: unsupported/denied input must not turn
            // into a permission request every poll.
            requestedRevision = decoderRevision
            runCatching { NativeEngine.setDisplayInputEnabled(id, true) }
          }
          val snapshot = JSONObject(NativeEngine.displayInputState(id))
          val phase = snapshot.optString("state", "suspended")
          textSupported = snapshot.optBoolean("text_supported", false)
          val binding = snapshot.optJSONObject("binding")
          val revision = snapshot.optLong("revision", -1)
          val bindingKey = binding?.toString() ?: ""
          val geometry = snapshot.optJSONObject("geometry")
          val raster = geometry?.optJSONObject("raster")
          val canvas = geometry?.optJSONObject("canvas")
          val valid = phase == "granted" && revision == decoderRevision
            && binding?.optLong("revision", -1) == decoderRevision
            && raster?.optInt("width") == sourceWidth && raster?.optInt("height") == sourceHeight
            && canvas?.optInt("width") == logicalWidth && canvas?.optInt("height") == logicalHeight
          if (!valid || bindingKey != grantBinding) {
            gestures.cancel(); inputHandler.removeCallbacks(hold)
            // The previous binding was revoked remotely. Never replay its key
            // releases into the replacement lease; Stop releases the old lease.
            keyboard.reset(); hideKeyboard()
          }
          inputStatus = if (phase == "granted" && !valid) "suspended" else phase
          grantRevision = if (valid) revision else -1
          grantBinding = if (valid) bindingKey else ""
          grantGeometry = if (valid) snapshot.getJSONObject("geometry") else null
          shortcutPolicy = if (valid) KeyboardShortcutPolicy.fromWire(snapshot.optString("shortcut_policy")) else null
          if (valid) updateTouchGeometry() else gestures.geometry = null
          if (valid) showKeyboard()
          publishInput(inputStatus, snapshot.optString("reason"))
        }
      } catch (error: Exception) {
        keyboard.reset(); hideKeyboard()
        gestures.cancel(); inputHandler.removeCallbacks(hold)
        grantRevision = -1; grantGeometry = null; gestures.geometry = null
        shortcutPolicy = null
        inputStatus = "suspended"; publishInput(inputStatus, error.message ?: "Touch paused")
      }
      inputHandler.postDelayed(this, 250)
    }
  }
  private fun updateTouchGeometry() {
    val geometry = grantGeometry ?: return
    val content = geometry.optJSONObject("content") ?: return
    val source = geometry.optJSONObject("source_size") ?: return
    gestures.geometry = TouchGeometry(video.left.toDouble(), video.top.toDouble(), video.width.toDouble(), video.height.toDouble(),
      content.optDouble("x"), content.optDouble("y"), content.optDouble("width"), content.optDouble("height"),
      source.optDouble("width"), source.optDouble("height"))
  }
  private fun sendTouch(command: TouchCommand) {
    val id = trustedId ?: return
    if (command != TouchCommand.Release && !canTouch()) return
    val value = JSONObject()
    fun point(p: TouchPoint) = JSONObject().put("x", p.x).put("y", p.y)
    when (command) {
      is TouchCommand.Move -> value.put("kind", "move").put("point", point(command.point))
      is TouchCommand.Button -> value.put("kind", "input").put("point", point(command.point))
        .put("event", JSONObject().put("kind", "button").put("button", command.button).put("down", command.down).put("clicks", command.clicks))
      is TouchCommand.Scroll -> value.put("kind", "input").put("point", point(command.point))
        .put("event", JSONObject().put("kind", "scroll").put("dx", command.dx).put("dy", command.dy))
      TouchCommand.Release -> {
        keyboard.reset()
        value.put("kind", "input").put("event", JSONObject().put("kind", "release"))
      }
    }
    try { NativeEngine.displayInputCommand(id, decoderRevision, value.toString()) }
    catch (_: Exception) { inputStatus = "suspended"; grantRevision = -1; inputHandler.removeCallbacks(hold); gestures.cancel(); publishInput("suspended", "Touch paused") }
  }
  private fun sendKeyboard(command: KeyboardCommand): Boolean {
    val id = trustedId ?: return false
    if (!canInput()) return false
    val event = JSONObject()
    when (command) {
      is KeyboardCommand.Key -> event.put("kind", "mac_key").put("code", command.code)
        .put("down", command.down).put("repeat", command.repeat)
      is KeyboardCommand.Modifiers -> event.put("kind", "modifiers").put("mask", command.mask)
      is KeyboardCommand.Text -> event.put("kind", "text").put("value", command.value)
    }
    return try {
      NativeEngine.displayInputCommand(id, decoderRevision,
        JSONObject().put("kind", "input").put("event", event).toString())
      true
    } catch (_: Exception) {
      keyboard.reset()
      runCatching { NativeEngine.displayInputCommand(id, decoderRevision, "{\"kind\":\"stop\"}") }
      grantRevision = -1; inputStatus = "suspended"
      hideKeyboard(); publishInput("suspended", "Keyboard paused")
      false
    }
  }
  private inner class InputPad(context: Context) : View(context) {
    init { isFocusableInTouchMode = true; contentDescription = "Remote display input" }
    override fun onCheckIsTextEditor() = keyboardEnabled
    override fun onFocusChanged(gainFocus: Boolean, direction: Int, previouslyFocusedRect: android.graphics.Rect?) {
      super.onFocusChanged(gainFocus, direction, previouslyFocusedRect)
      if (!gainFocus) { keyboard.release(); imeGeneration++ }
    }
    override fun onKeyPreIme(keyCode: Int, event: KeyEvent): Boolean {
      if (keyCode == KeyEvent.KEYCODE_BACK && imeVisible) {
        if (event.action == KeyEvent.ACTION_UP) setKeyboardEnabled(false)
        return true
      }
      return super.onKeyPreIme(keyCode, event)
    }
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
      val code = DisplayKeyboard.physical(event.keyCode)
      val modifier = DisplayKeyboard.isModifier(event.keyCode)
      if (!canKeyboard() || (code == null && !modifier)) return super.dispatchKeyEvent(event)
      if (event.action != KeyEvent.ACTION_DOWN && event.action != KeyEvent.ACTION_UP) return false
      val mask = (if (event.isShiftPressed) 1 else 0) or (if (event.isCtrlPressed) 2 else 0) or
        (if (event.isAltPressed) 4 else 0) or (if (event.isMetaPressed) 8 else 0)
      return keyboard.hardware(code, mask, event.action == KeyEvent.ACTION_DOWN, event.repeatCount > 0)
    }
    override fun onCreateInputConnection(info: EditorInfo): InputConnection? {
      if (!canKeyboard()) return null
      info.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
      info.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING or EditorInfo.IME_ACTION_NONE
      val token = ++imeGeneration
      val revision = decoderRevision
      val binding = grantBinding
      return object : BaseInputConnection(this, true) {
        private val composing = SpannableStringBuilder()
        private var closed = false
        private fun current() = !closed && token == imeGeneration && revision == decoderRevision && binding == grantBinding && canKeyboard()
        private val commits = KeyboardCommitSession(keyboard) { current() }
        override fun closeConnection() {
          closed = true
          commits.close()
          composing.clear()
          super.closeConnection()
        }
        private fun rejected(): Boolean {
          keyboardError = "Text requires a compatible peer, up to 64 characters, and released physical keys."
          publishKeyboard()
          return false
        }
        override fun getEditable(): Editable = composing
        override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
          if (!current()) return false
          if ((text?.length ?: 0) > DisplayKeyboard.MAX_TEXT * 2) return rejected()
          // Keep composition local until commitText/finishComposingText.
          composing.clear()
          return super.setComposingText(text ?: "", newCursorPosition)
        }
        override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
          if (!current()) return false
          if (!commits.commit(text?.toString() ?: "")) return rejected()
          composing.clear(); keyboardError = ""; publishKeyboard()
          return true
        }
        override fun finishComposingText(): Boolean = if (!current()) false else commitText(composing.toString(), 1)
        override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
          if (!current()) return false
          if (beforeLength !in 0..DisplayKeyboard.MAX_TEXT || afterLength !in 0..DisplayKeyboard.MAX_TEXT || beforeLength + afterLength > DisplayKeyboard.MAX_TEXT) return rejected()
          if (composing.isNotEmpty()) return super.deleteSurroundingText(beforeLength, afterLength)
          return if (keyboard.delete(beforeLength, afterLength)) true else rejected()
        }
        override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean {
          if (!current()) return false
          if (beforeLength !in 0..DisplayKeyboard.MAX_TEXT || afterLength !in 0..DisplayKeyboard.MAX_TEXT || beforeLength + afterLength > DisplayKeyboard.MAX_TEXT) return rejected()
          if (composing.isNotEmpty()) return super.deleteSurroundingTextInCodePoints(beforeLength, afterLength)
          return deleteSurroundingText(beforeLength, afterLength)
        }
        override fun sendKeyEvent(event: KeyEvent): Boolean = current() && dispatchKeyEvent(event)
      }
    }
    override fun onTouchEvent(event: MotionEvent): Boolean {
      if (!canTouch()) { gestures.cancel(); inputHandler.removeCallbacks(hold); return false }
      val points = (0 until event.pointerCount).associate { index ->
        event.getPointerId(index) to TouchPoint(event.getX(index).toDouble(), event.getY(index).toDouble())
      }
      val id = event.getPointerId(event.actionIndex)
      val point = points.getValue(id)
      when (event.actionMasked) {
        MotionEvent.ACTION_DOWN -> {
          if (!gestures.down(id, point, event.eventTime)) return false
          if (keyboardEnabled) requestFocus()
          parent.requestDisallowInterceptTouchEvent(true)
          inputHandler.postDelayed(hold, ViewConfiguration.getLongPressTimeout().toLong())
        }
        MotionEvent.ACTION_POINTER_DOWN -> { inputHandler.removeCallbacks(hold); gestures.additional(id, points, event.eventTime) }
        MotionEvent.ACTION_MOVE -> gestures.move(points)
        MotionEvent.ACTION_POINTER_UP -> { inputHandler.removeCallbacks(hold); gestures.move(points); gestures.up(id, point, event.eventTime) }
        MotionEvent.ACTION_UP -> { inputHandler.removeCallbacks(hold); gestures.move(points); gestures.up(id, point, event.eventTime); gestures.finish(); performClick() }
        MotionEvent.ACTION_CANCEL -> { inputHandler.removeCallbacks(hold); gestures.cancel() }
      }
      return true
    }
    override fun performClick(): Boolean { super.performClick(); return true }
  }
  override fun onAttachedToWindow() { super.onAttachedToWindow(); inputHandler.post(pollInput) }
  override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
    super.onWindowFocusChanged(hasWindowFocus)
    if (!hasWindowFocus) { suspendInput(); cursor.compositorSurfaceLost() }
    else publishCursorSurface()
  }
  override fun onVisibilityAggregated(isVisible: Boolean) {
    super.onVisibilityAggregated(isVisible)
    cursorAggregatedVisible = isVisible
    if (!isVisible) { suspendInput(); cursor.compositorSurfaceLost() }
    else publishCursorSurface()
  }
  private val video = SurfaceView(context)
  private val cursor = CursorOverlay(context)
  @Volatile private var socket: Socket? = null
  @Volatile private var generation = 0
  private var terminalGeneration = -1
  private val connectionLock = Any()
  private var worker: Thread? = null
  private var trustedId: String? = null
  fun setSession(id: String?) {
    if (trustedId == id) return
    stop(); trustedId = id
    if (video.holder.surface.isValid) connect()
  }
  private var sourceWidth = 1280
  private var sourceHeight = 800
  private var logicalWidth = 1280
  private var logicalHeight = 800
  private var cursorSurfaceEpoch = 0L
  private var cursorAggregatedVisible = false
  override val shouldUseAndroidLayout = true

  init {
    setBackgroundColor(Color.BLACK)
    keepScreenOn = true
    video.holder.addCallback(this)
    addView(video, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
    addView(inputPad, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
    addView(cursor, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
    ViewCompat.setOnApplyWindowInsetsListener(this) { _, insets ->
      val visible = insets.isVisible(WindowInsetsCompat.Type.ime())
      if (imeVisible != visible) { imeVisible = visible; publishKeyboard() }
      insets
    }
  }

  override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
    val scale = minOf(width.toDouble() / sourceWidth, height.toDouble() / sourceHeight)
    val fittedWidth = (sourceWidth * scale).toInt()
    val fittedHeight = (sourceHeight * scale).toInt()
    val x = (width - fittedWidth) / 2
    val y = (height - fittedHeight) / 2
    video.layout(x, y, x + fittedWidth, y + fittedHeight)
    inputPad.layout(0, 0, width, height)
    cursor.layout(0, 0, width, height)
    cursor.geometry(x.toFloat(), y.toFloat(), fittedWidth.toFloat(), fittedHeight.toFloat(), logicalWidth, logicalHeight)
    publishCursorSurface()
    updateTouchGeometry()
  }

  // Keep SurfaceHolder's default size-from-layout contract: no fixed size or
  // native window geometry override. Its producer default extent is the fitted
  // view size; the active decoded buffer can be smaller. A native child attaches
  // to the video's BLAST layer, so raster-to-holder scaling and view translation
  // are inherited. Pass both extents, never the global offset. Native creation
  // requires ANativeWindow == holder == fitted view, with admitted raster kept
  // separately for cursor coordinates and current decoded-frame validation.
  private fun publishCursorSurface() {
    val holder = video.holder
    val frame = holder.surfaceFrame
    cursor.compositorSurface(holder.surface, cursorSurfaceEpoch, decoderRevision,
      sourceWidth, sourceHeight, logicalWidth, logicalHeight,
      frame.width(), frame.height(), video.width, video.height,
      trustedId != null && frameSeen && decoderRevision >= 0 && holder.surface.isValid
        && cursorAggregatedVisible && isShown && windowVisibility == View.VISIBLE && hasWindowFocus())
  }
  override fun surfaceCreated(holder: SurfaceHolder) {
    cursorSurfaceEpoch++
    publishCursorSurface()
    connect()
  }
  override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
    cursorSurfaceEpoch++
    publishCursorSurface()
  }
  override fun surfaceDestroyed(holder: SurfaceHolder) {
    cursorSurfaceEpoch++
    cursor.compositorSurfaceLost()
    stop()
  }
  override fun onDetachedFromWindow() { inputHandler.removeCallbacks(pollInput); stop(); super.onDetachedFromWindow() }

  private fun status(session: Int, state: String, message: String = "") {
    post {
      if (session == generation) {
        if (state == "disconnected") {
          terminalGeneration = session
          suspendInput(); frameSeen = false; decoderRevision = -1
          cursor.compositorSurfaceLost()
        }
        onStatus(mapOf("state" to state, "message" to message))
      }
    }
  }

  private fun stop() {
    suspendInput(); frameSeen = false; decoderRevision = -1
    cursor.compositorSurfaceLost()
    synchronized(connectionLock) {
      generation++
      cursor.stop()
      trustedId?.let { runCatching { NativeEngine.cancelDisplay(it) } }
      runCatching { socket?.close() }
      socket = null
      worker = null
    }
  }

  private fun connect() {
    if (worker?.isAlive == true) return
    val id = trustedId
    val intent = appContext.currentActivity?.intent
    val port = intent?.getIntExtra("port", 0) ?: 0
    val token = intent?.getStringExtra("token")
    if (id == null && (port !in 1024..65535 || token == null || !token.matches(Regex("[0-9a-f]{64}")))) return
    val surface = video.holder.surface
    val session = ++generation
    status(session, "connecting")
    worker = Thread({
      try {
        val listener = object : DisplaySession.Listener {
          override fun configured(width: Int, height: Int) {
            configured(width, height, width, height)
          }
          override fun configured(width: Int, height: Int, sourceLogicalWidth: Int, sourceLogicalHeight: Int) {
            post {
              if (session == generation && terminalGeneration != session) {
                sourceWidth = width; sourceHeight = height
                logicalWidth = sourceLogicalWidth; logicalHeight = sourceLogicalHeight
                publishCursorSurface()
                requestLayout()
                onStatus(mapOf("state" to "connecting", "width" to width, "height" to height))
              }
            }
          }
          override fun configured(width: Int, height: Int, sourceLogicalWidth: Int, sourceLogicalHeight: Int, revision: Long) {
            post {
              if (session == generation && terminalGeneration != session) {
                suspendInput(); frameSeen = false; decoderRevision = revision
                sourceWidth = width; sourceHeight = height
                logicalWidth = sourceLogicalWidth; logicalHeight = sourceLogicalHeight
                publishCursorSurface()
                requestLayout()
                onStatus(mapOf("state" to "connecting", "width" to width, "height" to height))
              }
            }
          }
          override fun rendered() {
            rendered(-1)
          }
          override fun rendered(revision: Long) {
            if (!frameSeen) post {
              if (session == generation && terminalGeneration != session && revision == decoderRevision && !frameSeen) {
                frameSeen = true
                publishCursorSurface()
                onStatus(mapOf("state" to "streaming"))
              }
            }
          }
        }
        if (id != null) {
          val snapshot = org.json.JSONObject(NativeEngine.displayState())
          require(snapshot.optString("id") == id) { "Display session replaced" }
          if (snapshot.getJSONObject("config").optBoolean("separate_cursor")) {
            post { if (session == generation) cursor.start(id) }
          }
          MediaSession.receive(id, snapshot.getJSONObject("config"), surface, listener)
        } else {
          Socket().use { connection ->
            synchronized(connectionLock) {
              if (session != generation) return@Thread
              socket = connection
            }
            connection.connect(InetSocketAddress("127.0.0.1", port), 10000)
            connection.tcpNoDelay = true; connection.soTimeout = 15000
            DisplaySession.receive(connection, requireNotNull(token), surface, listener)
          }
        }
        status(session, "disconnected", "Display session ended.")
      } catch (error: Exception) {
        id?.let { runCatching { NativeEngine.cancelDisplay(it) } }
        if (session == generation) {
          android.util.Log.e("ExtendDisplay", "Display session ended", error)
          status(session, "disconnected", error.message ?: "Connection ended.")
        }
      } finally {
        if (session == generation) {
          socket = null
          post { if (session == generation) { suspendInput(); frameSeen = false; cursor.compositorSurfaceLost(); cursor.stop() } }
        }
      }
    }, "extend-display-session").also { it.start() }
  }
}
