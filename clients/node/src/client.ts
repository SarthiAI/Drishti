// Synchronous-looking, promise-based Drishti HTTP client for Node.
// A thin remote client over a running drishti-server. It never loads a model.

import { randomUUID } from "node:crypto";
import {
  DrishtiClientOptions,
  ResolvedConfig,
  resolveConfig,
} from "./config.js";
import {
  DrishtiConnectionError,
  DrishtiTimeoutError,
  apiErrorForStatus,
} from "./errors.js";
import {
  FullCheck,
  ModelManifest,
  OutputCheck,
  PiiCheck,
  PromptCheck,
  parseFullCheck,
  parseModelManifest,
  parseOutputCheck,
  parsePiiCheck,
  parsePromptCheck,
} from "./models.js";
import { backoffDelayMs, isRetryableStatus, sleep } from "./retry.js";

async function mapWithConcurrency<T, R>(
  items: T[],
  limit: number,
  fn: (item: T, index: number) => Promise<R>,
): Promise<R[]> {
  const results: R[] = new Array(items.length);
  const workers = Math.max(1, Math.min(limit, items.length || 1));
  let next = 0;
  const run = async (): Promise<void> => {
    while (true) {
      const i = next++;
      if (i >= items.length) break;
      results[i] = await fn(items[i], i);
    }
  };
  await Promise.all(Array.from({ length: workers }, run));
  return results;
}

export class DrishtiClient {
  private readonly config: ResolvedConfig;

  constructor(baseUrlOrOptions?: string | DrishtiClientOptions, options?: DrishtiClientOptions) {
    if (typeof baseUrlOrOptions === "string") {
      this.config = resolveConfig({ ...(options ?? {}), baseUrl: baseUrlOrOptions });
    } else {
      this.config = resolveConfig(baseUrlOrOptions ?? {});
    }
  }

  // ---- internals -----------------------------------------------------

  private async currentToken(): Promise<string | undefined> {
    if (this.config.tokenProvider) {
      const t = await this.config.tokenProvider();
      return t ?? undefined;
    }
    return this.config.token;
  }

  private async headers(requestId: string, auth: boolean, json: boolean): Promise<Record<string, string>> {
    const headers: Record<string, string> = {
      "X-Request-Id": requestId,
      Accept: "application/json",
    };
    if (json) headers["Content-Type"] = "application/json";
    if (auth) {
      const token = await this.currentToken();
      if (token) headers["Authorization"] = `Bearer ${token}`;
    }
    return headers;
  }

  private emit(event: Record<string, unknown>): void {
    // Never include the token in events.
    if (this.config.onEvent) {
      try {
        this.config.onEvent(event);
      } catch {
        // an observer must never break a request
      }
    }
  }

  private async request(
    method: string,
    path: string,
    opts: { json?: unknown; auth: boolean },
  ): Promise<Response> {
    const requestId = randomUUID();
    const attempts = this.config.maxRetries + 1;
    let lastErr: Error | undefined;

    for (let attempt = 0; attempt < attempts; attempt++) {
      const headers = await this.headers(requestId, opts.auth, opts.json !== undefined);
      this.emit({ event: "request", method, path, attempt, requestId });

      let res: Response;
      try {
        res = await fetch(`${this.config.baseUrl}${path}`, {
          method,
          headers,
          body: opts.json !== undefined ? JSON.stringify(opts.json) : undefined,
          signal: AbortSignal.timeout(this.config.timeoutMs),
        });
      } catch (err: any) {
        if (err && (err.name === "TimeoutError" || err.name === "AbortError")) {
          lastErr = new DrishtiTimeoutError(`request to ${path} timed out`);
        } else {
          lastErr = new DrishtiConnectionError(`connection to ${path} failed: ${err?.message ?? err}`);
        }
        if (attempt < attempts - 1) {
          await this.sleepBackoff(attempt);
          continue;
        }
        throw lastErr;
      }

      if (res.ok) {
        this.emit({ event: "response", path, status: res.status, requestId });
        return res;
      }
      if (isRetryableStatus(res.status) && attempt < attempts - 1) {
        await this.sleepBackoff(attempt);
        continue;
      }
      throw await this.errorFromResponse(res, requestId);
    }

    throw lastErr ?? new DrishtiConnectionError(`request to ${path} failed`);
  }

  private async sleepBackoff(attempt: number): Promise<void> {
    const delay = backoffDelayMs(attempt, this.config.backoffBaseMs, this.config.backoffCapMs);
    this.emit({ event: "retry", attempt, delayMs: Math.round(delay) });
    await sleep(delay);
  }

  private async errorFromResponse(res: Response, requestId: string): Promise<Error> {
    let message = `HTTP ${res.status}`;
    try {
      const body: any = await res.json();
      if (body && typeof body === "object" && "error" in body) {
        message = String(body.error);
      }
    } catch {
      const text = (await res.text().catch(() => "")).trim();
      if (text) message = text;
    }
    return apiErrorForStatus(res.status, message, requestId);
  }

  private async postJson(path: string, payload: unknown): Promise<any> {
    const res = await this.request("POST", path, { json: payload, auth: true });
    return res.json();
  }

  // ---- open endpoints ------------------------------------------------

  async health(): Promise<boolean> {
    const res = await this.request("GET", "/healthz", { auth: false });
    return (await res.text()).trim() === "ok";
  }

  async ready(): Promise<boolean> {
    const res = await this.request("GET", "/readyz", { auth: false });
    return (await res.text()).trim() === "ready";
  }

  async metrics(): Promise<string> {
    const res = await this.request("GET", "/metrics", { auth: false });
    return res.text();
  }

  // ---- manifest ------------------------------------------------------

  async manifest(): Promise<ModelManifest> {
    const res = await this.request("GET", "/v1/manifest", { auth: true });
    return parseModelManifest(await res.json());
  }

  // ---- checks --------------------------------------------------------

  async checkPrompt(text: string): Promise<PromptCheck> {
    return parsePromptCheck(await this.postJson("/v1/check/prompt", { input: text }));
  }

  async checkPii(text: string): Promise<PiiCheck> {
    return parsePiiCheck(await this.postJson("/v1/check/pii", { input: text }));
  }

  async checkOutput(text: string): Promise<OutputCheck> {
    return parseOutputCheck(await this.postJson("/v1/check/output", { output: text }));
  }

  async checkAll(prompt: string, output?: string): Promise<FullCheck> {
    const payload: Record<string, unknown> = { prompt };
    if (output !== undefined) payload.output = output;
    return parseFullCheck(await this.postJson("/v1/check/all", payload));
  }

  // ---- batch helpers (bounded concurrency) ---------------------------

  async checkPromptBatch(texts: string[], concurrency?: number): Promise<PromptCheck[]> {
    return mapWithConcurrency(texts, concurrency ?? this.config.batchConcurrency, (t) =>
      this.checkPrompt(t),
    );
  }

  async checkPiiBatch(texts: string[], concurrency?: number): Promise<PiiCheck[]> {
    return mapWithConcurrency(texts, concurrency ?? this.config.batchConcurrency, (t) =>
      this.checkPii(t),
    );
  }

  async checkOutputBatch(texts: string[], concurrency?: number): Promise<OutputCheck[]> {
    return mapWithConcurrency(texts, concurrency ?? this.config.batchConcurrency, (t) =>
      this.checkOutput(t),
    );
  }
}
