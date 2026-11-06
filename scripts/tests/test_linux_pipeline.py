"""Exercise the release scripts against isolated artifacts and suite pins."""

import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]
BINARIES = ("cec-support", "myownmesh", "allmystuff-serve", "amst")
TARGETS = {
    "x86_64-unknown-linux-gnu": "linux-x86_64",
    "aarch64-unknown-linux-gnu": "linux-aarch64",
}


def copy_script(name, root):
    destination = root / "scripts" / name
    destination.parent.mkdir(parents=True, exist_ok=True)
    # Universal newline reading also supports running from a Windows checkout
    # under WSL; Linux checkouts and the fixture both receive shell-safe LF.
    destination.write_text((SCRIPTS / name).read_text(), encoding="utf-8")
    return destination


@unittest.skipUnless(
    sys.platform.startswith("linux") and shutil.which("bash"),
    "Linux packaging requires native executable permissions and Bash",
)
class LinuxPackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="cec linux pipeline ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.script = copy_script("package-linux.sh", self.root)

    def stage_binaries(self, target):
        paths = {}
        for name in BINARIES:
            if name == "cec-support":
                path = self.root / "gui/src-tauri/target" / target / "release" / name
            else:
                path = self.root / "gui/src-tauri/binaries" / f"{name}-{target}"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(f"#!/bin/sh\necho {name}\n".encode())
            path.chmod(0o755)
            paths[name] = path
        return paths

    def package(self, target):
        return subprocess.run(
            ["bash", str(self.script), target],
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=30,
        )

    def test_both_architectures_ship_complete_executable_runtime_and_checksum(self):
        for target, platform in TARGETS.items():
            with self.subTest(target=target):
                paths = self.stage_binaries(target)
                result = self.package(target)
                self.assertEqual(result.returncode, 0, result.stderr)
                archive = self.root / "dist-bin" / f"cec-support-{platform}.tar.gz"
                with tarfile.open(archive, "r:gz") as bundle:
                    self.assertEqual(set(bundle.getnames()), set(BINARIES))
                    for name, source in paths.items():
                        member = bundle.getmember(name)
                        self.assertTrue(member.isfile(), name)
                        self.assertEqual(member.mode & 0o777, 0o755, name)
                        self.assertEqual(bundle.extractfile(member).read(), source.read_bytes())
                checksum = archive.with_name(archive.name + ".sha256").read_text().split()
                self.assertEqual(checksum, [hashlib.sha256(archive.read_bytes()).hexdigest(), archive.name])

    def test_missing_sidecar_prevents_publication(self):
        target = "x86_64-unknown-linux-gnu"
        self.stage_binaries(target)["allmystuff-serve"].unlink()
        result = self.package(target)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing executable:", result.stderr)
        self.assertEqual(list((self.root / "dist-bin").iterdir()), [])

    def test_unsupported_target_is_rejected(self):
        result = self.package("x86_64-pc-windows-msvc")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsupported Linux target:", result.stderr)
        self.assertFalse((self.root / "dist-bin").exists())


class LinuxReleaseGateTests(unittest.TestCase):
    def readiness(self, pin):
        with tempfile.TemporaryDirectory(prefix="cec linux gate ") as temporary:
            root = Path(temporary)
            script = copy_script("linux-release-ready.py", root)
            (root / ".allmystuff-rev").write_text(pin + "\n", encoding="utf-8")
            output = root / "github-output"
            output.write_text("existing=value\n", encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(script)],
                cwd=root,
                env={**os.environ, "GITHUB_OUTPUT": str(output)},
                capture_output=True,
                text=True,
                timeout=30,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            lines = output.read_text().splitlines()
            self.assertEqual(lines[0], "existing=value")
            self.assertEqual(len(lines), 2)
            return lines[1]

    def test_previous_upstream_release_remains_candidate_only(self):
        for pin in ("v0.2.121", "v0.2.122"):
            with self.subTest(pin=pin):
                self.assertEqual(self.readiness(pin), "ready=false")

    def test_minimum_and_newer_stable_upstream_releases_enable_publication(self):
        for pin in ("v0.2.123", "v0.2.124", "v0.3.0", "v1.0.0"):
            with self.subTest(pin=pin):
                self.assertEqual(self.readiness(pin), "ready=true")

    def test_malformed_and_prerelease_pins_do_not_publish(self):
        for pin in ("", "main", "0.2.123", "v0.2", "v0.2.123-rc.1", "v0.3.0-beta", "v0.2.123 extra"):
            with self.subTest(pin=pin):
                self.assertEqual(self.readiness(pin), "ready=false")


if __name__ == "__main__":
    unittest.main()
