# -*- coding: utf-8 -*-
"""Tests for the Flow Launcher plugin. Run: python3 -m unittest test_main"""

import io
import json
import os
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import main  # noqa: E402

EXE = "C:\\Bahth\\bahth.exe"

SAMPLE = [
    {"path": "D:\\العمل\\فاتورة مارس.pdf", "name": "فاتورة مارس.pdf", "size": 2048,
     "modified": "2024-03-05T14:07:09Z", "is_dir": False},
    {"path": "D:\\العمل\\الفواتير", "name": "الفواتير", "size": None,
     "modified": "", "is_dir": True},
]


def completed(code, out=b"", err=b""):
    return subprocess.CompletedProcess(args=[], returncode=code, stdout=out, stderr=err)


class QueryTests(unittest.TestCase):
    def setUp(self):
        p = mock.patch.object(main.os.path, "isfile", return_value=True)
        p.start()
        self.addCleanup(p.stop)

    def run_query(self, text, proc=None, side_effect=None):
        with mock.patch.object(main.subprocess, "run", return_value=proc, side_effect=side_effect) as run:
            reply = main.handle({"method": "query", "parameters": [text], "settings": {"bahth_path": EXE}})
        return reply, run

    def test_results_shape_and_arabic(self):
        out = ("\ufeff" + json.dumps(SAMPLE, ensure_ascii=False)).encode("utf-8")
        reply, run = self.run_query("فاتورة", completed(0, out))
        args = run.call_args[0][0]
        self.assertEqual(args, [EXE, "-s", "فاتورة", "--json", "-n", "30"])
        kw = run.call_args[1]
        self.assertEqual(kw["timeout"], main.TIMEOUT)
        self.assertEqual(kw["creationflags"], main.CREATE_NO_WINDOW)

        res = reply["result"]
        self.assertEqual(len(res), 2)
        for r in res:
            for key in ("Title", "SubTitle", "IcoPath", "JsonRPCAction", "ContextData"):
                self.assertIn(key, r)
            self.assertEqual(r["JsonRPCAction"]["method"], "open_file")
            self.assertEqual(r["JsonRPCAction"]["parameters"], [r["ContextData"]])
        self.assertEqual(res[0]["Title"], "فاتورة مارس.pdf")
        self.assertTrue(res[0]["SubTitle"].startswith("D:\\العمل\\فاتورة مارس.pdf"))
        self.assertIn("2.0 KB", res[0]["SubTitle"])
        self.assertIn("مجلد", res[1]["SubTitle"])
        self.assertGreater(res[0]["Score"], res[1]["Score"])

    def test_no_results(self):
        reply, _ = self.run_query("xyz", completed(1, b"[]\r\n"))
        self.assertEqual(len(reply["result"]), 1)
        self.assertEqual(reply["result"][0]["Title"], "لا نتائج")
        self.assertNotIn("JsonRPCAction", reply["result"][0])

    def test_missing_index(self):
        hint = "لا يوجد فهرس بعد\r\nNo index yet".encode("utf-8")
        reply, _ = self.run_query("عقد", completed(2, b"", hint))
        title = reply["result"][0]["Title"]
        self.assertIn("فهرس", title)
        self.assertIn("No index", reply["result"][0]["SubTitle"])

    def test_timeout(self):
        reply, _ = self.run_query("عقد", side_effect=subprocess.TimeoutExpired("bahth", 3))
        self.assertIn("timed out", reply["result"][0]["Title"])

    def test_bad_json_is_an_error_not_a_crash(self):
        reply, _ = self.run_query("عقد", completed(0, b"not json"))
        self.assertEqual(len(reply["result"]), 1)
        self.assertIn("IcoPath", reply["result"][0])

    def test_empty_query_does_not_run(self):
        reply, run = self.run_query("   ")
        run.assert_not_called()
        self.assertEqual(len(reply["result"]), 1)


class ExeTests(unittest.TestCase):
    def test_exe_not_found(self):
        with mock.patch.object(main.os.path, "isfile", return_value=False), \
                mock.patch.object(main.subprocess, "run") as run:
            reply = main.handle({"method": "query", "parameters": ["x"], "settings": {"bahth_path": EXE}})
        run.assert_not_called()
        self.assertIn("bahth.exe", reply["result"][0]["Title"])

    def test_exe_lookup_order(self):
        with mock.patch.dict(os.environ, {"BAHTH_EXE": "E:\\env\\bahth.exe", "LOCALAPPDATA": "C:\\L"}):
            self.assertEqual(main.find_exe({"bahth_path": ' "F:\\set\\bahth.exe" '}), "F:\\set\\bahth.exe")
            self.assertEqual(main.find_exe({"bahth_path": ""}), "E:\\env\\bahth.exe")
        with mock.patch.dict(os.environ, {"LOCALAPPDATA": "C:\\L"}, clear=True):
            self.assertEqual(main.find_exe(None), os.path.join("C:\\L", "Programs", "Bahth", "bahth.exe"))


class ActionTests(unittest.TestCase):
    def test_context_menu(self):
        reply = main.handle({"method": "context_menu", "parameters": ["D:\\ملف.txt"]})
        methods = [r["JsonRPCAction"]["method"] for r in reply["result"]]
        self.assertEqual(methods, ["open_folder", "open_file"])
        self.assertEqual(reply["result"][0]["JsonRPCAction"]["parameters"], ["D:\\ملف.txt"])

    def test_open_folder_uses_explorer_select(self):
        with mock.patch.object(main.subprocess, "Popen") as popen:
            self.assertIsNone(main.handle({"method": "open_folder", "parameters": ["D:\\ملف.txt"]}))
        self.assertEqual(popen.call_args[0][0], 'explorer /select,"D:\\ملف.txt"')

    def test_open_file_uses_startfile(self):
        with mock.patch.object(main.os, "startfile", create=True) as sf:
            main.handle({"method": "open_file", "parameters": ["D:\\ملف.txt"]})
        sf.assert_called_once_with("D:\\ملف.txt")

    def test_main_prints_ascii_json(self):
        buf = io.StringIO()
        with mock.patch.object(main.sys, "stdout", buf):
            main.main(["main.py", json.dumps({"method": "query", "parameters": [""]})])
        text = buf.getvalue()
        text.encode("ascii")  # Arabic is \u-escaped
        self.assertIn("result", json.loads(text))


if __name__ == "__main__":
    unittest.main()
