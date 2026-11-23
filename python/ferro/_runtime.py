"""Which Ferro Link the generated tool functions talk to.

``ferro.connect()`` makes its link the default. A ``Document`` binds its own
link while one of its helpers runs, so two documents (or threads) do not mix.
"""

import contextlib
import contextvars
from typing import Any, Dict, Iterator, Optional

from .link import NOT_OPEN, FerroError, Link

_bound: "contextvars.ContextVar[Optional[Link]]" = contextvars.ContextVar("ferro_link", default=None)
_default: Optional[Link] = None


def set_default(link: Optional[Link]) -> None:
    """Make ``link`` the target of ``ferro.tools`` calls outside any Document."""
    global _default
    _default = link


def current() -> Link:
    """The link in effect. Raises FerroError ``ferro.link.not_open`` if none."""
    link = _bound.get() or _default
    if link is None:
        raise FerroError(NOT_OPEN, "not connected: call ferro.connect() first")
    return link


@contextlib.contextmanager
def bound(link: Link) -> Iterator[Link]:
    """Route tool calls in this block (and this thread / task) to ``link``."""
    token = _bound.set(link)
    try:
        yield link
    finally:
        _bound.reset(token)


def call(tool: str, args: Dict[str, Any]) -> Any:
    """Call ``tool`` on the current link (used by the generated ``_tools``)."""
    return current().call(tool, args)
