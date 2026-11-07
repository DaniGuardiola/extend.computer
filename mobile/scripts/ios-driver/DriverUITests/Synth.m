#import "Synth.h"
#import <CoreGraphics/CoreGraphics.h>

@interface NSObject (DriverPrivateEvents)
- (instancetype)initForTouchAtPoint:(CGPoint)point offset:(double)offset;
- (void)moveToPoint:(CGPoint)point atOffset:(double)offset;
- (void)liftUpAtOffset:(double)offset;
- (instancetype)initWithName:(NSString *)name interfaceOrientation:(long long)orientation;
- (void)addPointerEventPath:(id)path;
- (BOOL)synthesizeWithError:(NSError **)error;
@end

BOOL DriverSynthesizeTouches(NSArray<NSArray<NSNumber *> *> *paths, NSString **error) {
  Class pathClass = NSClassFromString(@"XCPointerEventPath");
  Class recordClass = NSClassFromString(@"XCSynthesizedEventRecord");
  if (!pathClass || !recordClass) { if (error) *error = @"private event classes missing"; return NO; }
  id record = [[recordClass alloc] initWithName:@"driver touches" interfaceOrientation:1];
  for (NSArray<NSNumber *> *p in paths) {
    if (p.count < 6 || p.count % 3) { if (error) *error = @"bad path"; return NO; }
    id path = [[pathClass alloc] initForTouchAtPoint:CGPointMake(p[0].doubleValue, p[1].doubleValue) offset:p[2].doubleValue];
    NSUInteger last = p.count - 3;
    for (NSUInteger i = 3; i < last; i += 3)
      [path moveToPoint:CGPointMake(p[i].doubleValue, p[i + 1].doubleValue) atOffset:p[i + 2].doubleValue];
    [path moveToPoint:CGPointMake(p[last].doubleValue, p[last + 1].doubleValue) atOffset:p[last + 2].doubleValue];
    [path liftUpAtOffset:p[last + 2].doubleValue];
    [record addPointerEventPath:path];
  }
  NSError *failure = nil;
  if (![record synthesizeWithError:&failure]) { if (error) *error = failure.description ?: @"synthesis failed"; return NO; }
  return YES;
}
