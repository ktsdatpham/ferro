"""Object layer over the generated tool functions, in the spirit of the Revit
API (ADR 0005 part 2): ``Document`` → ``Element`` wrappers (``Wall``,
``Floor``, ``Opening``, ``Level``, ...).

Everything here is built from existing tools only (``list_elements``,
``get_element``, ``get_selection``, ``create_*``, ``set_*`` ...); there are no
Python-only capabilities (D-44). Lengths are millimetres unless a name says
otherwise (``volume_m3``, ``length_m`` come from ``list_elements`` as is).
"""

import contextlib
from typing import Any, Dict, Iterator, List, Optional, Sequence, Type

from . import _runtime
from . import _tools as tools
from ._tools import Length, Point
from .link import FerroError, Link


def as_array(values: Any) -> Any:
    """``values`` as a numpy array (numpy is imported only here, when asked).

    Raises ImportError with a hint when numpy is not installed."""
    try:
        import numpy  # noqa: PLC0415 - optional dependency, imported lazily
    except ImportError:
        raise ImportError("numpy is optional; install it to get arrays (pip install numpy)") from None
    return numpy.asarray(values, dtype=float)


class Element:
    """One element of the document, as ``list_elements`` summarises it.

    The summary is read when the wrapper is made; call :meth:`refresh` after
    changes. Attribute names follow the tool JSON (``volume_m3`` etc.).
    """

    def __init__(self, document: "Document", data: Dict[str, Any]) -> None:
        self.document = document
        self.data = dict(data)
        self._detail: Optional[Dict[str, Any]] = None

    # -- identity ---------------------------------------------------------
    @property
    def id(self) -> int:
        """Document-local element id."""
        return int(self.data["id"])

    @property
    def uid(self) -> str:
        """Stable global id (survives save / load, used by Revit / IFC)."""
        return str(self.data.get("uid", ""))

    @property
    def category(self) -> str:
        """Category, e.g. ``walls``, ``floors``, ``doors``, ``levels``, ``forms``."""
        return str(self.data.get("category", ""))

    @property
    def name(self) -> Optional[str]:
        return self.data.get("name")

    @property
    def level_id(self) -> Optional[int]:
        """Id of the element's level, if any."""
        return self.data.get("level")

    @property
    def area_m2(self) -> Optional[float]:
        return self.data.get("area_m2")

    @property
    def volume_m3(self) -> Optional[float]:
        return self.data.get("volume_m3")

    # -- detail -----------------------------------------------------------
    def detail(self) -> Dict[str, Any]:
        """``get_element``: faces, bounding box and parameters (cached until refresh)."""
        if self._detail is None:
            with _runtime.bound(self.document.link):
                self._detail = tools.get_element(element=self.id)
        return self._detail

    @property
    def params(self) -> Dict[str, Any]:
        """Parameters ``{key: {type, value}}`` (lengths mm, angles radians)."""
        return dict(self.detail().get("parameters") or {})

    def faces(self) -> List[Dict[str, Any]]:
        """Faces ``[{face, normal, area_m2, center_mm}]`` (for ``push_pull_face``)."""
        return list(self.detail().get("faces") or [])

    def bounding_box(self, as_numpy: bool = False) -> Any:
        """``{min: [x, y, z], max: [...]}`` in mm, or a 2×3 numpy array; None
        for elements without geometry."""
        bb = self.detail().get("bounding_box_mm")
        if bb is None or not as_numpy:
            return bb
        return as_array([bb["min"], bb["max"]])

    def refresh(self) -> "Element":
        """Re-read the summary (and drop the cached detail)."""
        fresh = self.document._summary(self.id)
        self.data = fresh
        self._detail = None
        return self

    # -- changes ----------------------------------------------------------
    def set_parameter(self, key: str, value: Any, type: Optional[str] = None) -> Dict[str, Any]:
        """Set a parameter (``set_parameter``).

        ``value`` may be the tool's ``{"type": ..., "value": ...}`` object, or a
        plain value with ``type`` given (``"length"`` in mm, ``"angle"`` in
        radians, ``"text"``, ...). Without ``type`` it is inferred: bool →
        ``bool``, int → ``integer``, float → ``number``, str → ``text``, list of
        str → ``tags``. Computed keys are read-only (Ferro refuses them).
        """
        if isinstance(value, dict) and "type" in value and type is None:
            typed = value
        else:
            typed = {"type": type or _infer_type(value), "value": value}
        with _runtime.bound(self.document.link):
            r = tools.set_parameter(element=self.id, key=key, value=typed)
        self._detail = None
        return r

    def set_name(self, name: Optional[str]) -> Dict[str, Any]:
        """Rename (None clears)."""
        with _runtime.bound(self.document.link):
            r = tools.set_name(element=self.id, name=name)
        self.data["name"] = name
        return r

    def move(self, vector: Point) -> Dict[str, Any]:
        """Move by ``vector`` (mm)."""
        with _runtime.bound(self.document.link):
            return tools.move_elements(elements=[self.id], vector=vector)

    def delete(self) -> Dict[str, Any]:
        """Delete this element (``delete_elements``)."""
        with _runtime.bound(self.document.link):
            return tools.delete_elements(elements=[self.id])

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Element) and other.id == self.id and other.document is self.document

    def __hash__(self) -> int:
        return hash(self.id)

    def __repr__(self) -> str:
        return f"{type(self).__name__}(id={self.id}, category={self.category!r}, name={self.name!r})"


