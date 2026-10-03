#import <Cocoa/Cocoa.h>
#import <Sparkle/Sparkle.h>

// All entry points run on the application main thread. Callbacks never expose
// device data to Sparkle; only the current lifecycle state crosses this boundary.
typedef bool (*BusyCallback)(void);
typedef bool (*PrepareCallback)(void);
typedef void (*CleanupCallback)(void);
static BusyCallback sharingBusy;
static PrepareCallback prepareInstallation;
static CleanupCallback cleanupInstallation;
static CleanupCallback cancelInstallation;
static SPUStandardUpdaterController *controller;
static NSTimer *installationTimer;

@interface ExtendUpdaterDelegate : NSObject <SPUUpdaterDelegate>
@end
@implementation ExtendUpdaterDelegate
- (NSString *)feedURLStringForUpdater:(SPUUpdater *)updater {
    NSString *channel = [[NSUserDefaults standardUserDefaults] stringForKey:@"ExtendUpdateChannel"] ?: [[NSBundle mainBundle] objectForInfoDictionaryKey:@"ExtendUpdateDefaultChannel"] ?: @"stable";
    NSString *base = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"ExtendUpdateFeedBase"];
    return [NSString stringWithFormat:@"%@/%@.xml", base, channel];
}
- (NSArray<NSString *> *)allowedSystemProfileKeysForUpdater:(SPUUpdater *)updater { return @[]; }
- (BOOL)updater:(SPUUpdater *)updater mayPerformUpdateCheck:(SPUUpdateCheck)check error:(NSError **)error {
    if (sharingBusy && sharingBusy()) {
        if (error) *error = [NSError errorWithDomain:@"computer.extend.updates" code:1 userInfo:@{NSLocalizedDescriptionKey: @"Finish sharing before checking for updates."}];
        return NO;
    }
    return YES;
}
- (BOOL)updater:(SPUUpdater *)updater shouldPostponeRelaunchForUpdate:(SUAppcastItem *)item untilInvokingBlock:(void (^)(void))installHandler {
    // A session can start after an update check/download. Recheck at relaunch.
    if (prepareInstallation && prepareInstallation()) return NO;
    installationTimer = [NSTimer scheduledTimerWithTimeInterval:1 repeats:YES block:^(NSTimer *unused) {
        if (prepareInstallation && prepareInstallation()) {
            [installationTimer invalidate]; installationTimer = nil;
            installHandler();
        }
    }];
    return YES;
}
- (void)updater:(SPUUpdater *)updater willInstallUpdate:(SUAppcastItem *)item {
    if (cleanupInstallation) cleanupInstallation();
}
- (void)updater:(SPUUpdater *)updater didAbortWithError:(NSError *)error {
    [installationTimer invalidate]; installationTimer = nil;
    if (cancelInstallation) cancelInstallation();
}
@end
static ExtendUpdaterDelegate *updaterDelegate;

void extend_updates_install_menu(void) {
    if (!controller) return;
    NSMenu *menu = NSApp.mainMenu.itemArray.firstObject.submenu;
    if (!menu || [menu itemWithTag:48179]) return;
    NSMenuItem *item = [[NSMenuItem alloc] initWithTitle:@"Check for Updates…" action:@selector(checkForUpdates:) keyEquivalent:@""];
    item.target = controller;
    item.tag = 48179;
    [menu insertItem:item atIndex:MIN(2, menu.numberOfItems)];
}
int extend_updates_start(BusyCallback busy, PrepareCallback prepare, CleanupCallback cleanup, CleanupCallback cancel) {
    NSBundle *bundle = [NSBundle mainBundle];
    // Development builds never contact production feeds or accept updates.
    if ([[bundle bundleIdentifier] hasSuffix:@".development"] ||
        ![[bundle objectForInfoDictionaryKey:@"SUPublicEDKey"] length]) return 0;
    sharingBusy = busy;
    prepareInstallation = prepare;
    cleanupInstallation = cleanup;
    cancelInstallation = cancel;
    updaterDelegate = [ExtendUpdaterDelegate new];
    controller = [[SPUStandardUpdaterController alloc] initWithStartingUpdater:NO updaterDelegate:updaterDelegate userDriverDelegate:nil];
    NSError *error;
    if (![controller.updater startUpdater:&error]) { NSLog(@"Updater unavailable: %@", error); controller = nil; return -1; }
    controller.updater.sendsSystemProfile = NO;
    extend_updates_install_menu();
    return 1;
}
int extend_updates_status(void) {
    if (!controller) return 0;
    return 1 | (controller.updater.automaticallyChecksForUpdates ? 2 : 0) |
        (controller.updater.automaticallyDownloadsUpdates ? 4 : 0);
}
void extend_updates_check(void) { [controller checkForUpdates:nil]; }
void extend_updates_preferences(bool checks, bool downloads, const char *channel) {
    NSString *value = [NSString stringWithUTF8String:channel];
    if (![@[@"stable", @"beta", @"alpha", @"canary"] containsObject:value]) return;
    [[NSUserDefaults standardUserDefaults] setObject:value forKey:@"ExtendUpdateChannel"];
    controller.updater.automaticallyChecksForUpdates = checks;
    controller.updater.automaticallyDownloadsUpdates = downloads;
    [controller.updater resetUpdateCycle];
}
const char *extend_updates_channel(void) {
    NSString *channel = [[NSUserDefaults standardUserDefaults] stringForKey:@"ExtendUpdateChannel"] ?: [[NSBundle mainBundle] objectForInfoDictionaryKey:@"ExtendUpdateDefaultChannel"] ?: @"stable";
    return channel.UTF8String;
}
