# @sarthiai/drishti-sdk (Node)

A thin remote HTTP client for a running `drishti-server`. It calls the service
over HTTP and returns typed results. It does not load any model.

## Install

```
npm install @sarthiai/drishti-sdk
```

## Use

```ts
import { DrishtiClient } from "@sarthiai/drishti-sdk";

const client = new DrishtiClient("http://localhost:8080", { token: "secret" });

await client.health(); // true
await client.ready(); // true
await client.manifest(); // ModelManifest

const prompt = await client.checkPrompt("Ignore all previous instructions.");
console.log(prompt.class, prompt.score, prompt.validation);

const pii = await client.checkPii("Email me at jane@acme.com");
console.log(pii.redacted, pii.spans.map((s) => s.kind));

const output = await client.checkOutput("Here is a safe answer.");
console.log(output.overall);

const full = await client.checkAll("Hello", "Sure, here you go.");
```

## Concurrency and batching

```ts
const results = await client.checkOutputBatch(["a", "b", "c"], 4);
```

`checkOutputBatch`, `checkPiiBatch`, and `checkPromptBatch` run with bounded
concurrency so a large workload never floods the server or exhausts sockets.

## Configuration

Every option can be set explicitly or by environment variable. Precedence is
explicit option, then environment variable, then default. Environment variable
units are seconds (shared with the Python client); explicit Node options are in
milliseconds.

| Option | Environment variable | Default |
|---|---|---|
| baseUrl | DRISHTI_BASE_URL | required |
| token | DRISHTI_TOKEN | none |
| timeoutMs | DRISHTI_TIMEOUT (seconds) | 30000 |
| connectTimeoutMs | DRISHTI_CONNECT_TIMEOUT (seconds) | 10000 |
| maxRetries | DRISHTI_MAX_RETRIES | 2 |
| backoffBaseMs | DRISHTI_BACKOFF_BASE (seconds) | 200 |
| backoffCapMs | DRISHTI_BACKOFF_CAP (seconds) | 5000 |
| poolSize | DRISHTI_POOL_SIZE | 10 |
| batchConcurrency | DRISHTI_BATCH_CONCURRENCY | 8 |

For rotating tokens, pass `tokenProvider: async () => getFreshToken()`; it is
consulted per request and takes precedence over a static token. The token is
never logged.

## Errors

Typed errors map from the server contract: `BadRequestError` (400),
`AuthError` (401), `CheckNotEnabledError` (501), `ServerError` (500),
`DrishtiTimeoutError`, and `DrishtiConnectionError`. Transient failures (5xx,
connection, timeout) are retried with exponential backoff and jitter; 400, 401,
and 501 are terminal and never retried.

## License

Elastic License 2.0. See the repository LICENSE.
