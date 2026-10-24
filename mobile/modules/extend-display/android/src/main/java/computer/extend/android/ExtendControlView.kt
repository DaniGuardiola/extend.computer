package computer.extend.android

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.text.Editable
import android.text.SpannableStringBuilder
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.views.ExpoView
import expo.modules.kotlin.viewevent.EventDispatcher
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.roundToInt

/** Touch and physical keys stay native; React receives connection status only. */
class ExtendControlView(context: Context, appContext: AppContext) : ExpoView(context, appContext) {
  private val onStatus by EventDispatcher()
  private var id: String? = null
  private var connected = false
  private var lastPhase = ""
  private var lastSent = -1L
  private var remoteWidth = 1920.0
  private var remoteHeight = 1080.0
  private var x = .5
  private var y = .5
  private var previousX = 0f
  private var previousY = 0f
  private var travel = 0f
  private var twoFinger = false
  private val touchGate = ControlTouchGate()
  private var keyboard = false
  private var imeGeneration = 0
  private val imeRecovery = ImeFocusRecovery()
  private var imeRecoveryQueued = false
  private var textSupported = false
  private var shortcutPolicy: KeyboardShortcutPolicy? = null
  private val keyboardState = DisplayKeyboard({ textSupported }) { command ->
    val event = JSONObject()
    when (command) {
      is KeyboardCommand.Key -> event.put("kind","mac_key").put("code",command.code).put("down",command.down).put("repeat",command.repeat)
      is KeyboardCommand.Modifiers -> event.put("kind","modifiers").put("mask",command.mask)
      is KeyboardCommand.Text -> event.put("kind","text").put("value",command.value)
    }
    send(event)
  }
  private fun canTouch() = connected && isAttachedToWindow && isShown && hasWindowFocus()
  private fun shortcutToken(): String? = if (keyboard && canTouch() && shortcutPolicy != null) "$id:$imeGeneration" else null
  fun editingShortcut(token: String, action: String): Boolean {
    val shortcut = EditingShortcut.fromWire(action) ?: throw IllegalArgumentException("Unknown shortcut")
    check(token == shortcutToken()) { "Keyboard input changed. Try again." }
    val policy = requireNotNull(shortcutPolicy)
    check(keyboardState.canShortcut()) { "Release physical keys before using shortcuts." }
    if (keyboardState.shortcut(shortcut, policy)) return true
    // A partial chord must close this exact session so the receiver releases it.
    id?.let { runCatching { NativeEngine.stopControl(it) } }
    connected = false
    shortcutPolicy = null
    return false
  }
  private val hold = Runnable {
    if (!twoFinger && travel < slop*2 && touchGate.hold(canTouch())) button(true)
  }
  private val handler = Handler(Looper.getMainLooper())
  private val slop = ViewConfiguration.get(context).scaledTouchSlop
  private val surface = Pad(context)
  private fun recoverIme() {
    if (imeRecoveryQueued || !imeRecovery.needed(imeGeneration)) return
    val generation = imeGeneration
    val session = id
    imeRecoveryQueued = true
    handler.post {
      imeRecoveryQueued = false
      val eligible = session != null && session == id && generation == imeGeneration &&
        keyboard && connected && isAttachedToWindow && isShown && hasWindowFocus() && surface.hasFocus()
      if (imeRecovery.claim(generation, eligible)) {
        // Blur already discarded the old connection's permission to commit.
        // Refresh once after focus returns; never replay its composing draft or
        // repeatedly show the keyboard from the native status polling loop.
        (context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager).restartInput(surface)
      }
      // Multiple focus transitions may coalesce behind this UI task. Only the
      // latest invalidation can get a new one-shot task; ineligible current
      // generations wait for a real focus/connection transition instead.
      if (generation != imeGeneration) recoverIme()
    }
  }
  override val shouldUseAndroidLayout = true
  init { addView(surface, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT)) }
  override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) { surface.layout(0,0,width,height) }
  fun setSession(value: String?) { if (id == value) return; stop(); id = value; handler.post(poll) }
  fun setKeyboard(value: Boolean) {
    imeGeneration++
    imeRecovery.clear()
    if (!value) keyboardState.release()
    keyboard = value
    val ime = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
    if (value) { surface.requestFocus(); ime.restartInput(surface); ime.showSoftInput(surface,InputMethodManager.SHOW_IMPLICIT) }
    else ime.hideSoftInputFromWindow(surface.windowToken,0)
  }
  private val poll = object : Runnable {
    override fun run() {
      val session = id ?: return
      try {
        val state = JSONObject(NativeEngine.controlState())
        if (state.optString("id") != session) { connected = false; onStatus(mapOf("state" to "closed", "error" to "Control session replaced")); return }
        val wasConnected = connected
        connected = state.getString("state") == "connected"
        if (connected && !wasConnected) recoverIme()
        keepScreenOn = connected
        val phase = state.getString("state"); val sent = state.optLong("sent")
        if (phase != lastPhase || (sent > 0 && sent / 32 != lastSent / 32)) {
          android.util.Log.i("ExtendControl", "Control stats: $state")
          lastPhase = phase; lastSent = sent
        }
        textSupported = connected && state.optBoolean("text_supported",false)
        shortcutPolicy = if (connected) KeyboardShortcutPolicy.fromWire(state.optString("shortcut_policy")) else null
        if (connected) { remoteWidth = state.getDouble("width"); remoteHeight = state.getDouble("height") }
        onStatus(mapOf("state" to state.getString("state"), "error" to state.optString("error"),
          "shortcutPolicy" to (shortcutPolicy?.wire ?: ""), "shortcutToken" to (shortcutToken() ?: "")))
        if (state.getString("state") in listOf("connecting","connected")) handler.postDelayed(this,250)
      } catch (error: Exception) { connected = false; onStatus(mapOf("state" to "failed", "error" to (error.message ?: "Control ended"))) }
    }
  }
  private fun send(event: JSONObject): Boolean {
    if (!connected) return false
    return try { NativeEngine.controlEvent(requireNotNull(id),event.toString()); true }
    catch (error: Exception) { connected = false; onStatus(mapOf("state" to "failed", "error" to (error.message ?: "Control ended"))); false }
  }
  private fun button(down: Boolean, button: Int = 0) { send(JSONObject().put("kind","button").put("button",button).put("down",down).put("clicks",1)) }
  private fun move(dx: Float, dy: Float) {
    x = (x + dx / remoteWidth).coerceIn(0.0,1.0); y = (y + dy / remoteHeight).coerceIn(0.0,1.0)
    send(JSONObject().put("kind","move").put("x",x).put("y",y))
  }
  private fun cancelTouch() {
    handler.removeCallbacks(hold)
    if (touchGate.finish()) button(false)
  }
  private fun stop() { cancelTouch(); imeGeneration++; imeRecovery.clear(); keyboardState.reset(); textSupported=false; shortcutPolicy=null; keepScreenOn=false; handler.removeCallbacks(poll); id?.let { runCatching { NativeEngine.stopControl(it) } }; connected = false }
  override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
    super.onWindowFocusChanged(hasWindowFocus)
    if (!hasWindowFocus) { cancelTouch(); imeGeneration++; imeRecovery.invalidate(imeGeneration); keyboardState.release() }
    else recoverIme()
  }
  override fun onDetachedFromWindow() { stop(); super.onDetachedFromWindow() }
  override fun onWindowVisibilityChanged(visibility: Int) { super.onWindowVisibilityChanged(visibility); if (visibility != View.VISIBLE) stop() }

  private inner class Pad(context: Context) : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.argb(36,128,128,128); strokeWidth = 1f }
    init { isFocusableInTouchMode = true; contentDescription = "Remote trackpad"; setBackgroundColor(Color.TRANSPARENT) }
    override fun onFocusChanged(gainFocus: Boolean, direction: Int, previouslyFocusedRect: android.graphics.Rect?) {
      super.onFocusChanged(gainFocus,direction,previouslyFocusedRect)
      if (!gainFocus) { cancelTouch(); imeGeneration++; imeRecovery.invalidate(imeGeneration); keyboardState.release() }
      else recoverIme()
    }
    override fun onDraw(canvas: Canvas) {
      val step = 32 * resources.displayMetrics.density
      var at = step; while (at < width) { canvas.drawLine(at,0f,at,height.toFloat(),paint); at += step }
      at = step; while (at < height) { canvas.drawLine(0f,at,width.toFloat(),at,paint); at += step }
    }
    override fun onTouchEvent(event: MotionEvent): Boolean {
      if (!canTouch()) { cancelTouch(); return true }
      if (event.actionMasked == MotionEvent.ACTION_DOWN) {
        cancelTouch()
        if (!touchGate.begin(canTouch())) return true
      } else if (!touchGate.accepts(canTouch())) return true
      requestFocus(); parent.requestDisallowInterceptTouchEvent(true)
      if (event.actionMasked == MotionEvent.ACTION_DOWN) android.util.Log.i("ExtendControl", "Trackpad gesture received")
      val cx = (0 until event.pointerCount).map { event.getX(it) }.average().toFloat()
      val cy = (0 until event.pointerCount).map { event.getY(it) }.average().toFloat()
      when (event.actionMasked) {
        MotionEvent.ACTION_DOWN -> { previousX=cx; previousY=cy; travel=0f; twoFinger=false; handler.postDelayed(hold,500) }
        MotionEvent.ACTION_POINTER_DOWN -> { handler.removeCallbacks(hold); if (touchGate.releaseHeld()) button(false); twoFinger=true; previousX=cx; previousY=cy }
        MotionEvent.ACTION_MOVE -> {
          val dx=cx-previousX; val dy=cy-previousY; travel += abs(dx)+abs(dy)
          if (travel >= slop*2) handler.removeCallbacks(hold)
          if (event.pointerCount >= 2) {
            send(JSONObject().put("kind","scroll").put("dx",dx.roundToInt().coerceIn(-4096,4096)).put("dy",dy.roundToInt().coerceIn(-4096,4096)))
          } else if (!twoFinger) {
            move(dx,dy)
          }
          previousX=cx; previousY=cy
        }
        MotionEvent.ACTION_POINTER_UP -> { /* Suppress the remaining finger until the gesture ends. */ }
        MotionEvent.ACTION_UP -> {
          handler.removeCallbacks(hold)
          if (touchGate.finish()) button(false)
          else if (travel < slop*2) { button(true,if (twoFinger) 1 else 0); button(false,if (twoFinger) 1 else 0) }
          performClick()
        }
        MotionEvent.ACTION_CANCEL -> cancelTouch()
      }
      return true
    }
    override fun performClick(): Boolean { super.performClick(); return true }
    override fun onCheckIsTextEditor() = keyboard
    override fun onCreateInputConnection(info: EditorInfo): InputConnection? {
      if (!keyboard) return null
      info.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
      info.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_ACTION_NONE
      val token = ++imeGeneration
      imeRecovery.clear()
      val session = id
      return object : BaseInputConnection(this,true) {
        private val composing = SpannableStringBuilder()
        private var closed = false
        private fun current() = !closed && token == imeGeneration && session == id && keyboard && connected && isShown && hasWindowFocus() && hasFocus()
        private val commits = KeyboardCommitSession(keyboardState) { current() }
        override fun getEditable(): Editable = composing
        override fun closeConnection() { closed=true; commits.close(); composing.clear(); super.closeConnection() }
        private fun reject(): Boolean {
          onStatus(mapOf("state" to "connected","inputError" to "Text requires a compatible peer, up to 64 characters, and released physical keys."))
          return false
        }
        override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
          if (!current()) return false
          if (!commits.commit(text?.toString() ?: "")) return reject()
          composing.clear(); return true
        }
        override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
          if (!current()) return false
          if ((text?.length ?: 0) > DisplayKeyboard.MAX_TEXT * 2) return reject()
          composing.clear(); return super.setComposingText(text ?: "",newCursorPosition)
        }
        override fun finishComposingText(): Boolean = if (!current()) false else commitText(composing.toString(),1)
        override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
          if (!current()) return false
          if (beforeLength !in 0..16 || afterLength !in 0..16 || beforeLength+afterLength>16) return reject()
          if (composing.isNotEmpty()) return super.deleteSurroundingText(beforeLength,afterLength)
          return if (keyboardState.delete(beforeLength,afterLength)) true else reject()
        }
        override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean {
          if (!current()) return false
          if (beforeLength !in 0..16 || afterLength !in 0..16 || beforeLength+afterLength>16) return reject()
          if (composing.isNotEmpty()) return super.deleteSurroundingTextInCodePoints(beforeLength,afterLength)
          return deleteSurroundingText(beforeLength,afterLength)
        }
        override fun sendKeyEvent(event: KeyEvent): Boolean = current() && dispatchKeyEvent(event)
      }
    }
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
      if (!connected) return super.dispatchKeyEvent(event)
      val code=DisplayKeyboard.physical(event.keyCode)
      val modifiers=when(event.keyCode) { KeyEvent.KEYCODE_SHIFT_LEFT,KeyEvent.KEYCODE_SHIFT_RIGHT,KeyEvent.KEYCODE_CTRL_LEFT,KeyEvent.KEYCODE_CTRL_RIGHT,KeyEvent.KEYCODE_ALT_LEFT,KeyEvent.KEYCODE_ALT_RIGHT,KeyEvent.KEYCODE_META_LEFT,KeyEvent.KEYCODE_META_RIGHT -> true; else -> false }
      if (code == null && !modifiers) return super.dispatchKeyEvent(event)
      if (event.action !in listOf(KeyEvent.ACTION_DOWN,KeyEvent.ACTION_UP)) return true
      val mask=(if(event.isShiftPressed) 1 else 0) or (if(event.isCtrlPressed) 2 else 0) or (if(event.isAltPressed) 4 else 0) or (if(event.isMetaPressed) 8 else 0)
      return keyboardState.hardware(code,mask,event.action==KeyEvent.ACTION_DOWN,event.repeatCount>0)
    }
  }
}
