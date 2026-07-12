// Shared retry policy: which failures are retryable and how long to wait.
// Exponential backoff with full jitter, capped.

// 429 is included for future-proofing even though the current server does not
// emit it. 400, 401, and 501 are terminal and never retried.
export const RETRYABLE_STATUSES = new Set<number>([429, 500, 502, 503, 504]);

export function isRetryableStatus(status: number): boolean {
  return RETRYABLE_STATUSES.has(status);
}

// Full-jitter backoff for a zero-indexed retry attempt, in milliseconds.
export function backoffDelayMs(attempt: number, baseMs: number, capMs: number): number {
  const ceiling = Math.min(capMs, baseMs * 2 ** attempt);
  if (ceiling <= 0) return 0;
  return Math.random() * ceiling;
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
