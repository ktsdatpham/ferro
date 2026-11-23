"""Tests for the Ferro Python SDK against a fake Ferro Link server.

Run: python3 -m unittest discover -s python/tests
"""

import json
import os
import shutil
import socket
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

import ferro  # noqa: E402
from ferro import _runtime, link  # noqa: E402


class FakeFerro:
    """A Ferro Link server on 127.0.0.1:0 answering canned JSON.

    ``answers`` maps a tool name to a body (ok), a callable(args) -> body, or
    ("error", body). The token is checked like the app does. Every request is
    recorded in ``requests``. With ``delay`` set, it never answers.
    """

    def __init__(self, answers=None, token="secret", delay=None):
        self.answers = dict(answers or {})
        self.answers.setdefault("link_status", {"linked": True, "project": "Test"})
        self.token = token
        self.delay = delay
        self.requests = []
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(8)
        self.port = self.sock.getsockname()[1]
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self._serve, daemon=True)
        self.thread.start()
        self.home = tempfile.mkdtemp(prefix="ferro-home-")
        with open(os.path.join(self.home, "link.json"), "w", encoding="utf-8") as f:
            json.dump({"port": self.port, "token": token, "pid": 1, "project": "Test", "version": "0"}, f)

    def _serve(self):
        while not self.stop.is_set():
            try:
                conn, _ = self.sock.accept()
            except OSError:
                return
            with conn:
                data = b""
                while not data.endswith(b"\n"):
                    chunk = conn.recv(65536)
                    if not chunk:
                        break
                    data += chunk
                req = json.loads(data)
                self.requests.append(req)
                if self.delay is not None:
                    self.delay.wait(5)
                    continue
                if req.get("token") != self.token:
                    resp = {"ok": False, "body": {"error": "invalid token"}}
                else:
                    a = self.answers.get(req["tool"], {"error": f"unknown tool: {req['tool']}"})
                    if isinstance(a, tuple):
                        resp = {"ok": False, "body": a[1]}
                    elif callable(a):
                        b = a(req["args"])
                        resp = ({"ok": False, "body": b[1]} if isinstance(b, tuple)
                                else {"ok": True, "body": b})
                    elif "error" in a:
                        resp = {"ok": False, "body": a}
                    else:
                        resp = {"ok": True, "body": a}
                try:
                    conn.sendall((json.dumps(resp) + "\n").encode())
                except OSError:
                    pass

    def close(self):
        self.stop.set()
        if self.delay is not None:
            self.delay.set()
        self.sock.close()
        shutil.rmtree(self.home, ignore_errors=True)


