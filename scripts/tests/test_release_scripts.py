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
VERSION = "2.0.0-beta-1234"

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
        previous = os.environ.get("PREVIOUS_TAG")
        if not previous:
            sys.exit(1)
        print(previous)
    else:
        sys.exit("unexpected git arguments: " + repr(args))
elif name == "gh":
    if args[:2] == ["release", "view"]:
        channel = args[2] == "beta-2.x"
        state = os.environ.get("CHANNEL_STATE" if channel else "RELEASE_STATE", "missing")
        if state == "missing":
            sys.exit(1)
        print(json.dumps({"isDraft": state == "draft", "isPrerelease": state != "stable",
                          "assets": [{"name": "update.json"}] if os.environ.get("CHANNEL_VERSION") else [],
                          "targetCommitish": os.environ["GITHUB_SHA"]}))
    elif args[:2] == ["release", "download"]:
        if os.environ.get("DOWNLOAD_EXIT"):
            sys.exit(1)
        version = os.environ["CHANNEL_VERSION"] if args[2] == "beta-2.x" else args[2][1:]
        directory = Path(args[args.index("--dir") + 1])
        metadata = {"schema": 1, "version": version, "platforms": {
            "macos-" + platform: {"url": f"https://github.com/example/muxy/releases/download/v{version}/Muxy-{version}-{arch}.dmg", "size": len(arch)}
            for platform, arch in [("aarch64", "arm64"), ("x86_64", "x86_64")]
        }}
        (directory / "update.json").write_text(json.dumps(metadata))
    elif args[:2] == ["release", "upload"]:
        sys.exit(int(os.environ.get("UPLOAD_EXIT", "0")))
    elif args[0] == "api":
        previous = next(arg.split("=", 1)[1] for arg in args if arg.startswith("previous_tag_name="))
        print("Generated changes since " + previous)
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

    def version_release_calls(self):
        return [call for call in self.calls("gh")
                if call[1] == "release" and call[2] != "download" and call[3] == f"v{VERSION}"]

    def publish(self):
        return self.run_script("publish-beta.sh", VERSION, self.directory)

    def test_publishes_both_architectures_at_exact_sha_without_latest(self):
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.version_release_calls()
        self.assertEqual([call[2] for call in calls], ["view", "create", "upload", "edit"])
        for call in (calls[1], calls[3]):
            self.assertEqual(call[3], f"v{VERSION}")
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
        notes = (self.directory / "release-notes.md").read_text()
        self.assertIn("Rust/GPUI beta", notes)
        self.assertIn("Muxy Beta.app", notes)
        self.assertIn("Library/Application Support/Muxy Beta", notes)
        self.assertNotIn("alpha", notes.lower())

    def test_alpha_versions_are_rejected_before_publishing(self):
        result = self.run_script("publish-beta.sh", "2.0.0-alpha-1234", self.directory)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("release version must be 2.0.0-beta-", result.stderr)
        self.assertEqual(self.calls("gh"), [])

    def test_generated_notes_continue_from_previous_beta_or_alpha(self):
        for previous in ("v2.0.0-beta-1233", "v2.0.0-alpha-1233"):
            with self.subTest(previous=previous):
                self.log.unlink(missing_ok=True)
                self.env["PREVIOUS_TAG"] = previous
                result = self.publish()
                self.assertEqual(result.returncode, 0, result.stderr)
                describe = next(call for call in self.calls("git") if "describe" in call)
                self.assertIn("v2.0.0-beta-*", describe)
                self.assertIn("v2.0.0-alpha-*", describe)
                api = next(call for call in self.calls("gh") if call[1] == "api")
                self.assertIn("repos/example/muxy/releases/generate-notes", api)
                self.assertIn(f"tag_name=v{VERSION}", api)
                self.assertIn(f"previous_tag_name={previous}", api)
                self.assertIn("Generated changes since", (self.directory / "release-notes.md").read_text())

    def test_published_rerun_does_not_replace_assets(self):
        self.env.update(RELEASE_STATE="published", TAG_SHA=SHA)
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[2] for call in self.version_release_calls()], ["view"])

    def test_draft_rerun_resumes_upload_and_publish(self):
        self.env.update(RELEASE_STATE="draft")
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([call[2] for call in self.version_release_calls()], ["view", "upload", "edit"])

    def test_feed_is_promoted_only_after_versioned_assets_are_published(self):
        self.assertEqual(self.publish().returncode, 0)
        calls = self.calls("gh")
        published = next(i for i, call in enumerate(calls) if call[2:4] == ["edit", f"v{VERSION}"])
        promoted = next(i for i, call in enumerate(calls) if call[2:4] == ["upload", "beta-2.x"])
        self.assertLess(published, promoted)
        metadata = json.loads((self.directory / "update.json").read_text())
        self.assertEqual(metadata["version"], VERSION)
        self.assertEqual(set(metadata["platforms"]), {"macos-aarch64", "macos-x86_64"})
        self.assertEqual(metadata["platforms"]["macos-aarch64"]["size"], 5)
        for call in calls:
            if call[2] in ("create", "edit"):
                self.assertIn("--prerelease", call)
                self.assertIn("--latest=false", call)

    def test_older_finishing_build_does_not_roll_back_the_feed(self):
        self.env.update(CHANNEL_STATE="published", CHANNEL_VERSION="2.0.0-beta-1235")
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(call[2:4] == ["upload", "beta-2.x"] for call in self.calls("gh")))

    def test_newer_build_replaces_existing_feed(self):
        self.env.update(CHANNEL_STATE="published", CHANNEL_VERSION="2.0.0-beta-999")
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(any(call[2:4] == ["upload", "beta-2.x"] for call in self.calls("gh")))

    def test_published_rerun_resumes_feed_promotion_from_published_metadata(self):
        self.env.update(RELEASE_STATE="published", TAG_SHA=SHA)
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls("gh")
        self.assertTrue(any(call[2:4] == ["download", f"v{VERSION}"] for call in calls))
        self.assertTrue(any(call[2:4] == ["upload", "beta-2.x"] for call in calls))
        self.assertFalse(any(call[2:4] == ["upload", f"v{VERSION}"] for call in calls))

    def test_stable_channel_and_failed_metadata_download_are_never_promoted(self):
        for override in ({"CHANNEL_STATE": "stable"}, {"DOWNLOAD_EXIT": "1"}):
            with self.subTest(override=override):
                self.log.unlink(missing_ok=True)
                before = self.env.copy()
                self.env.update(override)
                self.assertNotEqual(self.publish().returncode, 0)
                self.assertFalse(any(call[2:4] == ["upload", "beta-2.x"] for call in self.calls("gh")))
                self.env = before

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
