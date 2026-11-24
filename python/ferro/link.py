"""Ferro Link client (ADR 0001, D-65): drive the Ferro app open on this computer.

Protocol (see ``crates/ferro-tools/src/link.rs``; ``revit-addin/src/FerroLink.cs``
is the C# twin of this client):

1. The app writes ``$FERRO_HOME/link.json`` (else ``~/.ferro/link.json``) with
   ``{port, token, pid, project, version}`` while it is open.
2. A client connects to ``127.0.0.1:<port>``, sends one JSON line
   ``{token, tool, args, agent: {name, version}}`` and reads one JSON line
   ``{ok, body}``. One request per connection.
3. ``ok: false`` bodies carry ``{error, failures?: [{severity, code, message,
   elements}]}``; they are raised as :class:`FerroError`.

Standard library only.
"""

import json
import os
import socket
from typing import Any, Dict, List, Optional

from . import __version__ as _sdk_version

LINK_FILE = "link.json"

#: Client-side error codes (the server's own failure codes, such as
#: ``ferro.opening.outside_host``, are passed through unchanged).
NOT_OPEN = "ferro.link.not_open"  # no link.json: the app is not running
UNREACHABLE = "ferro.link.unreachable"  # link.json exists but nobody answers
INVALID_TOKEN = "ferro.link.invalid_token"  # the app refused the token
TIMEOUT = "ferro.link.timeout"  # no answer within the timeout
PROTOCOL = "ferro.link.protocol"  # the answer was not one JSON line {ok, body}
TOOL_FAILED = "ferro.tool.failed"  # the tool refused, without a failure code


class FerroError(Exception):
    """A Ferro call that did not succeed.

    Attributes:
        code: The first failure's code (e.g. ``ferro.model.not_found``), or one
            of this module's client codes (``ferro.link.*``, ``ferro.tool.failed``).
        message: Human-readable message from Ferro.
        failures: The server's failure list, ``[{severity, code, message,
            elements}]``; empty when Ferro sent none.
        body: The full error body, for anything else the tool returned.
    """

    def __init__(self, code: str, message: str, failures: Optional[List[Dict[str, Any]]] = None,
                 body: Optional[Dict[str, Any]] = None) -> None:
        self.code = code
        self.message = message
        self.failures = list(failures or [])
        self.body = body or {}
        super().__init__(f"{code}: {message}")


def ferro_dir(home: Optional[str] = None) -> str:
    """The per-user Ferro folder: ``home``, else ``$FERRO_HOME``, else ``~/.ferro``."""
    if home:
        return home
    env = os.environ.get("FERRO_HOME")
    if env:
        return env
    return os.path.join(os.path.expanduser("~"), ".ferro")


def link_path(home: Optional[str] = None) -> str:
    """Path of ``link.json``."""
    return os.path.join(ferro_dir(home), LINK_FILE)


def error_from_body(body: Any) -> FerroError:
    """Turn an ``ok: false`` body into a :class:`FerroError` (mirrors FerroLink.cs)."""
    if not isinstance(body, dict):
        return FerroError(TOOL_FAILED, str(body) or "Ferro refused the request.")
    message = str(body.get("error") or "Ferro refused the request.")
    failures = body.get("failures") if isinstance(body.get("failures"), list) else []
    first = failures[0] if failures and isinstance(failures[0], dict) else None
    if first is not None:
        code = str(first.get("code") or TOOL_FAILED)
        m = str(first.get("message") or "")
        # Hosts often put the same text in `error` and the first failure.
        if m and m not in message and message not in m:
            message = f"{message}: {m}"
        elif m and message in m:
            message = m
    elif message == "invalid token":
        code = INVALID_TOKEN
    else:
        code = TOOL_FAILED
    return FerroError(code, message, failures, body)


