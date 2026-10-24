export type ManualPairingState = {
  id: string;
  state: "starting" | "connecting" | "waiting" | "paired" | "failed" | "expired" | "closed";
  peer?: string;
  error?: string;
};
type Bridge = {
  reserveManualPairing(): string;
  startManualPairing(id: string, address: string, code: string, name: string): Promise<string>;
  getManualPairing(id: string): Promise<string>;
  stopManualPairing(id: string): void;
};
type Operation = { id: string; reading: boolean };
const phases = new Set(["starting", "connecting", "waiting", "paired", "failed", "expired", "closed"]);

/** One native reservation, including async work that has not started yet. */
export class ManualPairingAttempt {
  private current?: Operation;
  private readonly bridge: Bridge;
  constructor(bridge: Bridge) { this.bridge = bridge; }
  get id() { return this.current?.id; }

  async start(address: string, code: string, name: string): Promise<ManualPairingState | null> {
    if (this.current) throw new Error("Pairing already in progress");
    const operation = { id: this.bridge.reserveManualPairing(), reading: false };
    this.current = operation;
    try {
      return this.accept(operation, await this.bridge.startManualPairing(operation.id, address, code, name));
    } catch (error) {
      if (this.current !== operation) return null;
      this.stop();
      throw error;
    }
  }
  async read(): Promise<ManualPairingState | null> {
    const operation = this.current;
    if (!operation || operation.reading) return null;
    operation.reading = true;
    try {
      return this.accept(operation, await this.bridge.getManualPairing(operation.id));
    } catch (error) {
      if (this.current !== operation) return null;
      this.stop();
      throw error;
    } finally { operation.reading = false; }
  }
  stop() {
    const operation = this.current;
    this.current = undefined;
    if (operation) this.bridge.stopManualPairing(operation.id);
  }
  private accept(operation: Operation, raw: string): ManualPairingState | null {
    if (this.current !== operation) return null;
    const state = JSON.parse(raw) as ManualPairingState;
    if (state.id !== operation.id || !phases.has(state.state) || (state.state === "paired" && !/^[0-9a-f]{64}$/.test(state.peer || "")))
      throw new Error("Pairing operation changed");
    if (!["starting", "connecting", "waiting"].includes(state.state)) this.stop();
    return state;
  }
}
