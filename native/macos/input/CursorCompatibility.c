#include "CursorCompatibility.h"
#include <dlfcn.h>

CGError ExtendComputerDisableWarpSuppression(void) {
    // macOS still applies a 250 ms default to CGWarpMouseCursorPosition.
    // This deprecated public API controls that path; configuring a private
    // event source only affects events posted through that source.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    CGError result = CGSetLocalEventsSuppressionInterval(0.0);
#pragma clang diagnostic pop
    return result;
}

// macOS public hide/show calls normally require the foreground application.
// Resolve this optional compatibility API at runtime: never steal focus, and
// keep cursor control usable with a visible cursor if it becomes unavailable.
bool ExtendComputerEnableBackgroundCursorVisibility(void) {
    typedef int32_t (*ConnectionFunction)(void);
    typedef CGError (*PropertyFunction)(int32_t, int32_t, CFStringRef, CFTypeRef);
    ConnectionFunction connection = (ConnectionFunction)dlsym(RTLD_DEFAULT, "_CGSDefaultConnection");
    PropertyFunction setProperty = (PropertyFunction)dlsym(RTLD_DEFAULT, "CGSSetConnectionProperty");
    if (!connection || !setProperty) return false;
    int32_t identifier = connection();
    return setProperty(identifier, identifier, CFSTR("SetsCursorInBackground"), kCFBooleanTrue) == kCGErrorSuccess;
}