class Link:
    """A connection target: the Ferro app found through ``link.json``.

    Each :meth:`call` opens a TCP connection, sends one request line and reads
    one answer line, like the protocol requires.

    Args:
        port: TCP port on 127.0.0.1.
        token: Secret from ``link.json``.
        project: Project name the app reported.
        agent: Name recorded in the document journal for changes made through
            this link.
        timeout: Seconds to wait for each answer (connecting waits at most 1 s
            or ``timeout``, whichever is shorter).
    """

    def __init__(self, port: int, token: str, project: str = "", agent: str = "python",
                 agent_version: str = _sdk_version, timeout: float = 120.0) -> None:
        self.port = int(port)
        self.token = token
        self.project = project
        self.agent = agent
        self.agent_version = agent_version
        self.timeout = timeout

    @classmethod
    def find(cls, home: Optional[str] = None, **kwargs: Any) -> "Link":
        """Read ``link.json``. Raises FerroError ``ferro.link.not_open`` when the
        app is not running (no file) or the file cannot be read."""
        path = link_path(home)
        try:
            with open(path, encoding="utf-8") as f:
                info = json.load(f)
            port = int(info["port"])
            token = str(info["token"])
        except FileNotFoundError:
            raise FerroError(NOT_OPEN, f"Ferro is not open (no {path}). Open the Ferro app first.") from None
        except (OSError, ValueError, KeyError, TypeError) as e:
            raise FerroError(NOT_OPEN, f"cannot read {path}: {e}") from None
        return cls(port, token, str(info.get("project", "")), **kwargs)

    def request(self, tool: str, args: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        """Send one request and return the raw answer ``{ok, body}``."""
        line = json.dumps({
            "token": self.token,
            "tool": tool,
            "args": args or {},
            "agent": {"name": self.agent, "version": self.agent_version},
        }, default=_to_json) + "\n"
        try:
            sock = socket.create_connection(("127.0.0.1", self.port), timeout=min(1.0, self.timeout))
        except socket.timeout:
            raise FerroError(UNREACHABLE, "Ferro is not answering. Is the Ferro app open?") from None
        except OSError as e:
            raise FerroError(UNREACHABLE, f"Ferro is not answering ({e}). Is the Ferro app open?") from None
        try:
            sock.settimeout(self.timeout)
            sock.sendall(line.encode("utf-8"))
            data = b""
            while not data.endswith(b"\n"):
                chunk = sock.recv(65536)
                if not chunk:
                    break
                data += chunk
        except socket.timeout:
            raise FerroError(TIMEOUT, f"no answer from Ferro within {self.timeout:g} s ({tool})") from None
        except OSError as e:
            raise FerroError(UNREACHABLE, f"connection to Ferro failed: {e}") from None
        finally:
            sock.close()
        if not data.strip():
            raise FerroError(PROTOCOL, "Ferro closed the connection without an answer.")
        try:
            resp = json.loads(data.decode("utf-8"))
        except ValueError as e:
            raise FerroError(PROTOCOL, f"Ferro sent an answer that is not JSON: {e}") from None
        if not isinstance(resp, dict) or "ok" not in resp:
            raise FerroError(PROTOCOL, "Ferro's answer has no `ok` field.")
        return resp

    def call(self, tool: str, args: Optional[Dict[str, Any]] = None) -> Any:
        """Call a tool. Returns its body; raises FerroError when ``ok`` is false.

        Pressing Ctrl+C (KeyboardInterrupt) while waiting for ``run_python``
        also stops that script in Ferro (``cancel_python``: it is rolled back),
        then the KeyboardInterrupt goes on as usual.
        """
        try:
            resp = self.request(tool, args)
        except KeyboardInterrupt:
            if tool == "run_python":
                self.cancel_python()
            raise
        body = resp.get("body", {})
        if resp.get("ok") is not True:
            raise error_from_body(body)
        return body

    def cancel_python(self) -> bool:
        """Stop the script this client started with ``run_python`` (Ferro
        rolls it back; its ``run_python`` call fails with
        ``ferro.python.cancelled``). Returns True when a script was stopped,
        False when none was running or Ferro could not be reached. Waits at
        most 5 s."""
        saved = self.timeout
        self.timeout = min(saved, 5.0)
        try:
            body = self.call("cancel_python", {})
        except FerroError:
            return False
        finally:
            self.timeout = saved
        return bool(isinstance(body, dict) and body.get("cancelled"))

    def __repr__(self) -> str:
        return f"Link(port={self.port}, project={self.project!r})"


def _to_json(value: Any) -> Any:
    """JSON fallback: numpy arrays / scalars and other sequences become lists."""
    if hasattr(value, "tolist"):
        return value.tolist()
    if isinstance(value, (tuple, set, frozenset)):
        return list(value)
    raise TypeError(f"cannot send {type(value).__name__} to Ferro")
