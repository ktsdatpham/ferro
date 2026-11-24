"""Bootstrap of ``run_python`` (ADR 0005 part 2): runs one script sent by Ferro.

Ferro starts ``python3 -I -u -c <bootstrap>`` with the SDK first on
``sys.path`` and calls :func:`main`. The script source arrives on stdin; the
Ferro Link port and the run's one-time token arrive in the environment
(``FERRO_LINK_PORT``, ``FERRO_LINK_TOKEN``), never in argv. All tool calls of
the script land in the run's transaction group in Ferro: the whole script is
all or nothing.

The script sees ``doc`` (:class:`ferro.Document`), ``tools`` (generated tool
functions) and ``ferro``. A variable ``result`` is returned to the caller as
JSON (written to ``FERRO_RUN_RESULT``). Exit status: 0 when the script ends
normally (or calls ``sys.exit(0)``), 1 on an uncaught exception (traceback on
stderr), the script's own status for ``sys.exit(n)``.
"""

import json
import linecache
import os
import sys
import traceback
from typing import Any, Dict, Optional

from . import _runtime
from .link import Link

#: File name shown in tracebacks for the script's source.
FILENAME = "<run_python>"

#: Exit status reserved by the bootstrap for "interpreter too old".
_EXIT_UNSUPPORTED = 86


def _run_link(environ: Optional[Dict[str, str]] = None) -> Link:
    env = os.environ if environ is None else environ
    return Link(int(env["FERRO_LINK_PORT"]), env["FERRO_LINK_TOKEN"], agent="run_python",
                timeout=600.0)


def _write_result(ns: Dict[str, Any], path: Optional[str]) -> None:
    if not path or "result" not in ns:
        return
    from .link import _to_json  # noqa: PLC0415 - private helper, same package

    try:
        text = json.dumps(ns["result"], default=_to_json)
    except (TypeError, ValueError):
        text = json.dumps(repr(ns["result"]))
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def main(source: Optional[str] = None, environ: Optional[Dict[str, str]] = None) -> int:
    """Run the script; return the process exit status."""
    env = os.environ if environ is None else environ
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, "reconfigure", None)
        if reconfigure is not None:
            # "\n" on every OS: Windows would write "\r\n", and agents
            # compare outputs across machines.
            reconfigure(encoding="utf-8", errors="backslashreplace", newline="\n")
    if source is None:
        source = sys.stdin.buffer.read().decode("utf-8", errors="replace")
    import ferro  # noqa: PLC0415 - the package this module belongs to

    link = _run_link(env)
    _runtime.start_run(link)
    ns: Dict[str, Any] = {
        "__name__": "__main__",
        "__file__": FILENAME,
        "__builtins__": __builtins__,
        "ferro": ferro,
        "tools": ferro.tools,
        "doc": ferro.Document(link),
    }
    linecache.cache[FILENAME] = (len(source), None, source.splitlines(True), FILENAME)
    status = 0
    try:
        exec(compile(source, FILENAME, "exec"), ns)  # noqa: S102 - this is the point
    except SystemExit as e:
        if e.code is None:
            status = 0
        elif isinstance(e.code, int):
            status = e.code
        else:
            sys.stderr.write(f"{e.code}\n")
            status = 1
    except BaseException as e:  # noqa: BLE001 - report everything, then roll back
        # Skip this frame: the traceback starts in the script.
        tb = e.__traceback__.tb_next if e.__traceback__ is not None else None
        traceback.print_exception(type(e), e, tb)
        status = 1
    finally:
        sys.stdout.flush()
        sys.stderr.flush()
    if status == _EXIT_UNSUPPORTED:
        status = 1
    if status == 0:
        _write_result(ns, env.get("FERRO_RUN_RESULT"))
    return status
