#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <objc/message.h>
#include <math.h>
#import "VirtualDisplay.h"

// macOS currently exposes no public virtual-monitor API. Keep this small,
// runtime-checked adapter separate from public capture and codec APIs.
static id SendObject(id object, NSString *selector, id argument) {
    return ((id (*)(id, SEL, id))objc_msgSend)(object, NSSelectorFromString(selector), argument);
}
// Retain topology separately from the OS object: mode changes can restore a
// persisted mirror group and turn the virtual monitor into its master.
@interface ECDisplayHandle : NSObject
@property(nonatomic, strong) id display;
@property(nonatomic) CGPoint origin;
@property(nonatomic) CGDirectDisplayID mainID;
@end
@implementation ECDisplayHandle
@end

#define EC_TOPOLOGY_LIMIT 64
typedef struct {
    CGDirectDisplayID id, mirrors;
    bool active;
    CGPoint origin;
} ECTopologyRow;
typedef struct { ECTopologyRow rows[EC_TOPOLOGY_LIMIT]; uint32_t count; CGDirectDisplayID mainID; } ECTopology;
typedef struct {
    bool (*read)(void *, ECTopology *);
    bool (*repair)(void *, CGDirectDisplayID, const CGDirectDisplayID *, uint32_t, CGPoint);
    void *context;
} ECTopologyBackend;

static bool ECReadTopology(void *context, ECTopology *snapshot) {
    CGDirectDisplayID ids[EC_TOPOLOGY_LIMIT]; uint32_t count = 0;
    if (CGGetOnlineDisplayList(EC_TOPOLOGY_LIMIT, ids, &count) != kCGErrorSuccess || count >= EC_TOPOLOGY_LIMIT) return false;
    snapshot->count = count;
    snapshot->mainID = CGMainDisplayID();
    for (uint32_t i = 0; i < count; i++) {
        snapshot->rows[i] = (ECTopologyRow){ids[i], CGDisplayMirrorsDisplay(ids[i]),
            CGDisplayIsActive(ids[i]) != 0, CGDisplayBounds(ids[i]).origin};
    }
    return true;
}
static bool ECRepairTopology(void *context, CGDirectDisplayID virtualID,
                             const CGDirectDisplayID *slaves, uint32_t count, CGPoint origin) {
    CGDisplayConfigRef config = NULL;
    if (CGBeginDisplayConfiguration(&config) != kCGErrorSuccess) return false;
    CGError result = CGConfigureDisplayMirrorOfDisplay(config, virtualID, kCGNullDirectDisplay);
    for (uint32_t i = 0; i < count && result == kCGErrorSuccess; i++)
        result = CGConfigureDisplayMirrorOfDisplay(config, slaves[i], kCGNullDirectDisplay);
    if (result == kCGErrorSuccess)
        result = CGConfigureDisplayOrigin(config, virtualID, (int32_t)origin.x, (int32_t)origin.y);
    if (result != kCGErrorSuccess) { CGCancelDisplayConfiguration(config); return false; }
    return CGCompleteDisplayConfiguration(config, kCGConfigureForAppOnly) == kCGErrorSuccess;
}
static const ECTopologyRow *ECFindDisplay(const ECTopology *snapshot, CGDirectDisplayID id) {
    for (uint32_t i = 0; i < snapshot->count; i++)
        if (snapshot->rows[i].id == id) return &snapshot->rows[i];
    return NULL;
}
static uint32_t ECLinkedSlaves(const ECTopology *snapshot, CGDirectDisplayID id, CGDirectDisplayID *slaves) {
    uint32_t count = 0;
    for (uint32_t i = 0; i < snapshot->count; i++)
        if (snapshot->rows[i].id != id && snapshot->rows[i].mirrors == id)
            slaves[count++] = snapshot->rows[i].id;
    return count;
}
static bool ECMainIsActive(const ECTopology *snapshot, CGDirectDisplayID id) {
    const ECTopologyRow *main = ECFindDisplay(snapshot, id);
    return snapshot->mainID == id && main && main->active;
}
static bool ECValidExtension(const ECTopology *snapshot, CGDirectDisplayID id) {
    CGDirectDisplayID slaves[EC_TOPOLOGY_LIMIT];
    const ECTopologyRow *virtual = ECFindDisplay(snapshot, id);
    return virtual && virtual->active && virtual->mirrors == kCGNullDirectDisplay &&
        ECLinkedSlaves(snapshot, id, slaves) == 0;
}
// Called after mode selection. A repaired topology is never acknowledged until
// a fresh inventory confirms it; the caller's bounded mode loop retries.
static bool ECGuardExtension(ECTopologyBackend backend, CGDirectDisplayID id, CGPoint origin, CGDirectDisplayID mainID) {
    ECTopology snapshot;
    if (!backend.read(backend.context, &snapshot)) return false;
    const ECTopologyRow *virtual = ECFindDisplay(&snapshot, id);
    if (!virtual) return false;
    CGDirectDisplayID slaves[EC_TOPOLOGY_LIMIT];
    uint32_t count = ECLinkedSlaves(&snapshot, id, slaves);
    if (virtual->mirrors == kCGNullDirectDisplay && count == 0)
        return virtual->active && ECMainIsActive(&snapshot, mainID); // Healthy topology is a no-op.
    if (!backend.repair(backend.context, id, slaves, count, origin)) return false;
    if (!backend.read(backend.context, &snapshot) || !ECValidExtension(&snapshot, id)) return false;
    // CoreGraphics can snap requested origins to preserve adjacency. The
    // invariant is the prior main role, not exact coordinates after resizing.
    return ECMainIsActive(&snapshot, mainID);
}
static ECTopologyBackend ECSystemTopology(void) {
    return (ECTopologyBackend){ECReadTopology, ECRepairTopology, NULL};
}

