// Isolated integration host for the real updater bridge and standard Sparkle UI.
// This executable is never included in the shipped desktop app.
#import <Cocoa/Cocoa.h>

extern int extend_updates_start(bool (*)(void), bool (*)(void), void (*)(void), void (*)(void));
extern void extend_updates_preferences(bool, bool, const char *);
extern void extend_updates_check(void);
static BOOL sharing, reserved;
static NSDate *postponedAt;
static NSString *directory;
static NSMutableSet<NSString *> *clicked;

static void record(NSString *name) {
    [@"verified\n" writeToFile:[directory stringByAppendingPathComponent:name]
                  atomically:YES encoding:NSUTF8StringEncoding error:nil];
    NSLog(@"Verification: %@", name);
}
static bool busy(void) { return sharing; }
static bool prepare(void) {
    if (sharing) {
        if (!postponedAt) { postponedAt = [NSDate date]; record(@"deferred.marker"); }
        return false;
    }
    reserved = YES; record(@"reserved.marker"); return true;
}
static void cleanup(void) {
    if (!reserved || sharing) { record(@"unsafe-install.marker"); exit(3); }
    record(@"cleanup.marker");
}
static void cancel(void) {
    reserved = NO; record(@"aborted.marker");
    if (getenv("EXTEND_UPDATE_TEST_EXPECT_ERROR")) exit(0);
}
static void inspect(NSView *view, NSWindow *window) {
    if (getenv("EXTEND_UPDATE_TEST_EXPECT_ERROR") && [view isKindOfClass:[NSTextField class]]) {
        NSString *text = ((NSTextField *)view).stringValue;
        if (text.length && ![clicked containsObject:text]) { [clicked addObject:text]; NSLog(@"Verification text: %@", text); }
    }
    if ([view isKindOfClass:[NSButton class]]) {
        NSButton *button = (NSButton *)view;
        NSString *title = button.title;
        if (getenv("EXTEND_UPDATE_TEST_EXPECT_ERROR") && ![clicked containsObject:title]) {
            [clicked addObject:title]; NSLog(@"Verification button: %@", title);
        }
        if (getenv("EXTEND_UPDATE_TEST_EXPECT_ERROR") && button.enabled &&
            ([title isEqualToString:@"OK"] || [title isEqualToString:@"Cancel Update"])) {
            record(@"error-dialog.marker");
            [button performClick:nil];
            return;
        }
        if (button.enabled && ![clicked containsObject:title] &&
            ([title isEqualToString:@"Install Update"] || [title isEqualToString:@"Install and Relaunch"])) {
            [clicked addObject:title];
            if ([title isEqualToString:@"Install Update"]) {
                NSView *content = window.contentView;
                NSBitmapImageRep *bitmap = [content bitmapImageRepForCachingDisplayInRect:content.bounds];
                [content cacheDisplayInRect:content.bounds toBitmapImageRep:bitmap];
                [[bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}]
                    writeToFile:[directory stringByAppendingPathComponent:@"native-update-dialog.png"] atomically:YES];
                sharing = YES;
            }
            NSLog(@"Verification: clicking %@", title);
            [button performClick:nil];
        }
    }
    for (NSView *child in view.subviews) inspect(child, window);
}
int main(void) {
    @autoreleasepool {
        directory = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"ExtendVerificationDirectory"] ?:
            [NSString stringWithUTF8String:getenv("EXTEND_UPDATE_TEST_DIRECTORY") ?: "/tmp/extend-updater-verification"];
        [[NSFileManager defaultManager] createDirectoryAtPath:directory withIntermediateDirectories:YES attributes:nil error:nil];
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
        NSMenu *menu = [NSMenu new];
        NSMenuItem *application = [NSMenuItem new];
        application.submenu = [NSMenu new]; [menu addItem:application]; NSApp.mainMenu = menu;
        NSString *version = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"CFBundleVersion"];
        if ([version integerValue] == 2) { record(@"relaunch.marker"); return 0; }
        clicked = [NSMutableSet new];
        if (extend_updates_start(busy, prepare, cleanup, cancel) != 1) return 4;
        if (![application.submenu itemWithTag:48179]) return 5;
        record(@"menu.marker");
        extend_updates_preferences(false, false, "stable");
        NSTimer *inspection = [NSTimer scheduledTimerWithTimeInterval:0.25 repeats:YES block:^(NSTimer *timer) {
            if (postponedAt && sharing && -postponedAt.timeIntervalSinceNow > 3) {
                sharing = NO; record(@"sharing-ended.marker");
            }
            for (NSWindow *window in NSApp.windows) if (window.visible) inspect(window.contentView, window);
        }];
        [[NSRunLoop mainRunLoop] addTimer:inspection forMode:NSModalPanelRunLoopMode];
        [NSTimer scheduledTimerWithTimeInterval:90 repeats:NO block:^(NSTimer *timer) {
            record(@"timeout.marker"); exit(6);
        }];
        [NSTimer scheduledTimerWithTimeInterval:1 repeats:NO block:^(NSTimer *timer) { extend_updates_check(); }];
        [NSApp run];
    }
    return 0;
}
