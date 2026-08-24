#!/usr/bin/env python3
"""Safety tests for the offline Agent update signature helper."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import stat
import tempfile
import unittest


HELPER_PATH = Path(__file__).with_name("sign-agent-update-manifest-v2.py")
SPEC = importlib.util.spec_from_file_location("agent_update_signer", HELPER_PATH)
assert SPEC is not None and SPEC.loader is not None
SIGNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SIGNER)


class SignerOutputSafetyTests(unittest.TestCase):
    def test_outputs_cannot_alias_protected_inputs_or_each_other(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / "agent"
            private_key = root / "private.pem"
            artifact.write_bytes(b"agent")
            private_key.write_bytes(b"key")

            with self.assertRaisesRegex(ValueError, "artifact"):
                SIGNER.validate_output_paths(
                    artifact, private_key, artifact, root / "manifest.sig"
                )
            alias = root / "private-key-hardlink"
            os.link(private_key, alias)
            with self.assertRaisesRegex(ValueError, "private key"):
                SIGNER.validate_output_paths(artifact, private_key, alias, None)
            with self.assertRaisesRegex(ValueError, "different files"):
                SIGNER.validate_output_paths(
                    artifact, private_key, root / "same.sig", root / "same.sig"
                )

    def test_raw_signature_replace_is_atomic_and_owner_only(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "agent.sig"
            output.write_bytes(b"old")
            output.chmod(0o644)
            SIGNER.atomic_private_write(output, b"new-signature")
            self.assertEqual(output.read_bytes(), b"new-signature")
            if os.name != "nt":
                self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
            self.assertEqual(list(output.parent.glob(f".{output.name}.tmp-*")), [])


if __name__ == "__main__":
    unittest.main()
