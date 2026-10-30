"""Integration checks for status propagation and confidential runner output."""

import contextlib
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run


class RunnerTests(unittest.TestCase):
    def execute(self, mode):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fuzz = root / "fuzz"
            (fuzz / "seeds" / "html").mkdir(parents=True)
            (fuzz / "seeds" / "html" / "example").write_text("<p>seed")
            (fuzz / "Cargo.lock").write_text("version = 4\n")
            binary = root / "bin"
            binary.mkdir()
            cargo = binary / "cargo"
            cargo.write_text(
                "#!/usr/bin/env python3\n"
                "import os, pathlib, sys, time\n"
                "mode = os.environ['FAKE_FUZZ_MODE']\n"
                "if 'metadata' in sys.argv:\n"
                "    sys.exit(43 if mode == 'metadata' else 0)\n"
                "print('PRIVATE-REPRODUCER-MARKER', flush=True)\n"
                "if mode == 'timeout': time.sleep(30)\n"
                "if mode == 'lock': pathlib.Path('fuzz/Cargo.lock').write_text('changed')\n"
                "if mode != 'zero': print('stat::number_of_executed_units: 12')\n"
                "sys.exit(77 if mode == 'crash' else 0)\n"
            )
            cargo.chmod(0o700)
            output = io.StringIO()
            env = {"PATH": str(binary) + os.pathsep + os.environ["PATH"], "FAKE_FUZZ_MODE": mode}
            with patch.dict(os.environ, env), patch.object(run, "WATCHDOG_GRACE", 0):
                with contextlib.redirect_stdout(output):
                    code = run.run_target("html", 1, root=root)
            logs = list((fuzz / "logs").glob("*.log"))
            self.assertEqual(len(logs), 1)
            self.assertEqual(logs[0].stat().st_mode & 0o777, 0o600)
            self.assertNotIn("PRIVATE-REPRODUCER-MARKER", output.getvalue())
            return code, output.getvalue()

    def test_success_requires_executions(self):
        code, output = self.execute("success")
        self.assertEqual(code, 0)
        self.assertIn("executions=12", output)

    def test_crash_preserves_failure(self):
        self.assertEqual(self.execute("crash")[0], 77)

    def test_zero_test_run_fails(self):
        self.assertNotEqual(self.execute("zero")[0], 0)

    def test_lockfile_mutation_fails(self):
        self.assertNotEqual(self.execute("lock")[0], 0)

    def test_dependency_failure_propagates(self):
        self.assertEqual(self.execute("metadata")[0], 43)

    def test_watchdog_terminates_fuzzer(self):
        self.assertEqual(self.execute("timeout")[0], 124)


if __name__ == "__main__":
    unittest.main()
