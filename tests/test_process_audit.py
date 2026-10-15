"""Owned Job Object audit tests; explicitly opt in to local process execution."""
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.name == "nt" and os.environ.get("TN_PROCESS_AUDIT_TESTS") == "1", "requires exclusive process-audit grant")
class ProcessAuditTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("audit", ROOT / "tools/process_audit.py")
        cls.audit = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.audit)

    def test_no_child_process_is_proven_and_arguments_survive_spaces(self):
        with tempfile.TemporaryDirectory(prefix="process audit ") as directory:
            result = self.audit.run([sys.executable, "-c", "import sys; assert sys.argv[1] == 'space value'", "space value"], directory, timeout_seconds=15)
            self.assertEqual(result["total_processes"], 1)
            self.assertEqual(result["exit_code"], 0)
            self.assertTrue(result["assigned_before_resume"])
            self.assertTrue(result["no_child_processes"])
            self.assertGreater(result["peak_job_memory_bytes"], 0)

    def test_short_lived_child_is_counted_after_it_has_exited(self):
        with tempfile.TemporaryDirectory() as directory:
            source = "import subprocess,sys; subprocess.run([sys.executable,'-c','pass'],check=True,creationflags=8)"
            result = self.audit.run([sys.executable, "-c", source], directory, timeout_seconds=15)
            self.assertEqual(result["exit_code"], 0)
            self.assertEqual(result["total_processes"], 2)
            self.assertFalse(result["no_child_processes"])

    def test_timeout_is_scoped_and_recorded(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.audit.run([sys.executable, "-c", "import time; time.sleep(30)"], directory, timeout_seconds=.1)
            self.assertTrue(result["timed_out"])
            self.assertEqual(result["active_processes"], 0)


if __name__ == "__main__":
    unittest.main()
