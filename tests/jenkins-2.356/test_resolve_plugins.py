#!/usr/bin/env python3
"""Regression tests for Jenkins plugin version ordering."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "resolve_plugins", Path(__file__).with_name("resolve_plugins.py")
)
assert SPEC is not None and SPEC.loader is not None
RESOLVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RESOLVER)


class VersionOrderingTests(unittest.TestCase):
    def test_numeric_segment_is_newer_than_qualifier(self) -> None:
        newer = "1087.1089.v2f1b_9a_b_040e4"
        older = "1087.v16065d268466"
        self.assertTrue(RESOLVER.at_least(newer, older))
        self.assertFalse(RESOLVER.at_least(older, newer))

    def test_numeric_segments_compare_numerically(self) -> None:
        self.assertTrue(RESOLVER.at_least("2.356", "2.319.1"))
        self.assertFalse(RESOLVER.at_least("2.99", "2.100"))


if __name__ == "__main__":
    unittest.main()