class Level(Element):
    """A named elevation."""

    @property
    def elevation_mm(self) -> Optional[float]:
        return self.data.get("elevation_mm")

    def set_elevation(self, elevation: Length) -> Dict[str, Any]:
        """``set_level_elevation``; elements on the level move with it."""
        with _runtime.bound(self.document.link):
            return tools.set_level_elevation(level=self.id, elevation=elevation)


class Wall(Element):
    """A parametric wall (straight or curved)."""

    @property
    def type_id(self) -> Optional[int]:
        return self.data.get("type")

    @property
    def length_m(self) -> Optional[float]:
        return self.data.get("length_m")

    @property
    def height_m(self) -> Optional[float]:
        return self.data.get("height_m")

    @property
    def thickness_m(self) -> Optional[float]:
        return self.data.get("thickness_m")

    def set(self, **changes: Any) -> Dict[str, Any]:
        """``set_wall``: start, end, mid, base_offset, height, top_level,
        top_offset, location_line. Omitted fields keep their value."""
        with _runtime.bound(self.document.link):
            return tools.set_wall(wall=self.id, **changes)

    def set_type(self, wall_type: int) -> Dict[str, Any]:
        """``set_element_type`` with a wall type id."""
        with _runtime.bound(self.document.link):
            return tools.set_element_type(element=self.id, element_type=wall_type)

    def add_door(self, position: Length, **opts: Any) -> "Opening":
        """Place a door (``create_opening``); position = distance from the wall
        start to the door centre. Options: width, height, sill, name."""
        return self._add("door", position, opts)

    def add_window(self, position: Length, **opts: Any) -> "Opening":
        """Place a window (``create_opening``). Options: width, height, sill, name."""
        return self._add("window", position, opts)

    def _add(self, kind: str, position: Length, opts: Dict[str, Any]) -> "Opening":
        with _runtime.bound(self.document.link):
            r = tools.create_opening(wall=self.id, kind=kind, position=position, **opts)
        return self.document._wrap_created(r, Opening)

    def openings(self) -> List["Opening"]:
        """Doors and windows hosted by this wall."""
        return [e for e in self.document.elements() if isinstance(e, Opening) and e.host_id == self.id]


class Floor(Element):
    """A floor slab."""

    @property
    def type_id(self) -> Optional[int]:
        return self.data.get("type")

    @property
    def outline_mm(self) -> Any:
        return self.data.get("outline_mm")

    def set(self, **changes: Any) -> Dict[str, Any]:
        """``set_floor``: points, holes, offset. Omitted fields keep their value."""
        with _runtime.bound(self.document.link):
            return tools.set_floor(floor=self.id, **changes)

    def set_type(self, floor_type: int) -> Dict[str, Any]:
        """``set_element_type`` with a floor type id."""
        with _runtime.bound(self.document.link):
            return tools.set_element_type(element=self.id, element_type=floor_type)


class Opening(Element):
    """A door or window hosted by a wall."""

    @property
    def host_id(self) -> Optional[int]:
        return self.data.get("host")

    @property
    def kind(self) -> Optional[str]:
        """``door`` or ``window``."""
        return self.data.get("kind")

    def set(self, **changes: Any) -> Dict[str, Any]:
        """``set_opening``: position, width, height, sill (mm or with units)."""
        with _runtime.bound(self.document.link):
            return tools.set_opening(opening=self.id, **changes)


