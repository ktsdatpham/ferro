"""Tests for the SDK functions plugins use (ADR 0009): ``ferro.ask`` and
``doc.selection()``, against the fake Ferro Link server of ``test_link``,
and the plugin template's ``main.py``. The real hosts are tested in
``crates/ferro-mcp/src/plugins/tests.rs``.

Run: python3 -m unittest discover -s python/tests
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import ferro  # noqa: E402
from ferro import _runtime  # noqa: E402
from ferro.link import Link  # noqa: E402
from test_link import FakeFerro  # noqa: E402

SELECTION = {"elements": [
    {"id": 4, "uid": "u4", "category": "walls"},
    {"id": 9, "uid": "u9", "category": "floors"},
]}


class AskTest(unittest.TestCase):
    def tearDown(self):
        _runtime._run = None
        _runtime.set_default(None)

    def test_outside_a_run_ask_returns_the_default(self):
        self.assertEqual(ferro.ask("Height", default="3m"), "3m")
        self.assertIsNone(ferro.ask("Name"))
        self.assertEqual(ferro.ask("Count", default=5), "5")

    def run_with(self, answer):
        f = FakeFerro(answers={"get_user_input": answer}, token="once")
        self.addCleanup(f.close)
        _runtime.start_run(Link(f.port, "once", agent="run_python"))
        return f

    def test_in_a_run_ask_sends_get_user_input(self):
        f = self.run_with(lambda args: {"value": "4.5m"})
        self.assertEqual(ferro.ask("Height", default="3m"), "4.5m")
        self.assertEqual(f.requests[-1]["tool"], "get_user_input")
        self.assertEqual(f.requests[-1]["args"], {"prompt": "Height", "default": "3m"})
        self.assertEqual(f.requests[-1]["token"], "once")

    def test_cancel_gives_none(self):
        self.run_with({"value": None})
        self.assertIsNone(ferro.ask("Height", default="3m"))

    def test_the_headless_answer_is_the_default(self):
        # What ferro-mcp / ferro-cli answer without the app.
        f = self.run_with(lambda args: {"value": args.get("default")})
        self.assertEqual(ferro.ask("Height", default="3m"), "3m")
        self.assertIsNone(ferro.ask("Name"))
        self.assertEqual(f.requests[-1]["args"], {"prompt": "Name", "default": None})

    def test_a_refusal_raises(self):
        self.run_with(("error", {"error": "no"}))
        with self.assertRaises(ferro.FerroError):
            ferro.ask("Height")


class SelectionTest(unittest.TestCase):
    def doc(self, answer):
        f = FakeFerro(answers={"get_selection": answer})
        self.addCleanup(f.close)
        return ferro.Document(Link(f.port, "secret"))

    def test_selection_is_a_list_of_elements_and_callable_for_ids(self):
        doc = self.doc(SELECTION)
        sel = doc.selection
        self.assertIsInstance(sel, ferro.Selection)
        self.assertEqual([e.id for e in sel], [4, 9])
        self.assertIsInstance(sel[0], ferro.Element)
        self.assertEqual(doc.selection(), [4, 9])
        self.assertEqual(len(doc.selection), 2)

    def test_headless_selection_is_empty(self):
        doc = self.doc({"elements": []})
        self.assertEqual(doc.selection(), [])
        self.assertEqual(list(doc.selection), [])
        doc = self.doc({})
        self.assertEqual(doc.selection(), [])


class TemplateTest(unittest.TestCase):
    """The template's main.py (written by `ferro_mcp::plugins::create_plugin`)
    compiles and calls only what the SDK has."""

    def test_template_main_compiles(self):
        here = os.path.dirname(os.path.abspath(__file__))
        src = os.path.join(here, "..", "..", "crates", "ferro-mcp", "src", "plugins.rs")
        with open(src, encoding="utf-8") as fh:
            text = fh.read()
        start = text.index('pub const TEMPLATE_MAIN: &str = r#"') + len('pub const TEMPLATE_MAIN: &str = r#"')
        main = text[start:text.index('"#;', start)]
        code = compile(main, "main.py", "exec")
        ns = {}
        exec(code, ns)  # noqa: S102 - defines functions only
        self.assertTrue(callable(ns["box_at_origin"]))
        self.assertTrue(hasattr(ferro, "ask"))
        self.assertTrue(hasattr(ferro.Document, "selection"))


if __name__ == "__main__":
    unittest.main()
