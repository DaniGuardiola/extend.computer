// Isolated production-guard regression. Never invokes CoreGraphics mutations.
// Build this file alone with -fobjc-arc and Foundation/CoreGraphics frameworks.
#import "VirtualDisplay.m"
#include <assert.h>
#include <stdio.h>

typedef struct {
    ECTopology snapshot;
    bool readFails, postReadFails, repairFails, keepMirror, loseMain, snapOrigin;
    unsigned reads, repairs;
    CGDirectDisplayID repairedID, slaves[EC_TOPOLOGY_LIMIT];
    uint32_t slaveCount;
    CGPoint requestedOrigin;
} Fixture;
static bool ReadFixture(void *context, ECTopology *snapshot) {
    Fixture *fixture = context;
    fixture->reads++;
    if (fixture->readFails || (fixture->postReadFails && fixture->reads > 1)) return false;
    *snapshot = fixture->snapshot;
    return true;
}
static bool RepairFixture(void *context, CGDirectDisplayID id,
                          const CGDirectDisplayID *slaves, uint32_t count, CGPoint origin) {
    Fixture *fixture = context;
    fixture->repairs++;
    fixture->repairedID = id;
    fixture->slaveCount = count;
    fixture->requestedOrigin = origin;
    memcpy(fixture->slaves, slaves, sizeof(*slaves) * count);
    if (fixture->repairFails) return false;
    for (uint32_t i = 0; i < fixture->snapshot.count; i++) {
        ECTopologyRow *row = &fixture->snapshot.rows[i];
        if (row->id == id) {
            row->mirrors = 0;
            row->active = true;
            row->origin = fixture->snapOrigin ? CGPointMake(origin.x + 10, origin.y) : origin;
        }
        for (uint32_t j = 0; j < count; j++)
            if (row->id == slaves[j] && !fixture->keepMirror) {
                row->mirrors = 0;
                row->active = true;
            }
    }
    if (!fixture->loseMain) fixture->snapshot.mainID = 1;
    return true;
}
static Fixture Baseline(void) {
    Fixture fixture = {0};
    fixture.snapshot = (ECTopology){.count=4, .mainID=1, .rows={
        {1, 0, true, {0, 0}}, {67, 0, true, {1728, 0}},
        // An unrelated mirror group must never enter the repair transaction.
        {90, 0, true, {-1920, 0}}, {91, 90, false, {-1920, 0}}
    }};
    return fixture;
}
static Fixture Mirrored(void) {
    Fixture fixture = Baseline();
    fixture.snapshot.mainID = 67;
    fixture.snapshot.rows[0].mirrors = 67;
    fixture.snapshot.rows[0].active = false;
    fixture.snapshot.rows[1].origin = CGPointZero;
    return fixture;
}
static bool Guard(Fixture *fixture, CGDirectDisplayID mainID) {
    return ECGuardExtension((ECTopologyBackend){ReadFixture, RepairFixture, fixture},
        67, CGPointMake(1728, 0), mainID);
}
int main(void) {
    @autoreleasepool {
        Fixture fixture = Baseline();
        assert(Guard(&fixture, 1));
        assert(fixture.repairs == 0 && fixture.reads == 1);

        fixture = Mirrored();
        assert(Guard(&fixture, 1));
        assert(fixture.repairs == 1 && fixture.reads == 2);
        assert(fixture.repairedID == 67 && fixture.slaveCount == 1 && fixture.slaves[0] == 1);
        assert(fixture.requestedOrigin.x == 1728 && fixture.requestedOrigin.y == 0);
        assert(fixture.snapshot.rows[2].id == 90 && fixture.snapshot.rows[3].mirrors == 90);
        assert(fixture.snapshot.rows[0].active && fixture.snapshot.mainID == 1);

        fixture = Mirrored(); fixture.snapOrigin = true;
        assert(Guard(&fixture, 1)); // CoreGraphics may snap origins after a resize.
        fixture = Mirrored(); fixture.keepMirror = true;
        assert(!Guard(&fixture, 1));
        fixture = Mirrored(); fixture.loseMain = true;
        assert(!Guard(&fixture, 1));
        fixture = Mirrored(); fixture.repairFails = true;
        assert(!Guard(&fixture, 1));
        fixture = Mirrored(); fixture.postReadFails = true;
        assert(!Guard(&fixture, 1));
        fixture = Baseline(); fixture.readFails = true;
        assert(!Guard(&fixture, 1) && fixture.repairs == 0);
        fixture = Baseline(); fixture.snapshot.rows[0].active = false;
        assert(!Guard(&fixture, 1) && fixture.repairs == 0);
        fixture = Baseline(); fixture.snapshot.rows[1].active = false;
        assert(!Guard(&fixture, 1) && fixture.repairs == 0);
        fixture = Baseline(); fixture.snapshot.rows[1].id = 68;
        assert(!Guard(&fixture, 1) && fixture.repairs == 0);
        fixture = Baseline(); fixture.snapshot.mainID = 67;
        assert(Guard(&fixture, 67) && fixture.repairs == 0); // User-chosen main is preserved.
        assert(!Guard(&fixture, 1));
        fixture = Baseline(); fixture.snapshot.rows[1].mirrors = 1;
        assert(Guard(&fixture, 1) && fixture.slaveCount == 0);
        puts("PASS: virtual-only mirror repair, unrelated groups, no-op, main role, snapped origin, failed repair/read and inactive/missing rejection");
    }
    return 0;
}