class WallType(Element):
    """A wall type (layers listed exterior → interior)."""


class FloorType(Element):
    """A floor type (layers listed top → bottom)."""


class Material(Element):
    """A material."""


_BY_BODY: Dict[str, Type[Element]] = {}


def _wrapper_for(data: Dict[str, Any]) -> Type[Element]:
    # Ferro names the body kind ("wall", "floor_type", ...); older apps did
    # not, so fall back to the summary shape.
    by_body = _BY_BODY.get(str(data.get("body", "")))
    if by_body is not None:
        return by_body
    if "body" in data:
        return Element
    cat = data.get("category")
    if "curve" in data:
        return Wall
    if "outline_mm" in data:
        return Floor
    if "host" in data and "kind" in data:
        return Opening
    if "layers" in data:
        return WallType if cat == "walls" else FloorType
    if cat == "levels":
        return Level
    if cat == "materials":
        return Material
    return Element


_BY_BODY.update({
    "wall": Wall,
    "floor": Floor,
    "opening": Opening,
    "wall_type": WallType,
    "floor_type": FloorType,
    "level": Level,
    "material": Material,
})


def _infer_type(value: Any) -> str:
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, float):
        return "number"
    if isinstance(value, str):
        return "text"
    if isinstance(value, (list, tuple)) and all(isinstance(v, str) for v in value):
        return "tags"
    raise TypeError("cannot infer the parameter type; pass type=...")


