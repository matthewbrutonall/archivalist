import json
import os
import subprocess
import unittest


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
HELPER = os.path.join(ROOT, "rust-tools", "scripts", "staff_job_projection.rb")
LOCALE = os.path.join(ROOT, "common", "locales", "en.yml")


class JobProjectionTests(unittest.TestCase):
    def test_pdf_job_fields_match_native_contract(self):
        payload = {"entries": [{"record": {
            "uri": "/repositories/4/jobs/1", "job_type": "print_to_pdf_job",
            "job": {"jsonmodel_type": "print_to_pdf_job", "source": "/repositories/4/resources/26"},
            "time_submitted": "2026-11-11 00:19:24 UTC",
        }, "output_files": [1]}]}
        result = subprocess.run(["ruby", HELPER, LOCALE], input=json.dumps(payload),
                                text=True, capture_output=True, check=True,
                                env={**os.environ, "TZ": "UTC"})
        job = json.loads(result.stdout)[0]
        self.assertEqual(job["title"], "Generate PDF")
        self.assertEqual(job["time_submitted"], "2026-11-11 00:19:24 +0000")
        self.assertEqual(job["files"], ["/jobs/1/file/1"])
        self.assertEqual(job["job_data"], [
            "output_file --- /jobs/1/file/1",
            "source --- /repositories/4/resources/26",
        ])

    def test_non_object_job_rejected(self):
        payload = {"entries": [{"record": {"job": "bad"}, "output_files": []}]}
        result = subprocess.run(["ruby", HELPER, LOCALE], input=json.dumps(payload),
                                text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
