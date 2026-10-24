//! API-31 cursor composition with Canvas fallback in an API-29-loadable library.
//! The Java cursor receiver is the sole transaction owner. Handles are opaque;
//! UI retirement never destroys a layer.
//! No SurfaceControl/bitmap API is called on desktop targets or by unit tests.

#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    raster_width: i32,
    raster_height: i32,
    logical_width: i32,
    logical_height: i32,
    holder_width: i32,
    holder_height: i32,
}

impl Geometry {
    fn validate(
        rw: i32,
        rh: i32,
        lw: i32,
        lh: i32,
        hw: i32,
        hh: i32,
        vw: i32,
        vh: i32,
    ) -> Option<Self> {
        if [rw, rh, lw, lh, hw, hh, vw, vh]
            .iter()
            .any(|n| !(1..=8192).contains(n))
            || rw > 4096
            || rh > 4096
            || !((rw == lw && rh == lh) || (rw == lw * 2 && rh == lh * 2))
            || hw != vw
            || hh != vh
        {
            return None;
        }
        // This view uses SurfaceHolder's default size-from-layout contract:
        // holder extent must exactly equal the fitted SurfaceView. The native
        // window reports that default producer extent, not the active decoded
        // buffer's coordinate extent. A child created from its Surface attaches
        // to the BLAST video layer and inherits raster-to-holder scaling. Keep
        // the destination in admitted raster coordinates to avoid double scale.
        // SurfaceView integer fitting may round one edge; reject stretches.
        for (width, height) in [(hw, hh), (vw, vh)] {
            let error =
                (i64::from(width) * i64::from(rh) - i64::from(height) * i64::from(rw)).abs();
            if error > 2 * i64::from(rw.max(rh)) {
                return None;
            }
        }
        Some(Self {
            raster_width: rw,
            raster_height: rh,
            logical_width: lw,
            logical_height: lh,
            holder_width: hw,
            holder_height: hh,
        })
    }
    fn accepts_window(self, width: i32, height: i32) -> bool {
        width == self.holder_width && height == self.holder_height
    }
    fn placement(
        self,
        point: Point,
        shape: Shape,
        bitmap_width: i32,
        bitmap_height: i32,
    ) -> Option<Placement> {
        if !shape.valid()
            || !point.valid()
            || !(1..=512).contains(&bitmap_width)
            || !(1..=512).contains(&bitmap_height)
        {
            return None;
        }
        let scale = f64::from(self.raster_width) / f64::from(self.logical_width);
        let left = point.x * f64::from(self.raster_width) - shape.hotspot_x * scale;
        let top = point.y * f64::from(self.raster_height) - shape.hotspot_y * scale;
        let result = Placement {
            left: left.round() as i32,
            top: top.round() as i32,
            scale_x: (shape.width * scale / f64::from(bitmap_width)) as f32,
            scale_y: (shape.height * scale / f64::from(bitmap_height)) as f32,
        };
        (result.scale_x.is_finite()
            && result.scale_y.is_finite()
            && result.scale_x > 0.
            && result.scale_y > 0.)
            .then_some(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Shape {
    revision: u64,
    width: f64,
    height: f64,
    hotspot_x: f64,
    hotspot_y: f64,
}
impl Shape {
    fn valid(self) -> bool {
        [self.width, self.height, self.hotspot_x, self.hotspot_y]
            .iter()
            .all(|n| n.is_finite())
            && self.width > 0.
            && self.width <= 512.
            && self.height > 0.
            && self.height <= 512.
            && self.hotspot_x >= 0.
            && self.hotspot_x <= self.width
            && self.hotspot_y >= 0.
            && self.hotspot_y <= self.height
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    revision: u64,
    sequence: u64,
    x: f64,
    y: f64,
    visible: bool,
    received_ms: i64,
}
impl Point {
    fn valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && (0. ..=1.).contains(&self.x)
            && (0. ..=1.).contains(&self.y)
            && self.received_ms >= 0
    }
    fn visible(self, shape: Shape, now_ms: i64) -> bool {
        self.visible
            && self.revision == shape.revision
            && now_ms >= self.received_ms
            && now_ms - self.received_ms < 2000
    }
}
#[repr(C)]
#[cfg(target_os = "android")]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    left: i32,
    top: i32,
    scale_x: f32,
    scale_y: f32,
}

#[derive(Default)]
struct State {
    shape: Option<Shape>,
    point: Option<Point>,
}
impl State {
    fn admit(&mut self, shape: Shape, point: Point) -> bool {
        if !shape.valid()
            || !point.valid()
            || self.shape.is_some_and(|old| {
                shape.revision < old.revision || (shape.revision == old.revision && shape != old)
            })
            || self.point.is_some_and(|old| {
                point.sequence < old.sequence || (point.sequence == old.sequence && point != old)
            })
        {
            return false;
        }
        self.shape = Some(shape);
        self.point = Some(point);
        true
    }
}

// Retirement and the final visible apply share only this small gate. Bitmap
// allocation/upload and media-state reads never hold it. UI retirement may wait
// for one in-flight platform apply, but never performs native destruction.
#[derive(Default)]
struct Retirement {
    retired: std::sync::atomic::AtomicBool,
    apply: std::sync::Mutex<()>,
}
impl Retirement {
    fn retire(&self) {
        let _gate = self.apply.lock().unwrap_or_else(|e| e.into_inner());
        self.retired
            .store(true, std::sync::atomic::Ordering::Release);
    }
    fn is_retired(&self) -> bool {
        self.retired.load(std::sync::atomic::Ordering::Acquire)
    }
    fn submit<T>(&self, operation: impl FnOnce() -> T) -> Option<T> {
        let _gate = self.apply.lock().unwrap_or_else(|e| e.into_inner());
        (!self.is_retired()).then(operation)
    }
}

fn admitted(state: &serde_json::Value, id: &str, revision: u64, geometry: Geometry) -> bool {
    let config = &state["config"];
    let scale = if config["retina"].as_bool() == Some(true) {
        2
    } else {
        1
    };
    state["id"].as_str() == Some(id) && state["revision"].as_u64() == Some(revision)
        && state["state"].as_str() == Some("streaming")
        // This field counts admitted frames in the current revision; it is not
        // a copy of the configuration revision number.
        && state["rendered_revision"].as_u64().unwrap_or(0) > 0
        && config["separate_cursor"].as_bool() == Some(true)
        && config["retina"].is_boolean()
        && config["width"].as_i64() == Some(i64::from(geometry.logical_width))
        && config["height"].as_i64() == Some(i64::from(geometry.logical_height))
        && geometry.raster_width == geometry.logical_width * scale
        && geometry.raster_height == geometry.logical_height * scale
}

#[cfg(target_os = "android")]
mod android {
    use super::*;
    use anyhow::{bail, ensure, Context, Result};
    use jni::{
        objects::{JClass, JObject, JString},
        sys::{jboolean, jdouble, jint, jlong, jobject},
        JNIEnv,
    };
    use std::{
        collections::HashMap,
        ffi::{c_char, c_void, CString},
        ptr,
        sync::{
            atomic::{AtomicU64, AtomicUsize, Ordering},
            Arc, Mutex, OnceLock,
        },
        thread::{self, ThreadId},
        time::Instant,
    };

    enum Window {}
    enum SurfaceControl {}
    enum Transaction {}
    enum HardwareBuffer {}
    enum TransactionStats {}
    #[repr(C)]
    #[derive(Default)]
    struct BufferDesc {
        width: u32,
        height: u32,
        layers: u32,
        format: u32,
        usage: u64,
        stride: u32,
        rfu0: u32,
        rfu1: u64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct BitmapInfo {
        width: u32,
        height: u32,
        stride: u32,
        format: i32,
        flags: u32,
    }
    const WRITE_RARELY: u64 = 2 << 4;
    const BUFFER_USAGE: u64 = WRITE_RARELY | (1 << 8) | (1 << 11);
    const SRGB: i32 = 142671872;

    #[link(name = "android")]
    extern "C" {
        fn android_get_device_api_level() -> i32;
        fn ANativeWindow_fromSurface(env: *mut jni::sys::JNIEnv, surface: jobject) -> *mut Window;
        fn ANativeWindow_release(window: *mut Window);
        fn ANativeWindow_getWidth(window: *mut Window) -> i32;
        fn ANativeWindow_getHeight(window: *mut Window) -> i32;
        fn ASurfaceControl_createFromWindow(
            window: *mut Window,
            name: *const c_char,
        ) -> *mut SurfaceControl;
        fn ASurfaceControl_release(control: *mut SurfaceControl);
        fn ASurfaceTransaction_create() -> *mut Transaction;
        fn ASurfaceTransaction_delete(transaction: *mut Transaction);
        fn ASurfaceTransaction_apply(transaction: *mut Transaction);
        // The public enum has int8_t ABI, not an int32_t parameter.
        fn ASurfaceTransaction_setVisibility(
            transaction: *mut Transaction,
            control: *mut SurfaceControl,
            visibility: i8,
        );
        fn ASurfaceTransaction_setZOrder(
            transaction: *mut Transaction,
            control: *mut SurfaceControl,
            layer: i32,
        );
        fn ASurfaceTransaction_reparent(
            transaction: *mut Transaction,
            control: *mut SurfaceControl,
            parent: *mut SurfaceControl,
        );
        fn ASurfaceTransaction_setBuffer(
            transaction: *mut Transaction,
            control: *mut SurfaceControl,
            buffer: *mut HardwareBuffer,
            fence: i32,
        );
        fn ASurfaceTransaction_setBufferTransparency(
            transaction: *mut Transaction,
            control: *mut SurfaceControl,
            transparency: i8,
        );
        fn ASurfaceTransaction_setBufferDataSpace(
            transaction: *mut Transaction,
            control: *mut SurfaceControl,
            dataspace: i32,
        );
        fn ASurfaceTransaction_setOnComplete(
            transaction: *mut Transaction,
            context: *mut c_void,
            callback: unsafe extern "C" fn(*mut c_void, *mut TransactionStats),
        );
        fn ASurfaceTransactionStats_getLatchTime(stats: *mut TransactionStats) -> i64;
        fn ASurfaceTransactionStats_getPresentFenceFd(stats: *mut TransactionStats) -> i32;
        fn AHardwareBuffer_isSupported(desc: *const BufferDesc) -> i32;
        fn AHardwareBuffer_allocate(
            desc: *const BufferDesc,
            buffer: *mut *mut HardwareBuffer,
        ) -> i32;
        fn AHardwareBuffer_release(buffer: *mut HardwareBuffer);
        fn AHardwareBuffer_lockAndGetInfo(
            buffer: *mut HardwareBuffer,
            usage: u64,
            fence: i32,
            rect: *const Rect,
            pixels: *mut *mut c_void,
            pixel_bytes: *mut i32,
            row_bytes: *mut i32,
        ) -> i32;
        fn AHardwareBuffer_unlock(buffer: *mut HardwareBuffer, fence: *mut i32) -> i32;
    }
    #[link(name = "jnigraphics")]
    extern "C" {
        fn AndroidBitmap_getInfo(
            env: *mut jni::sys::JNIEnv,
            bitmap: jobject,
            info: *mut BitmapInfo,
        ) -> i32;
        fn AndroidBitmap_lockPixels(
            env: *mut jni::sys::JNIEnv,
            bitmap: jobject,
            pixels: *mut *mut c_void,
        ) -> i32;
        fn AndroidBitmap_unlockPixels(env: *mut jni::sys::JNIEnv, bitmap: jobject) -> i32;
    }
    #[link(name = "log")]
    extern "C" {
        fn __android_log_write(priority: i32, tag: *const c_char, message: *const c_char) -> i32;
    }
    extern "C" {
        fn close(fd: i32) -> i32;
    }
    #[link(name = "dl")]
    extern "C" {
        fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
        fn dlsym(library: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(library: *mut c_void) -> i32;
    }
    type SetPosition = unsafe extern "C" fn(*mut Transaction, *mut SurfaceControl, i32, i32);
    type SetScale = unsafe extern "C" fn(*mut Transaction, *mut SurfaceControl, f32, f32);
    struct PositionScaleApi {
        _library: usize,
        position: SetPosition,
        scale: SetScale,
    }
    fn position_scale_api() -> Option<&'static PositionScaleApi> {
        static API: OnceLock<Option<PositionScaleApi>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            // No API-31 strong symbol references: the shared library must still
            // load on minSdk 29. Below 31 no lookup or native layer is attempted.
            if android_get_device_api_level() < 31 {
                return None;
            }
            let library = dlopen(b"libandroid.so\0".as_ptr().cast(), 2); // RTLD_NOW | RTLD_LOCAL
            if library.is_null() {
                return None;
            }
            let position = dlsym(
                library,
                b"ASurfaceTransaction_setPosition\0".as_ptr().cast(),
            );
            let scale = dlsym(library, b"ASurfaceTransaction_setScale\0".as_ptr().cast());
            if position.is_null() || scale.is_null() {
                dlclose(library);
                return None;
            }
            // One process-lifetime reference protects these function pointers;
            // it is never multiplied by cursor generations or callbacks.
            Some(PositionScaleApi {
                _library: library as usize,
                position: std::mem::transmute::<*mut c_void, SetPosition>(position),
                scale: std::mem::transmute::<*mut c_void, SetScale>(scale),
            })
        })
        .as_ref()
    }

    struct OwnedBuffer {
        pointer: *mut HardwareBuffer,
        fence: i32,
        width: i32,
        height: i32,
    }
    impl Drop for OwnedBuffer {
        fn drop(&mut self) {
            unsafe {
                if self.fence >= 0 {
                    close(self.fence);
                    self.fence = -1;
                }
                if !self.pointer.is_null() {
                    AHardwareBuffer_release(self.pointer);
                }
            }
        }
    }
    // FFI pointers may be stored in the registry, but every operation and final
    // destruction checks the originating Java cursor receiver's OS thread.
    unsafe impl Send for OwnedBuffer {}

    unsafe fn upload(env: &mut JNIEnv, bitmap: &JObject) -> Result<OwnedBuffer> {
        ensure!(
            env.call_method(bitmap, "isPremultiplied", "()Z", &[])?
                .z()?,
            "Cursor bitmap is not premultiplied"
        );
        ensure!(
            !env.call_method(bitmap, "isRecycled", "()Z", &[])?.z()?,
            "Cursor bitmap was recycled"
        );
        let mut info = BitmapInfo::default();
        ensure!(
            AndroidBitmap_getInfo(env.get_native_interface(), bitmap.as_raw(), &mut info) == 0
                && info.format == 1
                && (1..=512).contains(&info.width)
                && (1..=512).contains(&info.height)
                && info.stride >= info.width * 4,
            "Unsupported cursor bitmap"
        );
        // Do not read BitmapInfo.flags: its alpha bits require API 30.
        let desc = BufferDesc {
            width: info.width,
            height: info.height,
            layers: 1,
            format: 1,
            usage: BUFFER_USAGE,
            ..Default::default()
        };
        ensure!(
            AHardwareBuffer_isSupported(&desc) != 0,
            "Cursor hardware buffer is unsupported"
        );
        let mut buffer = OwnedBuffer {
            pointer: ptr::null_mut(),
            fence: -1,
            width: info.width as i32,
            height: info.height as i32,
        };
        ensure!(
            AHardwareBuffer_allocate(&desc, &mut buffer.pointer) == 0 && !buffer.pointer.is_null(),
            "Could not allocate cursor buffer"
        );
        let mut source = ptr::null_mut();
        ensure!(
            AndroidBitmap_lockPixels(env.get_native_interface(), bitmap.as_raw(), &mut source) == 0,
            "Could not lock cursor bitmap"
        );
        let mut destination = ptr::null_mut();
        let mut pixel_bytes = 0;
        let mut row_bytes = 0;
        let locked = AHardwareBuffer_lockAndGetInfo(
            buffer.pointer,
            WRITE_RARELY,
            -1,
            ptr::null(),
            &mut destination,
            &mut pixel_bytes,
            &mut row_bytes,
        ) == 0;
        let valid = locked
            && !source.is_null()
            && !destination.is_null()
            && pixel_bytes == 4
            && row_bytes >= (info.width * 4) as i32;
        if valid {
            for row in 0..info.height as usize {
                ptr::copy_nonoverlapping(
                    (source as *const u8).add(row * info.stride as usize),
                    (destination as *mut u8).add(row * row_bytes as usize),
                    info.width as usize * 4,
                );
            }
        }
        let unlocked = !locked || AHardwareBuffer_unlock(buffer.pointer, &mut buffer.fence) == 0;
        let bitmap_unlocked =
            AndroidBitmap_unlockPixels(env.get_native_interface(), bitmap.as_raw()) == 0;
        ensure!(
            valid && unlocked && bitmap_unlocked,
            "Could not upload cursor bitmap"
        );
        Ok(buffer)
    }

    static NEXT: AtomicU64 = AtomicU64::new(1);
    static HANDLES: OnceLock<Mutex<HashMap<u64, Arc<Entry>>>> = OnceLock::new();
    static PENDING_COMPLETIONS: AtomicUsize = AtomicUsize::new(0);
    fn handles() -> &'static Mutex<HashMap<u64, Arc<Entry>>> {
        HANDLES.get_or_init(|| Mutex::new(HashMap::new()))
    }
    struct Entry {
        id: String,
        epoch: u64,
        revision: u64,
        owner: ThreadId,
        retirement: Retirement,
        renderer: Mutex<Renderer>,
    }
    struct Renderer {
        layer: *mut SurfaceControl,
        geometry: Geometry,
        state: State,
        buffer: Option<OwnedBuffer>,
        displayed: Option<(u64, u64, Placement)>,
        buffer_revision: Option<u64>,
        trace: bool,
        samples: u64,
        origin: Instant,
    }
    unsafe impl Send for Renderer {}
    impl Renderer {
        unsafe fn hide(&mut self) {
            if self.displayed.take().is_none() {
                return;
            }
            let tx = ASurfaceTransaction_create();
            ASurfaceTransaction_setVisibility(tx, self.layer, 0);
            ASurfaceTransaction_apply(tx);
            ASurfaceTransaction_delete(tx);
        }
        unsafe fn retire(&mut self) {
            if self.layer.is_null() {
                return;
            }
            let tx = ASurfaceTransaction_create();
            ASurfaceTransaction_setVisibility(tx, self.layer, 0);
            ASurfaceTransaction_reparent(tx, self.layer, ptr::null_mut());
            ASurfaceTransaction_apply(tx);
            ASurfaceTransaction_delete(tx);
            ASurfaceControl_release(self.layer);
            self.layer = ptr::null_mut();
            self.buffer = None;
            self.displayed = None;
        }
    }
    // Destruction is deliberately explicit on the owner thread; dropping an
    // Entry from UI retirement must never issue a SurfaceControl transaction.
    fn entry(handle: jlong) -> Result<Arc<Entry>> {
        ensure!(handle > 0, "Invalid cursor handle");
        handles()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(handle as u64))
            .cloned()
            .context("Cursor renderer retired")
    }
    fn current(id: &str, revision: u64, geometry: Geometry) -> bool {
        let Ok(server) = crate::media_server() else {
            return false;
        };
        admitted(&server.snapshot(), id, revision, geometry)
    }
    fn log(message: String) {
        if let Ok(message) = CString::new(message) {
            unsafe {
                __android_log_write(3, b"ExtendCursorNative\0".as_ptr().cast(), message.as_ptr());
            }
        }
    }
    struct Completion {
        origin: Instant,
        submitted: u64,
        epoch: u64,
        revision: u64,
        sequence: u64,
    }
    unsafe extern "C" fn complete(context: *mut c_void, stats: *mut TransactionStats) {
        let sample = Box::from_raw(context.cast::<Completion>());
        let completed = sample.origin.elapsed().as_nanos() as u64;
        let latch = ASurfaceTransactionStats_getLatchTime(stats);
        let fence = ASurfaceTransactionStats_getPresentFenceFd(stats);
        if fence >= 0 {
            close(fence);
        }
        PENDING_COMPLETIONS.fetch_sub(1, Ordering::AcqRel);
        log(format!("clock=rust-instant-elapsed-ns epoch={} revision={} sequence={} stage=transaction-complete submit_ns={} complete_ns={} submit_to_complete_us={} latch_ns={} latch_clock=ndk-latch-domain present_fence_available={} physical_visibility_proven=false",
            sample.epoch, sample.revision, sample.sequence, sample.submitted, completed,
            completed.saturating_sub(sample.submitted) / 1000, latch, fence >= 0));
    }
    fn register_completion(
        tx: *mut Transaction,
        renderer: &mut Renderer,
        binding: &Entry,
        sequence: u64,
    ) {
        if !renderer.trace {
            return;
        }
        renderer.samples += 1;
        if renderer.samples > 600 || !(renderer.samples <= 8 || renderer.samples % 32 == 0) {
            return;
        }
        if PENDING_COMPLETIONS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 8).then_some(n + 1)
            })
            .is_err()
        {
            return;
        }
        let submitted = renderer.origin.elapsed().as_nanos() as u64;
        let sample = Box::new(Completion {
            origin: renderer.origin,
            submitted,
            epoch: binding.epoch,
            revision: binding.revision,
            sequence,
        });
        unsafe {
            ASurfaceTransaction_setOnComplete(tx, Box::into_raw(sample).cast(), complete);
        }
        log(format!("clock=rust-instant-elapsed-ns epoch={} revision={} sequence={} stage=transaction-submit submit_ns={} physical_visibility_proven=false",
            binding.epoch, binding.revision, sequence, submitted));
    }

    #[no_mangle]
    pub extern "system" fn Java_computer_extend_android_NativeEngine_createCursorCompositor(
        mut env: JNIEnv,
        _: JClass,
        id: JString,
        surface: JObject,
        epoch: jlong,
        revision: jlong,
        rw: jint,
        rh: jint,
        lw: jint,
        lh: jint,
        hw: jint,
        hh: jint,
        vw: jint,
        vh: jint,
        trace: jboolean,
    ) -> jlong {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<jlong> {
            ensure!(epoch > 0 && revision >= 0, "Invalid cursor binding");
            ensure!(
                position_scale_api().is_some(),
                "Cursor position/scale requires API 31 and public native symbols"
            );
            let geometry = Geometry::validate(rw, rh, lw, lh, hw, hh, vw, vh)
                .context("Uncertain cursor parent geometry")?;
            let id: String = env.get_string(&id)?.into();
            ensure!(
                current(&id, revision as u64, geometry),
                "Display geometry is not admitted"
            );
            ensure!(
                handles().lock().unwrap_or_else(|e| e.into_inner()).len() < 8,
                "Cursor renderer limit reached"
            );
            unsafe {
                let window =
                    ANativeWindow_fromSurface(env.get_native_interface(), surface.as_raw());
                ensure!(!window.is_null(), "Cursor parent is unavailable");
                let dimensions = (
                    ANativeWindow_getWidth(window),
                    ANativeWindow_getHeight(window),
                );
                if !geometry.accepts_window(dimensions.0, dimensions.1) {
                    ANativeWindow_release(window);
                    bail!(
                        "Cursor window default extent {}x{} differs from layout holder {}x{}",
                        dimensions.0,
                        dimensions.1,
                        hw,
                        hh
                    );
                }
                let layer =
                    ASurfaceControl_createFromWindow(window, b"extend-cursor\0".as_ptr().cast());
                ANativeWindow_release(window);
                ensure!(!layer.is_null(), "Native cursor composition is unavailable");
                let tx = ASurfaceTransaction_create();
                ASurfaceTransaction_setVisibility(tx, layer, 0);
                ASurfaceTransaction_setZOrder(tx, layer, 1);
                ASurfaceTransaction_setBufferTransparency(tx, layer, 1);
                ASurfaceTransaction_setBufferDataSpace(tx, layer, SRGB);
                ASurfaceTransaction_apply(tx);
                ASurfaceTransaction_delete(tx);
                let handle = NEXT.fetch_add(1, Ordering::Relaxed);
                if handle == 0 || handle > i64::MAX as u64 {
                    let mut retired = Renderer {
                        layer,
                        geometry,
                        state: State::default(),
                        buffer: None,
                        displayed: None,
                        buffer_revision: None,
                        trace: false,
                        samples: 0,
                        origin: Instant::now(),
                    };
                    retired.retire();
                    bail!("Cursor handle space exhausted");
                }
                let mut renderer = Renderer {
                    layer,
                    geometry,
                    state: State::default(),
                    buffer: None,
                    displayed: None,
                    buffer_revision: None,
                    trace: trace != 0,
                    samples: 0,
                    origin: Instant::now(),
                };
                let mut registry = handles().lock().unwrap_or_else(|e| e.into_inner());
                if registry.len() >= 8 {
                    drop(registry);
                    renderer.retire();
                    bail!("Cursor renderer limit reached");
                }
                registry.insert(
                    handle,
                    Arc::new(Entry {
                        id,
                        epoch: epoch as u64,
                        revision: revision as u64,
                        owner: thread::current().id(),
                        retirement: Retirement::default(),
                        renderer: Mutex::new(renderer),
                    }),
                );
                Ok(handle as jlong)
            }
        }));
        match result {
            Ok(Ok(handle)) => handle,
            Ok(Err(error)) => {
                log(format!("stage=backend-rejected reason={error}"));
                0
            }
            Err(_) => {
                log("stage=backend-rejected reason=Native cursor initialization failed".into());
                0
            }
        }
    }

    // 0: accepted/hidden; 1: suspended stale binding; -1: backend failed.
    #[no_mangle]
    pub extern "system" fn Java_computer_extend_android_NativeEngine_updateCursorCompositor(
        mut env: JNIEnv,
        _: JClass,
        handle: jlong,
        epoch: jlong,
        revision: jlong,
        bitmap: JObject,
        shape_revision: jlong,
        width: jdouble,
        height: jdouble,
        hotspot_x: jdouble,
        hotspot_y: jdouble,
        point_revision: jlong,
        sequence: jlong,
        x: jdouble,
        y: jdouble,
        visible: jboolean,
        received_ms: jlong,
        now_ms: jlong,
    ) -> jint {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<jint> {
            ensure!(
                [
                    epoch,
                    revision,
                    shape_revision,
                    point_revision,
                    sequence,
                    received_ms,
                    now_ms
                ]
                .iter()
                .all(|n| *n >= 0),
                "Invalid cursor state"
            );
            let binding = entry(handle)?;
            ensure!(
                binding.owner == thread::current().id(),
                "Cursor renderer thread changed"
            );
            let mut renderer = binding.renderer.lock().unwrap_or_else(|e| e.into_inner());
            if binding.retirement.is_retired()
                || epoch as u64 != binding.epoch
                || revision as u64 != binding.revision
                || !current(&binding.id, binding.revision, renderer.geometry)
            {
                unsafe {
                    renderer.hide();
                }
                return Ok(1);
            }
            let shape = Shape {
                revision: shape_revision as u64,
                width,
                height,
                hotspot_x,
                hotspot_y,
            };
            let point = Point {
                revision: point_revision as u64,
                sequence: sequence as u64,
                x,
                y,
                visible: visible != 0,
                received_ms,
            };
            ensure!(
                renderer.state.admit(shape, point),
                "Stale or altered cursor state"
            );
            if !point.visible(shape, now_ms) {
                unsafe {
                    renderer.hide();
                }
                return Ok(0);
            }
            let replace = renderer.buffer_revision != Some(shape.revision);
            let mut next = if replace {
                Some(unsafe { upload(&mut env, &bitmap)? })
            } else {
                None
            };
            let buffer = next
                .as_ref()
                .or(renderer.buffer.as_ref())
                .context("Cursor image is unavailable")?;
            let destination = renderer
                .geometry
                .placement(point, shape, buffer.width, buffer.height)
                .context("Invalid cursor geometry")?;
            let api = position_scale_api().context("Native cursor position/scale unavailable")?;
            if !replace && renderer.displayed == Some((shape.revision, point.sequence, destination))
            {
                return Ok(0);
            }
            if !current(&binding.id, binding.revision, renderer.geometry) {
                unsafe {
                    renderer.hide();
                }
                return Ok(1);
            }
            let applied = binding.retirement.submit(|| unsafe {
                let tx = ASurfaceTransaction_create();
                if let Some(buffer) = next.as_mut() {
                    ASurfaceTransaction_setBuffer(tx, renderer.layer, buffer.pointer, buffer.fence);
                    buffer.fence = -1; // Ownership transferred exactly once.
                }
                // A fresh layer never uses setGeometry/destinationFrame: AOSP
                // clamps its negative edges and it overrides position/matrix.
                // API-31 position preserves the negative logical hotspot;
                // parent clipping and inherited video scaling remain intact.
                (api.position)(tx, renderer.layer, destination.left, destination.top);
                (api.scale)(tx, renderer.layer, destination.scale_x, destination.scale_y);
                ASurfaceTransaction_setVisibility(tx, renderer.layer, 1);
                register_completion(tx, &mut renderer, &binding, point.sequence);
                ASurfaceTransaction_apply(tx);
                ASurfaceTransaction_delete(tx);
            });
            if applied.is_none() {
                return Ok(1);
            }
            if let Some(next) = next {
                renderer.buffer = Some(next);
                renderer.buffer_revision = Some(shape.revision);
            }
            renderer.displayed = Some((shape.revision, point.sequence, destination));
            Ok(0)
        }));
        match result {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => {
                log(format!("stage=backend-failed reason={error}"));
                -1
            }
            Err(_) => {
                log("stage=backend-failed reason=Native cursor update failed".into());
                -1
            }
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_computer_extend_android_NativeEngine_retireCursorCompositor(
        _: JNIEnv,
        _: JClass,
        handle: jlong,
    ) {
        if let Ok(binding) = entry(handle) {
            binding.retirement.retire();
        }
    }
    #[no_mangle]
    pub extern "system" fn Java_computer_extend_android_NativeEngine_closeCursorCompositor(
        _: JNIEnv,
        _: JClass,
        handle: jlong,
    ) {
        if let Ok(binding) = entry(handle) {
            if binding.owner != thread::current().id() {
                return;
            }
            binding.retirement.retire();
            unsafe {
                binding
                    .renderer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retire();
            }
            handles()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&(handle as u64));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shape(revision: u64) -> Shape {
        Shape {
            revision,
            width: 48.,
            height: 32.,
            hotspot_x: 24.,
            hotspot_y: 16.,
        }
    }
    fn point(revision: u64, sequence: u64, x: f64, y: f64) -> Point {
        Point {
            revision,
            sequence,
            x,
            y,
            visible: true,
            received_ms: 100,
        }
    }
    #[test]
    fn geometry_matches_canvas_hotspot_and_keeps_negative_edges() {
        let one = Geometry::validate(1280, 800, 1280, 800, 2560, 1600, 2560, 1600).unwrap();
        let two = Geometry::validate(2560, 1600, 1280, 800, 2560, 1600, 2560, 1600).unwrap();
        assert_eq!(
            one.placement(point(1, 1, 0., 0.), shape(1), 48, 32),
            Some(Placement {
                left: -24,
                top: -16,
                scale_x: 1.,
                scale_y: 1.
            })
        );
        assert_eq!(
            two.placement(point(1, 1, 1., 1.), shape(1), 96, 64),
            Some(Placement {
                left: 2512,
                top: 1568,
                scale_x: 1.,
                scale_y: 1.
            })
        );
        assert_eq!(
            one.placement(point(1, 1, 0.5, 0.5), shape(1), 48, 32),
            Some(Placement {
                left: 616,
                top: 384,
                scale_x: 1.,
                scale_y: 1.
            })
        );
        assert!(Geometry::validate(1280, 800, 1280, 800, 1600, 2560, 2560, 1600).is_none());
        assert!(Geometry::validate(1280, 800, 1000, 800, 1280, 800, 1280, 800).is_none());
        assert!(one
            .placement(point(1, 1, f64::NAN, 0.5), shape(1), 48, 32)
            .is_none());
    }
    #[test]
    fn actual_layout_sized_holder_inherits_video_scaling_without_double_scale() {
        // Physical receipt: decoded BLAST buffer 1138x710, default producer and
        // fitted view 2560x1597. The child inherits 2560/1138 and 1597/710.
        let geometry = Geometry::validate(1138, 710, 1138, 710, 2560, 1597, 2560, 1597).unwrap();
        assert!(geometry.accepts_window(2560, 1597));
        assert!(!geometry.accepts_window(1138, 710));
        assert!(!geometry.accepts_window(1597, 2560));
        assert!(Geometry::validate(1138, 710, 1138, 710, 2560, 1597, 2560, 1598).is_none());
        let center = geometry
            .placement(point(1, 1, 0.5, 0.5), shape(1), 96, 64)
            .unwrap();
        assert_eq!(
            center,
            Placement {
                left: 545,
                top: 339,
                scale_x: 0.5,
                scale_y: 0.5
            }
        );
        for (x, y) in [(0., 0.), (0.5, 0.5), (1., 1.), (0.371, 0.427)] {
            let cursor = point(1, 1, x, y);
            let image = shape(1);
            let placement = geometry.placement(cursor, image, 96, 64).unwrap();
            let inherited_x = 2560. / 1138.;
            let inherited_y = 1597. / 710.;
            let canvas_scale = f64::min(inherited_x, inherited_y);
            let canvas_left = (2560. - 1138. * canvas_scale) / 2. + x * 1138. * canvas_scale
                - image.hotspot_x * canvas_scale;
            let canvas_top = (1597. - 710. * canvas_scale) / 2. + y * 710. * canvas_scale
                - image.hotspot_y * canvas_scale;
            // Includes integer API position rounding plus subpixel centering
            // difference caused by SurfaceView's integer-fitted aspect.
            assert!((f64::from(placement.left) * inherited_x - canvas_left).abs() < 2.);
            assert!((f64::from(placement.top) * inherited_y - canvas_top).abs() < 2.);
            assert!(
                (f64::from(placement.scale_x) * 96. * inherited_x - image.width * canvas_scale)
                    .abs()
                    < 0.1
            );
        }
        let corner = geometry
            .placement(point(1, 1, 0., 0.), shape(1), 96, 64)
            .unwrap();
        assert!(corner.left < 0 && corner.top < 0); // API31 position keeps these.
    }
    #[test]
    fn fractional_shape_scale_depends_on_image_pixels_not_position_rounding() {
        let geometry = Geometry::validate(1138, 710, 569, 355, 2560, 1597, 2560, 1597).unwrap();
        let image = Shape {
            revision: 1,
            width: 15.25,
            height: 23.75,
            hotspot_x: 7.625,
            hotspot_y: 13.375,
        };
        let first = geometry
            .placement(point(1, 1, 0.371, 0.427), image, 30, 48)
            .unwrap();
        let next = geometry
            .placement(point(1, 2, 0.372, 0.428), image, 30, 48)
            .unwrap();
        assert!((first.scale_x - (30.5f32 / 30.)).abs() < 0.000001);
        assert!((first.scale_y - (47.5f32 / 48.)).abs() < 0.000001);
        assert_eq!((first.scale_x, first.scale_y), (next.scale_x, next.scale_y));
        assert_ne!((first.left, first.top), (next.left, next.top));
        assert!(geometry
            .placement(point(1, 1, 0., 0.), image, 0, 48)
            .is_none());
    }
    #[test]
    fn sequence_revision_replay_expiry_and_stationary_shape_are_bounded() {
        let mut state = State::default();
        let first = point(1, 10, 0.5, 0.5);
        assert!(state.admit(shape(1), first));
        assert!(state.admit(shape(1), first));
        assert!(!state.admit(shape(1), point(1, 9, 0.5, 0.5)));
        assert!(!state.admit(shape(1), point(1, 10, 0.6, 0.5)));
        assert!(state.admit(shape(2), point(1, 11, 0.5, 0.5)));
        assert!(!state.point.unwrap().visible(shape(2), 100));
        assert!(state.admit(shape(2), point(2, 12, 0.5, 0.5)));
        assert!(state.point.unwrap().visible(shape(2), 2099));
        assert!(!state.point.unwrap().visible(shape(2), 2100));
        assert!(!state.admit(shape(1), point(2, 13, 0.5, 0.5)));
        let mut changed = shape(2);
        changed.hotspot_x = 4.;
        assert!(!state.admit(changed, point(2, 13, 0.5, 0.5)));
    }
    #[test]
    fn admission_uses_current_frame_count_and_exact_session_profile() {
        let geometry = Geometry::validate(1280, 800, 1280, 800, 2560, 1600, 2560, 1600).unwrap();
        let mut snapshot = serde_json::json!({"id":"current", "state":"streaming", "revision":7,
            "rendered_revision":3360, "config":{"width":1280,"height":800,"retina":false,"separate_cursor":true}});
        assert!(admitted(&snapshot, "current", 7, geometry));
        assert!(!admitted(&snapshot, "previous", 7, geometry));
        assert!(!admitted(&snapshot, "current", 6, geometry));
        snapshot["rendered_revision"] = 0.into();
        assert!(!admitted(&snapshot, "current", 7, geometry));
        snapshot["rendered_revision"] = 1.into();
        snapshot["state"] = "configuring".into();
        assert!(!admitted(&snapshot, "current", 7, geometry));
        snapshot["state"] = "streaming".into();
        snapshot["config"]["retina"] = true.into();
        assert!(!admitted(&snapshot, "current", 7, geometry));
        let retina = Geometry::validate(2560, 1600, 1280, 800, 2560, 1600, 2560, 1600).unwrap();
        assert!(admitted(&snapshot, "current", 7, retina));
        snapshot["config"]["separate_cursor"] = false.into();
        assert!(!admitted(&snapshot, "current", 7, retina));
    }
    #[test]
    fn retirement_waits_for_inflight_apply_then_rejects_every_later_apply() {
        use std::sync::{mpsc, Arc};
        let retirement = Arc::new(Retirement::default());
        let (started_tx, started_rx) = mpsc::channel();
        let (finish_tx, finish_rx) = mpsc::channel();
        let writer = retirement.clone();
        let submit = std::thread::spawn(move || {
            writer.submit(|| {
                started_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                "applied"
            })
        });
        started_rx.recv().unwrap();
        let retired = retirement.clone();
        let (retire_started_tx, retire_started_rx) = mpsc::channel();
        let (retire_done_tx, retire_done_rx) = mpsc::channel();
        let stop = std::thread::spawn(move || {
            retire_started_tx.send(()).unwrap();
            retired.retire();
            retire_done_tx.send(()).unwrap();
        });
        retire_started_rx.recv().unwrap();
        assert!(retire_done_rx.try_recv().is_err());
        finish_tx.send(()).unwrap();
        assert_eq!(submit.join().unwrap(), Some("applied"));
        retire_done_rx.recv().unwrap();
        stop.join().unwrap();
        assert!(retirement.is_retired());
        assert_eq!(
            retirement.submit(|| panic!("Retired cursor was made visible")),
            None::<()>
        );
        retirement.retire(); // Idempotent repeated stop, including UI + worker close.
    }
}
