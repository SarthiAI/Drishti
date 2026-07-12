"""Client configuration resolution.

Precedence, highest first (ADR-008):
  1. explicit argument passed to the constructor
  2. environment variable
  3. built-in default

No tunable requires a code change or rebuild. Environment variable names are
shared with the Node client so operators learn them once.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Callable, Optional

DEFAULT_TIMEOUT = 30.0
DEFAULT_CONNECT_TIMEOUT = 10.0
DEFAULT_MAX_RETRIES = 2
DEFAULT_BACKOFF_BASE = 0.2
DEFAULT_BACKOFF_CAP = 5.0
DEFAULT_POOL_SIZE = 10
DEFAULT_BATCH_CONCURRENCY = 8

TokenProvider = Callable[[], Optional[str]]


def _env_str(name: str) -> Optional[str]:
    value = os.environ.get(name)
    return value if value not in (None, "") else None


def _env_float(name: str) -> Optional[float]:
    raw = _env_str(name)
    if raw is None:
        return None
    try:
        return float(raw)
    except ValueError:
        raise ValueError(f"environment variable {name}={raw!r} is not a valid number")


def _env_int(name: str) -> Optional[int]:
    raw = _env_str(name)
    if raw is None:
        return None
    try:
        return int(raw)
    except ValueError:
        raise ValueError(f"environment variable {name}={raw!r} is not a valid integer")


def _pick(explicit, env_value, default):
    if explicit is not None:
        return explicit
    if env_value is not None:
        return env_value
    return default


@dataclass
class ClientConfig:
    base_url: str
    token: Optional[str] = None
    token_provider: Optional[TokenProvider] = None
    timeout: float = DEFAULT_TIMEOUT
    connect_timeout: float = DEFAULT_CONNECT_TIMEOUT
    max_retries: int = DEFAULT_MAX_RETRIES
    backoff_base: float = DEFAULT_BACKOFF_BASE
    backoff_cap: float = DEFAULT_BACKOFF_CAP
    pool_size: int = DEFAULT_POOL_SIZE
    batch_concurrency: int = DEFAULT_BATCH_CONCURRENCY


def resolve_config(
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
) -> ClientConfig:
    resolved_base = _pick(base_url, _env_str("DRISHTI_BASE_URL"), None)
    if not resolved_base:
        raise ValueError(
            "base_url is required: pass it explicitly or set DRISHTI_BASE_URL"
        )

    return ClientConfig(
        base_url=resolved_base.rstrip("/"),
        token=_pick(token, _env_str("DRISHTI_TOKEN"), None),
        token_provider=token_provider,
        timeout=_pick(timeout, _env_float("DRISHTI_TIMEOUT"), DEFAULT_TIMEOUT),
        connect_timeout=_pick(
            connect_timeout, _env_float("DRISHTI_CONNECT_TIMEOUT"), DEFAULT_CONNECT_TIMEOUT
        ),
        max_retries=_pick(max_retries, _env_int("DRISHTI_MAX_RETRIES"), DEFAULT_MAX_RETRIES),
        backoff_base=_pick(backoff_base, _env_float("DRISHTI_BACKOFF_BASE"), DEFAULT_BACKOFF_BASE),
        backoff_cap=_pick(backoff_cap, _env_float("DRISHTI_BACKOFF_CAP"), DEFAULT_BACKOFF_CAP),
        pool_size=_pick(pool_size, _env_int("DRISHTI_POOL_SIZE"), DEFAULT_POOL_SIZE),
        batch_concurrency=_pick(
            batch_concurrency, _env_int("DRISHTI_BATCH_CONCURRENCY"), DEFAULT_BATCH_CONCURRENCY
        ),
    )
