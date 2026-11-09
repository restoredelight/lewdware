"""Storage safety checks using tiny real qcow2 disks, not mocked QEMU calls."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import vm_storage as storage


class StorageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name) / "test-vm"
        self.directory.mkdir()
        raw = self.directory / "seed.raw"
        raw.write_bytes(b"baseline" * 65536)
        storage.run("qemu-img", "convert", "-f", "raw", "-O", "qcow2",
                    raw, self.directory / "disk.qcow2")
        raw.unlink()
        (self.directory / "firmware.fd").write_bytes(b"firmware state")
        (self.directory / "tpm.permall").write_bytes(b"TPM state")

    def initialize(self):
        with storage.storage_lock(self.directory):
            storage.initialize(self.directory, ["disk.qcow2"],
                               ["firmware.fd", "tpm.permall"], compress=True)

    def test_reset_restores_disk_firmware_and_tpm_without_changing_base(self):
        # Internal snapshots must not travel into the compact baseline.
        storage.run("qemu-img", "snapshot", "-c", "old", self.directory / "disk.qcow2")
        self.initialize()
        base = self.directory / storage.BASE / "files" / "disk.qcow2"
        digest = hashlib.sha256(base.read_bytes()).digest()
        self.assertNotIn("snapshots", storage.info(base))
        storage.run("qemu-io", "-f", "qcow2", "-c", "write -P 0xab 0 4096",
                    self.directory / "disk.qcow2")
        (self.directory / "firmware.fd").write_bytes(b"changed")
        (self.directory / "tpm.permall").write_bytes(b"changed")
        with storage.storage_lock(self.directory):
            storage.reset(self.directory)
        storage.run("qemu-img", "compare", "-q", base, self.directory / "disk.qcow2")
        self.assertEqual(digest, hashlib.sha256(base.read_bytes()).digest())
        self.assertEqual((self.directory / "firmware.fd").read_bytes(), b"firmware state")
        self.assertEqual((self.directory / "tpm.permall").read_bytes(), b"TPM state")

    def test_rejects_running_vm_and_interrupted_transitions(self):
        pid = self.directory / "test-vm.pid"
        pid.write_text(str(os.getpid()))
        with self.assertRaisesRegex(RuntimeError, "shut down"):
            self.initialize()
        pid.unlink()
        (self.directory / storage.TRANSITION).write_text("{}")
        with self.assertRaisesRegex(RuntimeError, "interrupted"):
            storage.require_stopped(self.directory)

    def test_reset_refuses_replaced_disk(self):
        self.initialize()
        active = self.directory / "disk.qcow2"
        active.unlink()
        storage.run("qemu-img", "create", "-q", "-f", "qcow2", active, "1M")
        digest = hashlib.sha256(active.read_bytes()).digest()
        with self.assertRaisesRegex(RuntimeError, "unmanaged"):
            storage.reset(self.directory)
        self.assertEqual(digest, hashlib.sha256(active.read_bytes()).digest())

    def test_paths_cannot_escape_vm_or_alias_each_other(self):
        with self.assertRaises(ValueError):
            storage.checked_names(self.directory, ["../escape"])
        (self.directory / "link").symlink_to(self.directory / "disk.qcow2")
        with self.assertRaises(ValueError):
            storage.checked_names(self.directory, ["link"])
        with self.assertRaises(ValueError):
            storage.checked_names(self.directory, ["disk.qcow2", "./disk.qcow2"])

    def test_init_keeps_originals_until_guest_verification(self):
        self.initialize()
        manifest = json.loads((self.directory / storage.BASE / "manifest.json").read_text())
        retired = self.directory / manifest["retired"]
        self.assertTrue((retired / "disk.qcow2").is_file())
        self.assertEqual((retired / "tpm.permall").read_bytes(), b"TPM state")
        self.assertFalse((self.directory / storage.TRANSITION).exists())

    def test_interrupted_install_blocks_boot_and_keeps_original(self):
        replace = os.replace
        count = 0

        def fail_after_retiring_source(source, target):
            nonlocal count
            count += 1
            if count == 2:
                raise OSError("simulated interruption")
            replace(source, target)

        with patch.object(storage.os, "replace", side_effect=fail_after_retiring_source):
            with self.assertRaisesRegex(OSError, "simulated"):
                self.initialize()
        journal = json.loads((self.directory / storage.TRANSITION).read_text())
        self.assertTrue((Path(journal["retired"]) / "disk.qcow2").is_file())
        with self.assertRaisesRegex(RuntimeError, "interrupted"):
            storage.ensure_ready(self.directory)

    def test_boot_and_storage_operations_share_an_exclusive_lock(self):
        with storage.storage_lock(self.directory):
            with self.assertRaisesRegex(RuntimeError, "in progress"):
                with storage.storage_lock(self.directory):
                    self.fail("second operation acquired the VM lock")

    def test_missing_managed_disk_cannot_be_booted_as_a_new_empty_guest(self):
        self.initialize()
        (self.directory / "disk.qcow2").unlink()
        with self.assertRaisesRegex(RuntimeError, "managed guest file is missing"):
            storage.ensure_ready(self.directory)


if __name__ == "__main__":
    unittest.main()
