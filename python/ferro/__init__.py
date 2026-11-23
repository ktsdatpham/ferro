"""Ferro Python SDK (ADR 0005): script the Ferro app open on this computer.

    import ferro
    doc = ferro.connect()
    walls = doc.create_walls([(0, 0), ("8m", 0), ("8m", "6m"), (0, "6m")], closed=True, height="3m")
    walls[0].add_door(position="2m")
    print([w.refresh().volume_m3 for w in doc.walls])

``ferro.tools`` has one generated function per Ferro tool (the same API as
MCP); ``Document`` and the element wrappers are a small object layer on top.
Pure Python >= 3.9, standard library only (numpy optional, for arrays).
"""

__version__ = "0.0.0"

from typing import Optional  # noqa: E402

from . import _runtime  # noqa: E402
from . import _tools as tools  # noqa: E402
from .document import (  # noqa: E402
    Document,
    Element,
    Floor,
    FloorType,
    Level,
    Material,
    Opening,
    Wall,
    WallType,
    as_array,
)
from .link import FerroError, Link  # noqa: E402

__all__ = [
    "Document",
    "Element",
    "FerroError",
    "Floor",
    "FloorType",
    "Level",
    "Link",
    "Material",
    "Opening",
    "Wall",
    "WallType",
    "as_array",
    "connect",
    "tools",
]


def connect(home: Optional[str] = None, agent: str = "python", timeout: float = 120.0) -> Document:
    """Connect to the open Ferro app and return its :class:`Document`.

    Args:
        home: Ferro folder holding ``link.json`` (default ``$FERRO_HOME``, else
            ``~/.ferro``).
        agent: Name recorded in the journal for changes made by this script.
        timeout: Seconds to wait for each answer.

    Raises:
        FerroError: ``ferro.link.not_open`` when the app is not running,
            ``ferro.link.unreachable`` when it does not answer,
            ``ferro.link.invalid_token`` when it refuses the token.

    The returned document also becomes the default target of ``ferro.tools``.
    """
    link = Link.find(home, agent=agent, timeout=timeout)
    link.call("link_status", {})  # check that the app answers and accepts the token
    _runtime.set_default(link)
    return Document(link)
