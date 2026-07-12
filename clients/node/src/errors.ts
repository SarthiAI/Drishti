// Error taxonomy mapped from the Drishti HTTP contract.
// The server returns a JSON body { "error": "<msg>" } with a status code.

export class DrishtiError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DrishtiError";
  }
}

export class DrishtiAPIError extends DrishtiError {
  readonly status: number;
  readonly serverMessage: string;
  readonly requestId?: string;

  constructor(status: number, message: string, requestId?: string) {
    const suffix = requestId ? ` (request_id=${requestId})` : "";
    super(`[${status}] ${message}${suffix}`);
    this.name = "DrishtiAPIError";
    this.status = status;
    this.serverMessage = message;
    this.requestId = requestId;
  }
}

export class BadRequestError extends DrishtiAPIError {
  constructor(status: number, message: string, requestId?: string) {
    super(status, message, requestId);
    this.name = "BadRequestError";
  }
}

export class AuthError extends DrishtiAPIError {
  constructor(status: number, message: string, requestId?: string) {
    super(status, message, requestId);
    this.name = "AuthError";
  }
}

export class CheckNotEnabledError extends DrishtiAPIError {
  constructor(status: number, message: string, requestId?: string) {
    super(status, message, requestId);
    this.name = "CheckNotEnabledError";
  }
}

export class ServerError extends DrishtiAPIError {
  constructor(status: number, message: string, requestId?: string) {
    super(status, message, requestId);
    this.name = "ServerError";
  }
}

export class DrishtiTransportError extends DrishtiError {
  constructor(message: string) {
    super(message);
    this.name = "DrishtiTransportError";
  }
}

export class DrishtiTimeoutError extends DrishtiTransportError {
  constructor(message: string) {
    super(message);
    this.name = "DrishtiTimeoutError";
  }
}

export class DrishtiConnectionError extends DrishtiTransportError {
  constructor(message: string) {
    super(message);
    this.name = "DrishtiConnectionError";
  }
}

// Map an HTTP status to a typed error. 401 and 501 are specific contract
// statuses. Every other 4xx is a request-side error (bad request, wrong shape,
// unsupported media type, and anything a proxy or gateway may return), so it
// maps to BadRequestError. 5xx and anything else map to ServerError.
export function apiErrorForStatus(
  status: number,
  message: string,
  requestId?: string,
): DrishtiAPIError {
  if (status === 401) return new AuthError(status, message, requestId);
  if (status === 501) return new CheckNotEnabledError(status, message, requestId);
  if (status >= 400 && status < 500) return new BadRequestError(status, message, requestId);
  return new ServerError(status, message, requestId);
}