class SdkTest(unittest.TestCase):
    def tearDown(self):
        _runtime.set_default(None)

    def fake(self, **kw):
        f = FakeFerro(**kw)
        self.addCleanup(f.close)
        return f

    def test_connect_success(self):
        f = self.fake()
        doc = ferro.connect(home=f.home, agent="test-agent")
        self.assertIsInstance(doc, ferro.Document)
        self.assertEqual(doc.link.port, f.port)
        self.assertEqual(doc.link.project, "Test")
        req = f.requests[0]
        self.assertEqual(req["tool"], "link_status")
        self.assertEqual(req["token"], "secret")
        self.assertEqual(req["agent"]["name"], "test-agent")
        self.assertEqual(req["args"], {})

    def test_missing_link_json(self):
        home = tempfile.mkdtemp(prefix="ferro-empty-")
        self.addCleanup(shutil.rmtree, home, True)
        with self.assertRaises(ferro.FerroError) as cm:
            ferro.connect(home=home)
        self.assertEqual(cm.exception.code, link.NOT_OPEN)

    def test_not_connected(self):
        with self.assertRaises(ferro.FerroError) as cm:
            ferro.tools.undo()
        self.assertEqual(cm.exception.code, link.NOT_OPEN)

    def test_ferro_home_env_is_used(self):
        f = self.fake()
        old = os.environ.get("FERRO_HOME")
        os.environ["FERRO_HOME"] = f.home
        try:
            self.assertEqual(link.link_path(), os.path.join(f.home, "link.json"))
            self.assertEqual(ferro.connect().link.port, f.port)
        finally:
            if old is None:
                del os.environ["FERRO_HOME"]
            else:
                os.environ["FERRO_HOME"] = old

    def test_wrong_token_maps_to_invalid_token(self):
        f = self.fake()
        with open(os.path.join(f.home, "link.json"), "w", encoding="utf-8") as fh:
            json.dump({"port": f.port, "token": "stale"}, fh)
        with self.assertRaises(ferro.FerroError) as cm:
            ferro.connect(home=f.home)
        self.assertEqual(cm.exception.code, link.INVALID_TOKEN)
        self.assertEqual(cm.exception.failures, [])

    def test_unreachable_port(self):
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        s.close()
        with self.assertRaises(ferro.FerroError) as cm:
            link.Link(port, "x").call("link_status")
        self.assertEqual(cm.exception.code, link.UNREACHABLE)

    def test_tool_call_round_trip(self):
        f = self.fake(answers={"list_elements": lambda args: {
            "project": "Test",
            "elements": [{"id": 3, "uid": "u3", "name": None, "category": "walls", "level": 1,
                          "curve": {}, "type": 2, "length_m": 8.0, "height_m": 3.0, "volume_m3": 4.8},
                         {"id": 2, "uid": "u2", "name": "Generic", "category": "walls", "level": None,
                          "thickness_mm": 200.0, "layers": []}],
        }})
        doc = ferro.connect(home=f.home)
        walls = doc.walls
        self.assertEqual(len(walls), 1, "wall types are not walls")
        self.assertIsInstance(walls[0], ferro.Wall)
        self.assertEqual(walls[0].volume_m3, 4.8)
        self.assertEqual(walls[0].uid, "u3")
        self.assertEqual(f.requests[-1]["tool"], "list_elements")
        self.assertEqual(f.requests[-1]["args"], {"category": "walls"})
        # get(id) is one get_element call; the body kind picks the wrapper,
        # and rows without it (older apps) fall back to the summary shape.
        rows = {2: {"id": 2, "uid": "u2", "category": "walls", "body": "wall_type", "layers": []},
                4: {"id": 4, "uid": "u4", "category": "floors", "body": "floor_type"},
                5: {"id": 5, "uid": "u5", "category": "forms", "body": "form"},
                6: {"id": 6, "uid": "u6", "category": "walls", "thickness_mm": 200.0, "layers": []}}
        missing = {"error": "element 99 not found", "failures": [{
            "severity": "error", "code": "ferro.model.not_found",
            "message": "element 99 not found", "elements": [99]}]}
        f.answers["get_element"] = lambda args: rows.get(args["element"]) or ("error", missing)
        n = len(f.requests)
        self.assertIsInstance(doc.get(2), ferro.WallType)
        self.assertEqual([r["tool"] for r in f.requests[n:]], ["get_element"])
        self.assertIsInstance(doc.get(4), ferro.FloorType, "no shape hint needed")
        self.assertIs(type(doc.get(5)), ferro.Element)
        self.assertIsInstance(doc.get(6), ferro.WallType, "fallback without body")
        with self.assertRaises(ferro.FerroError) as cm:
            doc.get(99)
        self.assertEqual(cm.exception.code, "ferro.model.not_found")

    def test_failure_code_and_failures_pass_through(self):
        failure = {"severity": "error", "code": "ferro.opening.outside_host",
                   "message": "the opening does not fit", "elements": [3]}
        f = self.fake(answers={"create_opening": ("error", {
            "error": "command failed; nothing was changed", "failures": [failure]})})
        ferro.connect(home=f.home)
        with self.assertRaises(ferro.FerroError) as cm:
            ferro.tools.create_opening(wall=3, kind="window", position="7.9m")
        e = cm.exception
        self.assertEqual(e.code, "ferro.opening.outside_host")
        self.assertIn("does not fit", e.message)
        self.assertEqual(e.failures, [failure])

    def test_tool_error_without_failures(self):
        f = self.fake(answers={"undo": {"error": "nothing to undo"}})
        ferro.connect(home=f.home)
        with self.assertRaises(ferro.FerroError) as cm:
            ferro.tools.undo()
        self.assertEqual(cm.exception.code, link.TOOL_FAILED)
        self.assertEqual(cm.exception.message, "nothing to undo")

    def test_generated_function_argument_mapping(self):
        f = self.fake(answers={"create_walls": {"result": {"elements": [5, 6]}, "changed": [5, 6]},
                               "create_opening": {"result": {"element": 7}}})
        ferro.connect(home=f.home)
        ferro.tools.create_walls(points=[(0, 0), ("8m", 0, 0)], closed=None, height="3m",
                                 location_line="finish_exterior")
        args = f.requests[-1]["args"]
        self.assertEqual(args, {"points": [[0, 0], ["8m", 0, 0]], "height": "3m",
                                "location_line": "finish_exterior"})
        self.assertNotIn("closed", args, "None arguments are omitted")
        ferro.tools.create_opening(wall=5, kind="door", position=2000, width=None)
        self.assertEqual(f.requests[-1]["args"], {"wall": 5, "kind": "door", "position": 2000})
        # A required argument may be None on purpose (set_name clears the name).
        f.answers["set_name"] = {"changed": [5]}
        ferro.tools.set_name(element=5, name=None)
        self.assertEqual(f.requests[-1]["args"], {"element": 5, "name": None})
        # Keyword-only: positional calls are rejected.
        with self.assertRaises(TypeError):
            ferro.tools.create_opening(5, "door", 2000)  # type: ignore[misc]

    def test_object_layer_calls_generated_tools(self):
        f = self.fake(answers={
            "create_walls": {"result": {"elements": [5]}},
            "list_elements": {"project": "T", "elements": [
                {"id": 5, "uid": "u5", "category": "walls", "curve": {}},
                {"id": 7, "uid": "u7", "category": "doors", "host": 5, "kind": "door"}]},
            "get_element": lambda args: {
                7: {"id": 7, "uid": "u7", "category": "doors", "body": "opening", "host": 5, "kind": "door"},
            }.get(args["element"], {"id": args["element"]}),
            "create_opening": {"result": {"element": 7}},
            "set_parameter": {"changed": [5]},
            "set_name": {"changed": [5]},
        })
        doc = ferro.connect(home=f.home)
        (wall,) = doc.create_walls([(0, 0), (8000, 0)], height="3m")
        self.assertIsInstance(wall, ferro.Wall)
        self.assertEqual(f.requests[1]["args"], {"points": [[0, 0], [8000, 0]], "closed": False, "height": "3m"})
        door = wall.add_door("2m")
        self.assertIsInstance(door, ferro.Opening)
        self.assertEqual(door.host_id, 5)
        self.assertEqual([r["args"] for r in f.requests if r["tool"] == "create_opening"],
                         [{"wall": 5, "kind": "door", "position": "2m"}])
        wall.set_parameter("ferro.common.comments", "hello")
        self.assertEqual(f.requests[-1]["args"]["value"], {"type": "text", "value": "hello"})
        wall.set_parameter("acme.fire.rating_min", 90)
        self.assertEqual(f.requests[-1]["args"]["value"], {"type": "integer", "value": 90})
        with doc.transaction("walls"):
            ferro.tools.set_name(element=5, name="North")
        self.assertEqual(f.requests[-1]["tool"], "set_name")

    def test_timeout(self):
        f = self.fake(delay=threading.Event())
        lk = link.Link(f.port, "secret", timeout=0.3)
        with self.assertRaises(ferro.FerroError) as cm:
            lk.call("list_elements")
        self.assertEqual(cm.exception.code, link.TIMEOUT)

    def test_bad_answer_is_protocol_error(self):
        srv = socket.socket()
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)
        self.addCleanup(srv.close)

        def answer():
            c, _ = srv.accept()
            c.recv(65536)
            c.sendall(b"not json\n")
            c.close()

        threading.Thread(target=answer, daemon=True).start()
        with self.assertRaises(ferro.FerroError) as cm:
            link.Link(srv.getsockname()[1], "x", timeout=2).call("undo")
        self.assertEqual(cm.exception.code, link.PROTOCOL)

    def test_every_tool_has_a_function(self):
        with open(os.path.join(os.path.dirname(ferro.__file__), "tools.json"), encoding="utf-8") as fh:
            names = [t["name"] for t in json.load(fh)["tools"]]
        self.assertEqual(list(ferro.tools.TOOL_NAMES), names)
        for n in names:
            self.assertTrue(callable(getattr(ferro.tools, n)), n)


if __name__ == "__main__":
    unittest.main()