bool ECDisplaySupported(void) {
    for (NSString *name in @[@"CGVirtualDisplayDescriptor", @"CGVirtualDisplayMode", @"CGVirtualDisplaySettings", @"CGVirtualDisplay"])
        if (!NSClassFromString(name)) return false;
    return true;
}
// Creation runs on the main actor; the caller reads the reason right after.
static const char *lastFailure;
static void *Fail(const char *reason) {
    lastFailure = reason;
    return NULL;
}
const char *ECDisplayLastFailure(void) { return lastFailure ? lastFailure : "unknown"; }
void *ECDisplayCreate(uint32_t width, uint32_t height, bool retina, uint32_t fps, uint32_t *displayID) {
    lastFailure = NULL;
    if (!ECDisplaySupported()) return Fail("virtual displays unavailable");
    if (!displayID || width < 640 || height < 480 || width > 4096 / (retina ? 2 : 1) || height > 4096 / (retina ? 2 : 1) || fps < 30 || fps > 120) return Fail("unsupported size or frame rate");
    CGRect mainBounds = CGDisplayBounds(CGMainDisplayID());
    CGFloat desktopRight = CGRectGetMaxX(mainBounds);
    CGDirectDisplayID existing[64]; uint32_t existingCount = 0;
    if (CGGetActiveDisplayList(64, existing, &existingCount) == kCGErrorSuccess)
        for (uint32_t i = 0; i < existingCount; i++)
            desktopRight = MAX(desktopRight, CGRectGetMaxX(CGDisplayBounds(existing[i])));
    @try {
        id descriptor = [NSClassFromString(@"CGVirtualDisplayDescriptor") new];
        [descriptor setValue:@"extend.computer" forKey:@"name"];
        [descriptor setValue:@4096 forKey:@"maxPixelsWide"];
        [descriptor setValue:@4096 forKey:@"maxPixelsHigh"];
        // Session-local virtual monitor identity, unrelated to hardware vendors.
        [descriptor setValue:@(0xFFFF) forKey:@"vendorID"];
        [descriptor setValue:@(1) forKey:@"productID"];
        SEL sizeSelector = NSSelectorFromString(@"setSizeInMillimeters:");
        if ([descriptor respondsToSelector:sizeSelector])
            ((void (*)(id, SEL, CGSize))objc_msgSend)(descriptor, sizeSelector, CGSizeMake(width * 0.2646, height * 0.2646));
        [descriptor setValue:@(arc4random_uniform(UINT32_MAX - 1) + 1) forKey:@"serialNum"];
        SEL queue = NSSelectorFromString(@"setQueue:");
        if ([descriptor respondsToSelector:queue])
            ((void (*)(id, SEL, dispatch_queue_t))objc_msgSend)(descriptor, queue, dispatch_get_main_queue());
        SEL modeSelector = NSSelectorFromString(@"initWithWidth:height:refreshRate:");
        id mode = ((id (*)(id, SEL, NSUInteger, NSUInteger, double))objc_msgSend)(
            [NSClassFromString(@"CGVirtualDisplayMode") alloc], modeSelector, width, height, (double)fps);
        if (!mode) return Fail("mode rejected");
        id settings = [NSClassFromString(@"CGVirtualDisplaySettings") new];
        [settings setValue:@(retina) forKey:@"hiDPI"];
        [settings setValue:@[mode] forKey:@"modes"];
        id display = SendObject([NSClassFromString(@"CGVirtualDisplay") alloc], @"initWithDescriptor:", descriptor);
        if (!display) return Fail("display not created");
        BOOL applied = ((BOOL (*)(id, SEL, id))objc_msgSend)(display, NSSelectorFromString(@"applySettings:"), settings);
        uint32_t identifier = ((uint32_t (*)(id, SEL))objc_msgSend)(display, NSSelectorFromString(@"displayID"));
        if (!applied) return Fail("settings not applied");
        if (identifier == 0) return Fail("no display ID assigned");
        // macOS can restore a mirror group for a newly created monitor.
        // This adapter creates an extension; disable only its mirror links.
        CGDisplayConfigRef config = NULL;
        if (CGBeginDisplayConfiguration(&config) != kCGErrorSuccess) return Fail("configuration not started");
        CGError result = CGConfigureDisplayMirrorOfDisplay(config, identifier, kCGNullDirectDisplay);
        CGDirectDisplayID online[64]; uint32_t count = 0;
        if (result == kCGErrorSuccess && CGGetOnlineDisplayList(64, online, &count) == kCGErrorSuccess) {
            for (uint32_t i = 0; i < count && result == kCGErrorSuccess; i++)
                if (online[i] != identifier && CGDisplayMirrorsDisplay(online[i]) == identifier)
                    result = CGConfigureDisplayMirrorOfDisplay(config, online[i], kCGNullDirectDisplay);
        }
        if (result == kCGErrorSuccess)
            result = CGConfigureDisplayOrigin(config, identifier, (int32_t)ceil(desktopRight), (int32_t)mainBounds.origin.y);
        if (result != kCGErrorSuccess) { CGCancelDisplayConfiguration(config); return Fail("placement rejected"); }
        if (CGCompleteDisplayConfiguration(config, kCGConfigureForAppOnly) != kCGErrorSuccess) return Fail("placement not applied");
        *displayID = identifier;
        ECDisplayHandle *handle = [ECDisplayHandle new];
        handle.display = display;
        handle.origin = CGPointMake(ceil(desktopRight), mainBounds.origin.y);
        handle.mainID = CGMainDisplayID();
        return (void *)CFBridgingRetain(handle);
    } @catch (NSException *exception) {
        return Fail("private display API raised an exception");
    }
}
bool ECDisplayConfigure(void *handle, uint32_t width, uint32_t height, bool retina, uint32_t fps) {
    uint32_t scale = retina ? 2 : 1;
    if (!handle || width < 640 || height < 480 || width > 4096 / scale || height > 4096 / scale || fps < 30 || fps > 120) return false;
    @try {
        ECDisplayHandle *retained = (__bridge ECDisplayHandle *)handle;
        id display = retained.display;
        // Learn an origin only from a confirmed unmirrored extension. Never
        // overwrite it with the mirror master's corrupted origin at (0, 0).
        uint32_t identifier = ((uint32_t (*)(id, SEL))objc_msgSend)(display, NSSelectorFromString(@"displayID"));
        ECTopology before;
        if (!ECReadTopology(NULL, &before) || !ECValidExtension(&before, identifier)) return false;
        retained.origin = ECFindDisplay(&before, identifier)->origin;
        retained.mainID = before.mainID;
        SEL modeSelector = NSSelectorFromString(@"initWithWidth:height:refreshRate:");
        id mode = ((id (*)(id, SEL, NSUInteger, NSUInteger, double))objc_msgSend)(
            [NSClassFromString(@"CGVirtualDisplayMode") alloc], modeSelector, width, height, (double)fps);
        if (!mode) return false;
        id settings = [NSClassFromString(@"CGVirtualDisplaySettings") new];
        [settings setValue:@(retina) forKey:@"hiDPI"];
        [settings setValue:@[mode] forKey:@"modes"];
        return ((BOOL (*)(id, SEL, id))objc_msgSend)(display, NSSelectorFromString(@"applySettings:"), settings);
    } @catch (NSException *exception) { return false; }
}
void ECDisplayDestroy(void *display) {
    if (display) { id released = CFBridgingRelease(display); (void)released; }
}

