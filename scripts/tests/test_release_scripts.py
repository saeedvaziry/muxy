import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SHA = "a" * 40
VERSION = "2.0.0-alpha-1234"

FAKE_TOOL = r'''
import json, os, sys
from pathlib import Path
name = Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["TOOL_LOG"], "a") as log:
    log.write(json.dumps([name, *args]) + "\n")
if name == "git":
    if args[0] == "-C":
        args = args[2:]
    if args == ["rev-parse", "HEAD"]:
        print(os.environ["GITHUB_SHA"])
    elif args == ["rev-parse", "--is-shallow-repository"]:
        print("false")
    elif args == ["rev-list", "--count", "HEAD"]:
        print("1234")
    elif args[0] == "fetch":
        sys.exit(int(os.environ.get("FETCH_EXIT", "0")))
    elif args[0] == "merge-base":
        sys.exit(int(os.environ.get("ANCESTOR_EXIT", "0")))
    elif args[0] == "show-ref":
        sys.exit(0 if os.environ.get("TAG_SHA") else 1)
    elif args[0] == "rev-parse":
        print(os.environ["TAG_SHA"])
    elif args[0] == "describe":
        sys.exit(1)
    else:
        sys.exit("unexpected git arguments: " + repr(args))
elif name == "gh":
    if args[:2] == ["release", "view"]:
        state = os.environ.get("RELEASE_STATE", "missing")
        if state == "missing":
            sys.exit(1)
        print(json.dumps({"isDraft": state == "draft", "isPrerelease": state != "stable",
                          "targetCommitish": os.environ["GITHUB_SHA"]}))
    elif args[:2] == ["release", "upload"]:
        sys.exit(int(os.environ.get("UPLOAD_EXIT", "0")))
    elif args[:2] not in (["release", "create"], ["release", "edit"]):
        sys.exit("unexpected gh arguments: " + repr(args))
elif name == "xcrun":
    if args[:2] == ["notarytool", "submit"]:
        print(json.dumps({"id": "test-submission", "status": os.environ.get("NOTARY_STATUS", "Accepted")}))
        sys.exit(int(os.environ.get("NOTARY_EXIT", "0")))
    elif args[:2] == ["notarytool", "log"]:
        print("{}")
    elif args[0] != "stapler":
        sys.exit("unexpected xcrun arguments: " + repr(args))
elif name == "spctl":
    sys.exit(int(os.environ.get("SPCTL_EXIT", "0")))
else:
    sys.exit("unexpected tool: " + name)
'''


class ReleaseScriptTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.tools = self.directory / "tools"
        self.tools.mkdir()
        for name in ("git", "gh", "xcrun", "spctl"):
            tool = self.tools / name
            tool.write_text(f"#!{sys.executable}\n" + FAKE_TOOL)
            tool.chmod(0o755)
        self.log = self.directory / "tools.jsonl"
        self.env = {
            **os.environ,
            "PATH": str(self.tools) + os.pathsep + os.environ["PATH"],
            "TOOL_LOG": str(self.log),
            "GITHUB_REPOSITORY": "example/muxy",
            "GITHUB_SHA": SHA,
            "GITHUB_REF": "refs/heads/2.x",
            "APPLE_ID": "test@example.invalid",
            "APPLE_APP_SPECIFIC_PASSWORD": "test-password",
            "APPLE_TEAM_ID": "test-team",
        }
        for arch in ("arm64", "x86_64"):
            (self.directory / f"Muxy-{VERSION}-{arch}.dmg").write_bytes(arch.encode())

    def run_script(self, script, *args):
        return subprocess.run(
            ["bash", str(ROOT / "scripts" / script), *map(str, args)],
            env=self.env, capture_output=True, text=True,
        )

    def calls(self, tool):
        if not self.log.exists():
            return []
        return [entry for line in self.log.read_text().splitlines()
                if (entry := json.loads(line))[0] == tool]

    def publish(self):
        return self.run_script("publish-alpha.sh", VERSION, self.directory)

    def test_publishes_both_architectures_at_exact_sha_without_latest(self):
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls("gh")
        self.assertEqual([call[2] for call in calls], ["view", "create", "upload", "edit"])
        for call in (calls[1], calls[3]):
            self.assertEqual(call[call.index("--target") + 1], SHA)
            self.assertIn("--prerelease", call)
            self.assertIn("--latest=false", call)
        for arch in ("arm64", "x86_64"):
            filename = f"Muxy-{VERSION}-{arch}.dmg"
            self.assertIn(filename, calls[2])
            self.assertIn(hashlib.sha256(arch.encode()).hexdigest(),
                          (self.directory / "SHA256SUMS").read_text())
        self.assertIn("--draft", calls[1])
        self.assertIn("--draft=false", calls[3])

    def test_published_rerun_does_not_replace_assets(self):
        self.env.update(RELEASE_STATE="published", TAG_SHA=SHA)
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[2] for call in self.calls("gh")], ["view"])

    def test_draft_rerun_resumes_upload_and_publish(self):
        self.env.update(RELEASE_STATE="draft")
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[2] for call in self.calls("gh")], ["view", "upload", "edit"])

    def test_upload_failure_leaves_draft_unpublished(self):
        self.env["UPLOAD_EXIT"] = "1"
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertEqual([call[2] for call in self.calls("gh")], ["view", "create", "upload"])

    def test_missing_intel_artifact_prevents_release(self):
        (self.directory / f"Muxy-{VERSION}-x86_64.dmg").unlink()
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertEqual(self.calls("gh"), [])

    def test_wrong_branch_prevents_release(self):
        self.env["GITHUB_REF"] = "refs/heads/main"
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertEqual(self.calls("gh"), [])

    def test_tag_collision_prevents_release(self):
        self.env["TAG_SHA"] = "b" * 40
        result = self.publish()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("different commit", result.stderr)
        self.assertEqual(self.calls("gh"), [])

    def test_rewritten_history_prevents_release(self):
        self.env["ANCESTOR_EXIT"] = "1"
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertEqual(self.calls("gh"), [])

    def test_fetch_failure_is_not_treated_as_a_missing_tag(self):
        self.env["FETCH_EXIT"] = "1"
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertEqual(self.calls("gh"), [])

    def test_stable_release_is_never_modified(self):
        self.env["RELEASE_STATE"] = "stable"
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertEqual([call[2] for call in self.calls("gh")], ["view"])

    def notarize(self):
        return self.run_script("notarize-release.sh", self.directory / f"Muxy-{VERSION}-arm64.dmg")

    def test_accepted_notarization_is_stapled_and_assessed(self):
        result = self.notarize()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[1:3] for call in self.calls("xcrun")], [
            ["notarytool", "submit"], ["notarytool", "log"],
            ["stapler", "staple"], ["stapler", "validate"],
        ])
        self.assertEqual(len(self.calls("spctl")), 1)

    def test_rejected_notarization_with_zero_exit_is_not_stapled(self):
        self.env["NOTARY_STATUS"] = "Invalid"
        self.assertNotEqual(self.notarize().returncode, 0)
        self.assertEqual([call[1] for call in self.calls("xcrun")], ["notarytool", "notarytool"])
        self.assertEqual(self.calls("spctl"), [])

    def test_failed_submission_is_not_stapled(self):
        self.env["NOTARY_EXIT"] = "1"
        self.assertNotEqual(self.notarize().returncode, 0)
        self.assertEqual(self.calls("spctl"), [])

    def test_gatekeeper_failure_fails_notarization_step(self):
        self.env["SPCTL_EXIT"] = "1"
        self.assertNotEqual(self.notarize().returncode, 0)


if __name__ == "__main__":
    unittest.main()
