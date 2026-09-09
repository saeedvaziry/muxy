import importlib.util
import plistlib
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("alpha_release", ROOT / "scripts/alpha_release.py")
alpha = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(alpha)


class AlphaVersionTests(unittest.TestCase):
    def test_version_uses_full_history_count(self):
        with patch.object(alpha, "git", side_effect=["false", "1234"]) as git:
            self.assertEqual(alpha.checkout_version(ROOT), "2.0.0-alpha-1234")
        self.assertEqual(git.call_args_list[-1].args, (ROOT, "rev-list", "--count", "HEAD"))

    def test_shallow_history_is_rejected(self):
        with patch.object(alpha, "git", return_value="true"):
            with self.assertRaisesRegex(ValueError, "full checkout"):
                alpha.checkout_version(ROOT)

    def test_release_versions_reject_other_channels_and_unsafe_paths(self):
        for version in (
            "2.0.0", "2.0.0-alpha-0", "2.0.0-alpha-001", "2.0.0-alpha.1",
            "2.0.0-beta-1", "2.1.0-alpha-1", "2.0.0-alpha-1/../x", "2.0.0-alpha-1\n",
        ):
            with self.subTest(version=version), self.assertRaises(ValueError):
                alpha.build_number(version)
        self.assertEqual(alpha.build_number("2.0.0-alpha-1000"), "1000")

    def test_bundle_identity_and_numeric_apple_versions(self):
        info = plistlib.loads(plistlib.dumps(alpha.bundle_info("2.0.0-alpha-1234")))
        self.assertEqual(info["CFBundleIdentifier"], "com.muxy-alpha.app")
        self.assertEqual(info["CFBundleDisplayName"], "Muxy Alpha")
        self.assertEqual(info["CFBundleExecutable"], "muxy-app")
        self.assertEqual(info["CFBundleShortVersionString"], "2.0.0")
        self.assertEqual(info["CFBundleVersion"], "1234")
        self.assertEqual(info["MuxyVersion"], "2.0.0-alpha-1234")
        self.assertEqual(info["LSMinimumSystemVersion"], "14.0")
        self.assertNotIn("SUFeedURL", info)


class StampTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for source in [ROOT / "Cargo.toml", ROOT / "Cargo.lock", *ROOT.glob("crates/*/Cargo.toml")]:
            destination = self.root / source.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)

    def test_stamp_changes_only_workspace_versions_and_is_repeatable(self):
        before = {name: (self.root / name).read_text() for name in ("Cargo.toml", "Cargo.lock")}
        alpha.stamp_version(self.root, "2.0.0-alpha-1234")
        alpha.stamp_version(self.root, "2.0.0-alpha-1234")
        for name, original in before.items():
            self.assertEqual(
                (self.root / name).read_text(),
                original.replace('version = "2.0.0-alpha-0"', 'version = "2.0.0-alpha-1234"'),
            )

    def test_lockfile_mismatch_does_not_partially_stamp_files(self):
        lock = self.root / "Cargo.lock"
        lock.write_text(lock.read_text().replace('name = "muxy-server"', 'name = "missing-server"'))
        before = {name: (self.root / name).read_bytes() for name in ("Cargo.toml", "Cargo.lock")}
        with self.assertRaisesRegex(ValueError, "muxy-server"):
            alpha.stamp_version(self.root, "2.0.0-alpha-1234")
        for name, original in before.items():
            self.assertEqual((self.root / name).read_bytes(), original)


class BuildArgumentTests(unittest.TestCase):
    def test_invalid_arguments_fail_before_building(self):
        for args in (
            [], ["--arch"], ["--arch", "linux"], ["--unknown"],
            ["--arch", "arm64", "--version", "2.0.0"],
            ["--arch", "arm64", "--version", "2.0.0-alpha-1", "--sign-identity"],
        ):
            result = subprocess.run(
                ["bash", str(ROOT / "scripts/build-release.sh"), *args],
                capture_output=True, text=True,
            )
            with self.subTest(args=args):
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("Building app and server", result.stdout)


if __name__ == "__main__":
    unittest.main()