bool ECDisplaySelectMode(void *handle, uint32_t width, uint32_t height, bool retina, uint32_t fps) {
    if (!handle) return false;
    ECDisplayHandle *retained = (__bridge ECDisplayHandle *)handle;
    id display = retained.display;
    CGDirectDisplayID identifier = ((uint32_t (*)(id, SEL))objc_msgSend)(display, NSSelectorFromString(@"displayID"));
    uint32_t scale = retina ? 2 : 1;
    // Applying virtual settings can retain a low-density mode with the same
    // backing size. Select by both logical and physical dimensions explicitly.
    CFArrayRef modes = CGDisplayCopyAllDisplayModes(identifier,
        (__bridge CFDictionaryRef)@{(__bridge NSString *)kCGDisplayShowDuplicateLowResolutionModes: @YES});
    if (!modes) return false;
    CGDisplayModeRef selected = NULL;
    for (CFIndex i = 0; i < CFArrayGetCount(modes); i++) {
        CGDisplayModeRef mode = (CGDisplayModeRef)CFArrayGetValueAtIndex(modes, i);
        double refresh = CGDisplayModeGetRefreshRate(mode);
        if (CGDisplayModeGetWidth(mode) == width && CGDisplayModeGetHeight(mode) == height &&
            CGDisplayModeGetPixelWidth(mode) == width * scale && CGDisplayModeGetPixelHeight(mode) == height * scale &&
            (refresh == 0 || fabs(refresh - fps) < 0.5)) {
            selected = mode;
            break;
        }
    }
    CGError result = selected ? CGDisplaySetDisplayMode(identifier, selected, NULL) : kCGErrorFailure;
    CFRelease(modes);
    bool topology = result == kCGErrorSuccess &&
        ECGuardExtension(ECSystemTopology(), identifier, retained.origin, retained.mainID);
    // Unmirroring itself can change mode inventories: verify the selected
    // dimensions again AFTER the topology transaction, before acknowledging.
    CGDisplayModeRef current = CGDisplayCopyDisplayMode(identifier);
    bool matches = topology && current &&
        CGDisplayModeGetWidth(current) == width && CGDisplayModeGetHeight(current) == height &&
        CGDisplayModeGetPixelWidth(current) == width * scale && CGDisplayModeGetPixelHeight(current) == height * scale;
    if (current) CGDisplayModeRelease(current);
    return matches;
}
