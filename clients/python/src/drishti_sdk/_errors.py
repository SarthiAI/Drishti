"""Error taxonomy mapped from the Drishti HTTP contract.

The server returns a JSON body {"error": "<msg>"} with a status code. These
types map each status to a typed error and surface the server message.
"""

from __future__ import annotations

from typing import Optional


class DrishtiError(Exception):
    """Base for every error raised by the client."""


class DrishtiAPIError(DrishtiError):
    """The server returned an HTTP error response."""

    def __init__(self, status: int, message: str, request_id: Optional[str] = None) -> None:
        self.status = status
        self.message = message
        self.request_id = request_id
        suffix = f" (request_id={request_id})" if request_id else ""
        super().__init__(f"[{status}] {message}{suffix}")


class BadRequestError(DrishtiAPIError):
    """400: invalid input or configuration (includes input too long)."""


class AuthError(DrishtiAPIError):
    """401: missing or invalid bearer token."""


class CheckNotEnabledError(DrishtiAPIError):
    """501: the requested check is not enabled on this server."""


class ServerError(DrishtiAPIError):
    """500 or other 5xx: internal server error."""


class DrishtiTransportError(DrishtiError):
    """No HTTP response was received (network-level failure)."""


class DrishtiTimeoutError(DrishtiTransportError):
    """The request exceeded the configured timeout."""


class DrishtiConnectionError(DrishtiTransportError):
    """The connection to the server failed (refused, reset, DNS, and so on)."""


def api_error_for_status(status: int, message: str, request_id: Optional[str] = None) -> DrishtiAPIError:
    """Map an HTTP status to a typed error.

    401 and 501 are specific contract statuses. Every other 4xx is a
    request-side error (bad request, wrong shape, unsupported media type, and
    anything a proxy or gateway may return), so it maps to BadRequestError.
    5xx and anything else map to ServerError.
    """
    if status == 401:
        return AuthError(status, message, request_id)
    if status == 501:
        return CheckNotEnabledError(status, message, request_id)
    if 400 <= status < 500:
        return BadRequestError(status, message, request_id)
    return ServerError(status, message, request_id)
