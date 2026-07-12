"""Synchronous Drishti HTTP client.

A thin remote client over a running drishti-server. It never loads a model; it
serializes a request, calls the server, and deserializes a typed result.
"""

from __future__ import annotations

import logging
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from typing import Any, Callable, Dict, List, Optional

import httpx

from ._config import ClientConfig, TokenProvider, resolve_config
from ._errors import (
    DrishtiConnectionError,
    DrishtiTimeoutError,
    api_error_for_status,
)
from ._models import (
    FullCheck,
    ModelManifest,
    OutputCheck,
    PiiCheck,
    PromptCheck,
)
from ._retry import backoff_delay, is_retryable_status

logger = logging.getLogger("drishti_client")

EventHook = Callable[[Dict[str, Any]], None]


class DrishtiClient:
    """Synchronous client for a running drishti-server.

    Configuration precedence is explicit argument, then environment variable,
    then default (see ADR-008). The bearer token is never logged.
    """

    def __init__(
        self,
        base_url: Optional[str] = None,
        *,
        token: Optional[str] = None,
        token_provider: Optional[TokenProvider] = None,
        timeout: Optional[float] = None,
        connect_timeout: Optional[float] = None,
        max_retries: Optional[int] = None,
        backoff_base: Optional[float] = None,
        backoff_cap: Optional[float] = None,
        pool_size: Optional[int] = None,
        batch_concurrency: Optional[int] = None,
        on_event: Optional[EventHook] = None,
    ) -> None:
        self._config: ClientConfig = resolve_config(
            base_url,
            token=token,
            token_provider=token_provider,
            timeout=timeout,
            connect_timeout=connect_timeout,
            max_retries=max_retries,
            backoff_base=backoff_base,
            backoff_cap=backoff_cap,
            pool_size=pool_size,
            batch_concurrency=batch_concurrency,
        )
        self._on_event = on_event
        limits = httpx.Limits(
            max_connections=self._config.pool_size,
            max_keepalive_connections=self._config.pool_size,
        )
        timeout_cfg = httpx.Timeout(
            self._config.timeout, connect=self._config.connect_timeout
        )
        self._http = httpx.Client(
            base_url=self._config.base_url, limits=limits, timeout=timeout_cfg
        )

    # ---- lifecycle -----------------------------------------------------

    def close(self) -> None:
        self._http.close()

    def __enter__(self) -> "DrishtiClient":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    # ---- internals -----------------------------------------------------

    def _current_token(self) -> Optional[str]:
        if self._config.token_provider is not None:
            return self._config.token_provider()
        return self._config.token

    def _headers(self, request_id: str, auth: bool) -> Dict[str, str]:
        headers = {"X-Request-Id": request_id, "Accept": "application/json"}
        if auth:
            token = self._current_token()
            if token:
                headers["Authorization"] = f"Bearer {token}"
        return headers

    def _emit(self, event: Dict[str, Any]) -> None:
        # Never include the token in events or logs.
        if self._on_event is not None:
            try:
                self._on_event(event)
            except Exception:  # an observer must never break a request
                logger.exception("drishti_client on_event hook raised")
        logger.debug("drishti_client event: %s", event)

    def _request(
        self,
        method: str,
        path: str,
        *,
        json: Optional[Dict[str, Any]] = None,
        auth: bool = True,
    ) -> httpx.Response:
        request_id = uuid.uuid4().hex
        attempts = self._config.max_retries + 1
        last_exc: Optional[Exception] = None

        for attempt in range(attempts):
            headers = self._headers(request_id, auth)
            self._emit(
                {
                    "event": "request",
                    "method": method,
                    "path": path,
                    "attempt": attempt,
                    "request_id": request_id,
                }
            )
            try:
                response = self._http.request(method, path, json=json, headers=headers)
            except httpx.TimeoutException as exc:
                last_exc = DrishtiTimeoutError(f"request to {path} timed out")
            except httpx.TransportError as exc:
                last_exc = DrishtiConnectionError(f"connection to {path} failed: {exc}")
            else:
                if response.status_code < 400:
                    self._emit(
                        {
                            "event": "response",
                            "path": path,
                            "status": response.status_code,
                            "request_id": request_id,
                        }
                    )
                    return response
                # Error response. Retry only transient statuses.
                if is_retryable_status(response.status_code) and attempt < attempts - 1:
                    self._sleep_backoff(attempt)
                    continue
                raise self._error_from_response(response, request_id)

            # Transport failure path: retry if attempts remain.
            if attempt < attempts - 1:
                self._sleep_backoff(attempt)
                continue
            assert last_exc is not None
            raise last_exc

        # Unreachable, but keeps type checkers content.
        assert last_exc is not None
        raise last_exc

    def _sleep_backoff(self, attempt: int) -> None:
        delay = backoff_delay(attempt, self._config.backoff_base, self._config.backoff_cap)
        self._emit({"event": "retry", "attempt": attempt, "delay_s": round(delay, 3)})
        time.sleep(delay)

    def _error_from_response(self, response: httpx.Response, request_id: str):
        message = f"HTTP {response.status_code}"
        try:
            body = response.json()
            if isinstance(body, dict) and "error" in body:
                message = str(body["error"])
        except Exception:
            text = response.text.strip()
            if text:
                message = text
        return api_error_for_status(response.status_code, message, request_id)

    def _post_json(self, path: str, payload: Dict[str, Any]) -> Dict[str, Any]:
        response = self._request("POST", path, json=payload, auth=True)
        return response.json()

    # ---- open endpoints ------------------------------------------------

    def health(self) -> bool:
        response = self._request("GET", "/healthz", auth=False)
        return response.text.strip() == "ok"

    def ready(self) -> bool:
        response = self._request("GET", "/readyz", auth=False)
        return response.text.strip() == "ready"

    def metrics(self) -> str:
        response = self._request("GET", "/metrics", auth=False)
        return response.text

    # ---- manifest ------------------------------------------------------

    def manifest(self) -> ModelManifest:
        response = self._request("GET", "/v1/manifest", auth=True)
        return ModelManifest.from_dict(response.json())

    # ---- checks --------------------------------------------------------

    def check_prompt(self, text: str) -> PromptCheck:
        return PromptCheck.from_dict(self._post_json("/v1/check/prompt", {"input": text}))

    def check_pii(self, text: str) -> PiiCheck:
        return PiiCheck.from_dict(self._post_json("/v1/check/pii", {"input": text}))

    def check_output(self, text: str) -> OutputCheck:
        return OutputCheck.from_dict(self._post_json("/v1/check/output", {"output": text}))

    def check_all(self, prompt: str, output: Optional[str] = None) -> FullCheck:
        payload: Dict[str, Any] = {"prompt": prompt}
        if output is not None:
            payload["output"] = output
        return FullCheck.from_dict(self._post_json("/v1/check/all", payload))

    # ---- batch helpers (bounded concurrency via a thread pool) ---------

    def check_prompt_batch(
        self, texts: List[str], concurrency: Optional[int] = None
    ) -> List[PromptCheck]:
        return self._batch(self.check_prompt, texts, concurrency)

    def check_pii_batch(
        self, texts: List[str], concurrency: Optional[int] = None
    ) -> List[PiiCheck]:
        return self._batch(self.check_pii, texts, concurrency)

    def check_output_batch(
        self, texts: List[str], concurrency: Optional[int] = None
    ) -> List[OutputCheck]:
        return self._batch(self.check_output, texts, concurrency)

    def _batch(self, fn: Callable[[str], Any], items: List[str], concurrency: Optional[int]):
        workers = concurrency or self._config.batch_concurrency
        workers = max(1, min(workers, len(items) or 1))
        results: List[Any] = [None] * len(items)
        with ThreadPoolExecutor(max_workers=workers) as pool:
            futures = {pool.submit(fn, item): i for i, item in enumerate(items)}
            for future in futures:
                index = futures[future]
                results[index] = future.result()
        return results
