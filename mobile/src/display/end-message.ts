// Native termination can carry an empty error after an ordinary source stop.
// Treat missing detail as an unspecified end, without claiming a successful
// disconnect or hiding an actual transport, permission, or decoder failure.
export function displayEndMessage(message?: string): string | undefined {
  const detail = message?.trim();
  if (!detail || /^(?:Display ended\s*:?|Display session ended\.|Connection ended\.)$/.test(detail)) {
    return undefined;
  }
  return detail;
}
