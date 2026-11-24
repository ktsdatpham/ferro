"""Tests for the ``run_python`` side of the SDK (ADR 0005 part 2): the
bootstrap ``ferro._run`` and ``Document.transaction`` as a savepoint of the
run's group. Against the fake Ferro Link server of ``test_link``; the real
host is tested in ``crates/ferro-mcp/src/python.rs``.

Run: python3 -m unittest discover -s python/tests
"""

import io
import json
import os
import shutil
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import ferro  # noqa: E402
from ferro import _run, _runtime  # noqa: E402
from test_link import FakeFerro  # noqa: E402

BOX = {"result": {"element": 7}, "changed": {}, "warnings": []}


class RunTest(unittest.TestCase):
    def setUp(self):
        self.f = FakeFerro(answers={
            "create_box": BOX,
            "begin_transaction": {"depth": 1},
            "commit_transaction": {"depth": 0},
            "rollback_transaction": {"depth": 0},
        }, token="once")
        self.addCleanup(self.f.close)
        self.tmp = tempfile.mkdtemp(prefix="ferro-run-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.env = {
            "FERRO_LINK_PORT": str(self.f.port),
            "FERRO_LINK_TOKEN": "once",
            "FERRO_RUN_RESULT": os.path.join(self.tmp, "result.json"),
        }

    def tearDown(self):
        _runtime._run = None
        _runtime.set_default(None)

    def main(self, source):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            status = _run.main(source, self.env)
        return status, out.getvalue(), err.getvalue()

    def tools(self):
        return [r["tool"] for r in self.f.requests]

    def result(self):
        try:
            with open(self.env["FERRO_RUN_RESULT"], encoding="utf-8") as fh:
                return json.load(fh)
        except FileNotFoundError:
            return None

    # -- bootstrap ----------------------------------------------------------
    def test_script_gets_doc_and_tools_and_returns_result(self):
        status, out, _ = self.main(
            "doc.call('create_box', origin=[0, 0, 0], width=1, depth=1, height=1)\n"
            "print('hi')\nresult = {'n': 1, 'pt': (1, 2)}\n")
        self.assertEqual(status, 0)
        self.assertEqual(out, "hi\n")
        self.assertEqual(self.result(), {"n": 1, "pt": [1, 2]})
        req = self.f.requests[0]
        self.assertEqual(req["tool"], "create_box")
        self.assertEqual(req["token"], "once", "calls carry the one-time token")

    def test_exception_exits_1_with_a_traceback_of_the_script(self):
        status, _, err = self.main("x = 1\nraise KeyError('nope')\n")
        self.assertEqual(status, 1)
        self.assertIn('File "<run_python>", line 2', err)
        self.assertIn("raise KeyError('nope')", err, "source lines are shown")
        self.assertIn("KeyError: 'nope'", err)
        self.assertIsNone(self.result(), "no result after a failure")

    def test_sys_exit(self):
        self.assertEqual(self.main("import sys\nresult = 1\nsys.exit()\n")[0], 0)
        self.assertEqual(self.result(), 1)
        self.assertEqual(self.main("import sys\nsys.exit(4)\n")[0], 4)
        self.assertEqual(self.main("import sys\nsys.exit(86)\n")[0], 1, "86 is reserved")
        status, _, err = self.main("import sys\nsys.exit('bad input')\n")
        self.assertEqual((status, err), (1, "bad input\n"))

    def test_unserialisable_result_becomes_its_repr(self):
        self.main("result = object()\n")
        self.assertTrue(str(self.result()).startswith("<object object"))

    def test_connect_inside_a_run_returns_the_run_document(self):
        status, _, _ = self.main("d = ferro.connect(home='/nonexistent')\nresult = d.link is doc.link\n")
        self.assertEqual(status, 0)
        self.assertIs(self.result(), True)
        self.assertNotIn("link_status", self.tools(), "no link.json lookup inside a run")

    # -- Document.transaction ------------------------------------------------
    def test_transaction_inside_a_run_is_a_savepoint(self):
        self.main("with doc.transaction('walls'):\n"
                  "    doc.call('create_box', origin=[0, 0, 0], width=1, depth=1, height=1)\n")
        self.assertEqual(self.tools(), ["begin_transaction", "create_box", "commit_transaction"])
        self.assertEqual(self.f.requests[0]["args"], {"name": "walls"})

    def test_transaction_rolls_back_its_block_on_an_exception(self):
        status, out, _ = self.main(
            "try:\n"
            "    with doc.transaction('a'):\n"
            "        with doc.transaction('b'):\n"
            "            doc.call('create_box', origin=[0, 0, 0], width=1, depth=1, height=1)\n"
            "            raise ValueError('x')\n"
            "except ValueError:\n"
            "    print('caught')\n")
        self.assertEqual((status, out), (0, "caught\n"))
        self.assertEqual(self.tools(), ["begin_transaction", "begin_transaction", "create_box",
                                        "rollback_transaction", "rollback_transaction"])

    def test_transaction_outside_a_run_sends_no_group_requests(self):
        f = FakeFerro(answers={"create_box": BOX})
        self.addCleanup(f.close)
        doc = ferro.connect(home=f.home)
        with doc.transaction("plain"):
            doc.call("create_box", origin=[0, 0, 0], width=1, depth=1, height=1)
        with self.assertRaises(RuntimeError):
            with doc.transaction("plain"):
                raise RuntimeError("no rollback outside run_python")
        self.assertEqual([r["tool"] for r in f.requests], ["link_status", "create_box"])


if __name__ == "__main__":
    unittest.main()
