"""Shared retry policy: which failures are retryable and how long to wait.

Exponential backoff with full jitter, capped. Used by both the sync and async
clients so the policy is identical.
"""

from __future__ import annotations

import random

# Transient HTTP statuses worth retrying. 429 is included for future-proofing
# even though the current server does not emit it. 400, 401, and 501 are
# terminal and never retried.
RETRYABLE_STATUSES = frozenset({429, 500, 502, 503, 504})


def is_retryable_status(status: int) -> bool:
    return status in RETRYABLE_STATUSES


def backoff_delay(attempt: int, base: float, cap: float) -> float:
    """Full-jitter backoff for a zero-indexed retry attempt.

    delay = random(0, min(cap, base * 2**attempt))
    """
    ceiling = min(cap, base * (2 ** attempt))
    if ceiling <= 0:
        return 0.0
    return random.uniform(0.0, ceiling)
