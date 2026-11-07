#import <Foundation/Foundation.h>

/// Multi-finger touch synthesis through XCTest's private event record (as
/// WebDriverAgent does). Each path is a flat [x, y, t, x, y, t, ...] list in
/// screen points and seconds: first triple = finger down, middle = moves,
/// last = lift. Returns NO with a message when the private API is missing.
BOOL DriverSynthesizeTouches(NSArray<NSArray<NSNumber *> *> *paths, NSString *_Nullable *_Nullable error);
