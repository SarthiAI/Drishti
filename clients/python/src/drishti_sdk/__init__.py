"""drishti-sdk: a thin remote HTTP client for a running drishti-server.

This package never loads a model. It calls a drishti-server over HTTP and
returns typed results. For the in-process (embedded) binding, see the separate
`drishti` package.
"""

from ._config import ClientConfig
from ._errors import (
    AuthError,
    BadRequestError,
    CheckNotEnabledError,
    DrishtiAPIError,
    DrishtiConnectionError,
    DrishtiError,
    DrishtiTimeoutError,
    DrishtiTransportError,
    ServerError,
    api_error_for_status,
)
from ._models import (
    FullCheck,
    ModelManifest,
    ModelManifestEntry,
    OutputCheck,
    PiiCheck,
    PiiSpan,
    PromptCheck,
)
from .aio import AsyncDrishtiClient
from .client import DrishtiClient

__version__ = "0.1.0"

__all__ = [
    "DrishtiClient",
    "AsyncDrishtiClient",
    "ClientConfig",
    "PromptCheck",
    "PiiCheck",
    "PiiSpan",
    "OutputCheck",
    "FullCheck",
    "ModelManifest",
    "ModelManifestEntry",
    "DrishtiError",
    "DrishtiAPIError",
    "BadRequestError",
    "AuthError",
    "CheckNotEnabledError",
    "ServerError",
    "DrishtiTransportError",
    "DrishtiTimeoutError",
    "DrishtiConnectionError",
    "api_error_for_status",
    "__version__",
]
