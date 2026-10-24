import type { DeviceAvailability } from "../../modules/extend-display";

export const STATUS_TTL_MS = 25_000;
export type DeviceStatus = DeviceAvailability & { checkedAt: number };
export type RequestResult<T> =
  { kind: "done"; value: T } | { kind: "timeout" } | { kind: "busy" };

/** A timeout expires UI evidence, but never frees the slot of unfinished native I/O. */
export class DeviceRequests {
  private pending = false;
  async run<T>(
    operation: () => Promise<T>,
    deadlineMs = 12_000,
  ): Promise<RequestResult<T>> {
    if (this.pending) return { kind: "busy" };
    this.pending = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const request = Promise.resolve()
      .then(operation)
      .finally(() => {
        this.pending = false;
      });
    try {
      return await Promise.race([
        request.then((value) => ({ kind: "done", value }) as const),
        new Promise<{ kind: "timeout" }>((resolve) => {
          timer = setTimeout(() => resolve({ kind: "timeout" }), deadlineMs);
        }),
      ]);
    } finally {
      if (timer) clearTimeout(timer);
    }
  }
}
export const deviceRequests = new DeviceRequests();
export function currentStatus(
  status: DeviceStatus | undefined,
  now: number,
): DeviceAvailability {
  return !status || now - status.checkedAt >= STATUS_TTL_MS
    ? { state: "checking" }
    : status;
}
