// Client configuration resolution.
//
// Precedence, highest first (ADR-008):
//   1. explicit option passed to the constructor
//   2. environment variable
//   3. built-in default
//
// Environment variable names and their units (seconds) are shared with the
// Python client. Explicit Node options are in milliseconds, idiomatic for Node.

export type TokenProvider = () => string | null | undefined | Promise<string | null | undefined>;

export type EventHook = (event: Record<string, unknown>) => void;

export interface DrishtiClientOptions {
  baseUrl?: string;
  token?: string;
  tokenProvider?: TokenProvider;
  timeoutMs?: number;
  connectTimeoutMs?: number;
  maxRetries?: number;
  backoffBaseMs?: number;
  backoffCapMs?: number;
  poolSize?: number;
  batchConcurrency?: number;
  onEvent?: EventHook;
}

export interface ResolvedConfig {
  baseUrl: string;
  token?: string;
  tokenProvider?: TokenProvider;
  timeoutMs: number;
  connectTimeoutMs: number;
  maxRetries: number;
  backoffBaseMs: number;
  backoffCapMs: number;
  poolSize: number;
  batchConcurrency: number;
  onEvent?: EventHook;
}

const DEFAULTS = {
  timeoutMs: 30_000,
  connectTimeoutMs: 10_000,
  maxRetries: 2,
  backoffBaseMs: 200,
  backoffCapMs: 5_000,
  poolSize: 10,
  batchConcurrency: 8,
};

function envStr(name: string): string | undefined {
  const value = process.env[name];
  return value !== undefined && value !== "" ? value : undefined;
}

// Env values are in seconds (shared with Python); convert to milliseconds.
function envSeconds(name: string): number | undefined {
  const raw = envStr(name);
  if (raw === undefined) return undefined;
  const n = Number(raw);
  if (Number.isNaN(n)) throw new Error(`environment variable ${name}=${raw} is not a valid number`);
  return n * 1000;
}

function envInt(name: string): number | undefined {
  const raw = envStr(name);
  if (raw === undefined) return undefined;
  const n = Number.parseInt(raw, 10);
  if (Number.isNaN(n)) throw new Error(`environment variable ${name}=${raw} is not a valid integer`);
  return n;
}

function pick<T>(explicit: T | undefined, envValue: T | undefined, fallback: T): T {
  if (explicit !== undefined) return explicit;
  if (envValue !== undefined) return envValue;
  return fallback;
}

export function resolveConfig(options: DrishtiClientOptions = {}): ResolvedConfig {
  const baseUrl = pick(options.baseUrl, envStr("DRISHTI_BASE_URL"), "");
  if (!baseUrl) {
    throw new Error("baseUrl is required: pass it explicitly or set DRISHTI_BASE_URL");
  }
  return {
    baseUrl: baseUrl.replace(/\/+$/, ""),
    token: pick(options.token, envStr("DRISHTI_TOKEN"), undefined),
    tokenProvider: options.tokenProvider,
    timeoutMs: pick(options.timeoutMs, envSeconds("DRISHTI_TIMEOUT"), DEFAULTS.timeoutMs),
    connectTimeoutMs: pick(
      options.connectTimeoutMs,
      envSeconds("DRISHTI_CONNECT_TIMEOUT"),
      DEFAULTS.connectTimeoutMs,
    ),
    maxRetries: pick(options.maxRetries, envInt("DRISHTI_MAX_RETRIES"), DEFAULTS.maxRetries),
    backoffBaseMs: pick(options.backoffBaseMs, envSeconds("DRISHTI_BACKOFF_BASE"), DEFAULTS.backoffBaseMs),
    backoffCapMs: pick(options.backoffCapMs, envSeconds("DRISHTI_BACKOFF_CAP"), DEFAULTS.backoffCapMs),
    poolSize: pick(options.poolSize, envInt("DRISHTI_POOL_SIZE"), DEFAULTS.poolSize),
    batchConcurrency: pick(
      options.batchConcurrency,
      envInt("DRISHTI_BATCH_CONCURRENCY"),
      DEFAULTS.batchConcurrency,
    ),
    onEvent: options.onEvent,
  };
}
