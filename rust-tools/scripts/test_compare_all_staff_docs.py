import unittest

import compare_all_staff_docs as c


class StrictTest(unittest.TestCase):
    def diffs(self, a, b):
        out = []
        c.strict(a, b, "d", out)
        return out

    def test_equal(self):
        self.assertEqual(self.diffs({"a": [1, "x"], "b": 2.5}, {"b": 2.5, "a": [1, "x"]}), [])

    def test_bool_int_and_int_float_differ(self):
        self.assertTrue(self.diffs({"a": True}, {"a": 1}))
        self.assertTrue(self.diffs({"a": 1}, {"a": 1.0}))

    def test_missing_extra_value_and_length(self):
        self.assertTrue(self.diffs({"a": 1}, {"a": 1, "b": 2}))
        self.assertTrue(self.diffs({"a": 1}, {"a": 2}))
        self.assertTrue(self.diffs({"a": [1]}, {"a": [1, 2]}))
        self.assertTrue(self.diffs({"a": [1, 2]}, {"a": [2, 1]}))


if __name__ == "__main__":
    unittest.main()