class Document:
    """The document open in the Ferro app, reached through Ferro Link.

    Made by :func:`ferro.connect`. Every change is a Command run as one
    Transaction (one undo step) and recorded in the journal with this
    client's agent name; with the app's default autonomy the changes collect
    into one Proposal the user accepts or rejects (D-26).
    """

    def __init__(self, link: Link) -> None:
        self.link = link

    # -- raw access -------------------------------------------------------
    def call(self, tool: str, **args: Any) -> Any:
        """Call any tool by name (None arguments are dropped)."""
        return self.link.call(tool, {k: v for k, v in args.items() if v is not None})

    @contextlib.contextmanager
    def bound(self) -> Iterator["Document"]:
        """Route ``ferro.tools.*`` calls in this block to this document."""
        with _runtime.bound(self.link):
            yield self

    # -- queries ----------------------------------------------------------
    @property
    def project(self) -> str:
        """Project name."""
        with _runtime.bound(self.link):
            return str(tools.list_elements(category="levels").get("project", ""))

    def elements(self, category: Optional[str] = None) -> List[Element]:
        """All elements (``list_elements``), optionally of one category
        (``walls``, ``floors``, ``doors``, ``windows``, ``levels``, ``forms``,
        ``materials`` ...). Wrapped as Wall / Floor / Opening / Level / ...;
        wall and floor types share their instances' category."""
        with _runtime.bound(self.link):
            body = tools.list_elements(category=category)
        return [self._wrap(d) for d in body.get("elements", [])]

    def get(self, id: int) -> Element:
        """The element with this id. Raises FerroError ``ferro.model.not_found``."""
        return self._wrap(self._summary(id))

    @property
    def walls(self) -> List[Element]:
        """Wall instances (parametric ``Wall`` and directly edited walls), not wall types."""
        return [e for e in self.elements("walls") if not isinstance(e, WallType)]

    @property
    def floors(self) -> List[Element]:
        """Floor instances, not floor types."""
        return [e for e in self.elements("floors") if not isinstance(e, FloorType)]

    @property
    def levels(self) -> List[Element]:
        return self.elements("levels")

    @property
    def openings(self) -> List[Element]:
        """Doors and windows."""
        return self.elements("doors") + self.elements("windows")

    @property
    def selection(self) -> List[Element]:
        """What the user has selected in the app (``get_selection``)."""
        with _runtime.bound(self.link):
            body = tools.get_selection()
        return [self._wrap(d) for d in body.get("elements", [])]

    def select(self, elements: Sequence[Any]) -> Dict[str, Any]:
        """Select elements (or ids) in the app (``select_elements``); [] clears."""
        with _runtime.bound(self.link):
            return tools.select_elements(elements=[_id(e) for e in elements])

    @property
    def version(self) -> int:
        """Document version; it increases with every change."""
        with _runtime.bound(self.link):
            return int(tools.get_document_version().get("version", 0))

    # -- creation ---------------------------------------------------------
    def create_level(self, name: str, elevation: Length, **opts: Any) -> Level:
        with _runtime.bound(self.link):
            r = tools.create_level(name=name, elevation=elevation, **opts)
        return self._wrap_created(r, Level)

    def create_wall(self, start: Point, end: Point, **opts: Any) -> Wall:
        """``create_wall``; options: level, wall_type, height, top_level, ... ."""
        with _runtime.bound(self.link):
            r = tools.create_wall(start=start, end=end, **_ids(opts))
        return self._wrap_created(r, Wall)

    def create_walls(self, points: Sequence[Point], closed: bool = False, **opts: Any) -> List[Wall]:
        """``create_walls``: a chain through points (draw outer walls clockwise)."""
        with _runtime.bound(self.link):
            r = tools.create_walls(points=points, closed=closed, **_ids(opts))
        ids = (r.get("result") or {}).get("elements") or []
        with _runtime.bound(self.link):
            known = {d.get("id"): d for d in tools.list_elements(category="walls").get("elements", [])}
        return [Wall(self, known.get(i, {"id": i})) for i in ids]

    def create_floor(self, points: Sequence[Point], **opts: Any) -> Floor:
        """``create_floor`` from a plan outline (points [x, y] in mm)."""
        with _runtime.bound(self.link):
            r = tools.create_floor(points=points, **_ids(opts))
        return self._wrap_created(r, Floor)

    # -- history ----------------------------------------------------------
    def undo(self) -> Dict[str, Any]:
        with _runtime.bound(self.link):
            return tools.undo()

    def redo(self) -> Dict[str, Any]:
        with _runtime.bound(self.link):
            return tools.redo()

    @contextlib.contextmanager
    def transaction(self, name: str) -> Iterator["Document"]:
        """Group a block of changes under a name.

        **Inside** ``run_python`` (ADR 0005 part 2) the whole script is already
        one transaction group in Ferro: all or nothing, one undo step, one
        Proposal for agents. This block is a savepoint of that group
        (``SubTransaction``): when an exception leaves the block, the changes
        made in it are rolled back and the exception propagates; catch it to
        keep the script's other changes. Blocks may nest.

        **Outside** ``run_python`` (a script connected with
        :func:`ferro.connect`) there is no group: each call inside is its own
        Transaction (one undo step) and an exception does not roll back the
        calls already made. With the app's default autonomy all calls from
        this client collect into one Proposal the user accepts or rejects.
        Send the script through ``run_python`` to make it atomic.

        ``name`` documents the block; Ferro does not store it.
        """
        if _runtime.run_link() is not self.link:
            with _runtime.bound(self.link):
                yield self
            return
        self.link.call("begin_transaction", {"name": name})
        try:
            with _runtime.bound(self.link):
                yield self
        except BaseException:
            self.link.call("rollback_transaction", {})
            raise
        self.link.call("commit_transaction", {})

    # -- internals --------------------------------------------------------
    def _summary(self, id: int) -> Dict[str, Any]:
        """One call (``get_element``). Raises ``ferro.model.not_found``."""
        try:
            with _runtime.bound(self.link):
                return tools.get_element(element=id)
        except FerroError as e:
            if e.code == "ferro.model.not_found" or "not found" in e.message:
                raise FerroError("ferro.model.not_found", f"element {id} not found") from e
            raise

    def _wrap(self, data: Dict[str, Any]) -> Element:
        return _wrapper_for(data)(self, data)

    def _lazy(self, id: int, cls: Type[Element]) -> Any:
        try:
            return self._wrap(self._summary(id))
        except FerroError:
            return cls(self, {"id": id})

    def _wrap_created(self, body: Dict[str, Any], cls: Type[Element]) -> Any:
        element = (body.get("result") or {}).get("element")
        if element is None:
            raise FerroError("ferro.tool.failed", "the tool did not return an element id", body=body)
        return self._lazy(int(element), cls)

    def __repr__(self) -> str:
        return f"Document({self.link!r})"


def _id(e: Any) -> int:
    return e.id if isinstance(e, Element) else int(e)


def _ids(opts: Dict[str, Any]) -> Dict[str, Any]:
    """Accept Element wrappers where the tool wants an id (level=lvl)."""
    return {k: (_id(v) if isinstance(v, Element) else v) for k, v in opts.items()}
