#pragma once
#include <CoreGraphics/CoreGraphics.h>

// Configure the legacy cursor-warp path, which has no event-source argument.
CGError ExtendComputerDisableWarpSuppression(void);

// Optional private WindowServer compatibility; absence leaves the cursor visible.
bool ExtendComputerEnableBackgroundCursorVisibility(void);
