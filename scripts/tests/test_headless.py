import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import runtime_tests


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


HEADLESS = module("check-headless")
AUDIT = module("audit-linux")


class HeadlessTests(unittest.TestCase):
    def test_newer_userspace_verification_never_invokes_build_tools(self):
        with patch.object(sys, "argv", ["check-headless.py", "--target", "aarch64-unknown-linux-gnu",
                                       "--runtime-binary", "/tmp/floor-runtime/muxy"]), \
             patch.object(HEADLESS.platform, "system", return_value="Linux"), \
             patch.object(HEADLESS.platform, "machine", return_value="aarch64"), \
             patch.object(HEADLESS.subprocess, "check_output") as build_tool, \
             patch.object(HEADLESS, "verify_runtime") as verify:
            HEADLESS.main()
        build_tool.assert_not_called()
        verify.assert_called_once_with(Path("/tmp/floor-runtime/muxy").resolve(), "aarch64-unknown-linux-gnu")

    def test_runtime_test_export_requires_all_suites_and_copies_exact_executables(self):
        with tempfile.TemporaryDirectory(prefix="runtime tests ") as temporary:
            root = Path(temporary)
            artifacts = [{"reason": "compiler-artifact", "executable": "/unused/bin",
                          "target": {"name": "muxy", "kind": ["bin"]}}]
            for name in runtime_tests.SUITES:
                source = root / name
                source.write_text(name)
                artifacts.append({"reason": "compiler-artifact", "executable": str(source),
                                  "target": {"name": name, "kind": ["test"]}})
            destination = root / "exported"
            with patch.object(runtime_tests.subprocess, "check_output",
                              return_value="\n".join(map(json.dumps, artifacts[:-1]))):
                with self.assertRaisesRegex(ValueError, "lifecycle"):
                    runtime_tests.build(destination)
            self.assertFalse(destination.exists())
            with patch.object(runtime_tests.subprocess, "check_output",
                              return_value="\n".join(map(json.dumps, artifacts))):
                runtime_tests.build(destination, "aarch64-unknown-linux-gnu")
            self.assertEqual(sorted(p.name for p in destination.iterdir()), sorted(runtime_tests.SUITES))
            for name in runtime_tests.SUITES:
                self.assertEqual((destination / name).read_text(), name)

    def test_runtime_tests_use_installed_pair_without_tools_and_propagate_failure(self):
        with tempfile.TemporaryDirectory(prefix="runtime tests ") as temporary:
            root = Path(temporary)
            binary = (root / "installed pair/muxy").resolve()
            for name in runtime_tests.SUITES:
                (root / name).write_text(f"#!{sys.executable}\n" +
                    "import json, os, sys\nfrom pathlib import Path\n" +
                    "with open(os.environ['TEST_LOG'], 'a') as log:\n" +
                    "    log.write(json.dumps([Path(sys.argv[0]).name, os.environ['MUXY_TEST_RUNTIME'], " +
                    "os.environ['MUXY_TEST_SERVER'], os.environ['MUXY_TEST_SERVER_PROFILE']]) + '\\n')\n" +
                    "sys.exit(7 if os.environ.get('FAIL_SUITE') == Path(sys.argv[0]).name else 0)\n")
            log = root / "log"
            env = {"PATH": str(root / "no tools"), "TEST_LOG": str(log)}
            runtime_tests.run(root, binary, profile="beta", env=env)
            self.assertEqual([json.loads(line) for line in log.read_text().splitlines()],
                [[name, str(binary), str(binary.with_name("muxy-server")), "beta"] for name in runtime_tests.SUITES])
            log.unlink()
            with self.assertRaises(subprocess.CalledProcessError) as failure:
                runtime_tests.run(root, binary, profile="beta", env={**env, "FAIL_SUITE": "commands"})
            self.assertEqual(failure.exception.returncode, 7)
            self.assertEqual(len(log.read_text().splitlines()), 1)

    def test_zig_build_uses_baseline_cpu_and_preserves_other_arguments(self):
        with tempfile.TemporaryDirectory(prefix="zig test ") as temporary:
            zig = Path(temporary) / "real zig"
            zig.write_text(f"#!{sys.executable}\nimport json, sys\nprint(json.dumps(sys.argv[1:]))\n")
            zig.chmod(0o755)
            for arguments, expected in [
                (["version"], ["version"]),
                (["build", "-Dother=true"], ["build", "-Dother=true"]),
                (["build", "-Demit-lib-vt=true", "--prefix", "a path"],
                 ["build", "-Demit-lib-vt=true", "--prefix", "a path", "-Dcpu=baseline"]),
            ]:
                output = subprocess.check_output(
                    [ROOT / "scripts/zig/zig", *arguments],
                    env={**os.environ, "MUXY_ZIG": str(zig)}, text=True,
                )
                self.assertEqual(json.loads(output), expected)

    def test_metadata_closure_includes_transitive_build_and_test_packages(self):
        def package(name, *dependencies):
            return {"id": name, "name": name, "dependencies": [{"name": dep} for dep in dependencies]}
        metadata = {"workspace_members": ["muxy-cli", "muxy-server", "runtime", "protocol", "test-helper", "unused"],
                    "packages": [package("muxy-cli", "runtime"), package("muxy-server", "runtime"), package("runtime", "protocol", "test-helper"),
                                 package("protocol"), package("test-helper", "protocol"), package("unused")]}
        self.assertEqual(HEADLESS.packages(metadata), ["muxy-cli", "muxy-server", "protocol", "runtime", "test-helper"])
        bad = copy.deepcopy(metadata)
        bad["packages"][2]["dependencies"].append({"name": "gpui"})
        with self.assertRaisesRegex(ValueError, "desktop dependency"):
            HEADLESS.packages(bad)

    def test_elf_audit_rejects_wrong_architecture_libraries_and_glibc_floor(self):
        header = "Class: ELF64\nData: 2's complement, little endian\nMachine: AArch64\n"
        segments = "[Requesting program interpreter: /lib/ld-linux-aarch64.so.1]"
        dynamic = "0x1 (NEEDED) Shared library: [libc.so.6]\n"
        versions = "Name: GLIBC_2.17\nName: GLIBC_2.35\n"
        linked = "libc.so.6 => /lib/aarch64-linux-gnu/libc.so.6 (0x0)"
        good = [header, segments, dynamic, versions, linked, "aarch64-unknown-linux-gnu"]
        AUDIT.audit(*good)
        loader = good.copy()
        loader[2] += "0x1 (NEEDED) Shared library: [ld-linux-aarch64.so.1]\n"
        AUDIT.audit(*loader)
        for index, value in [
            (0, header.replace("AArch64", "ARM")),
            (0, header.replace("ELF64", "ELF32")),
            (1, "[Requesting program interpreter: /tmp/loader]"),
            (2, dynamic + "0x1 (NEEDED) Shared library: [libghostty-vt.so]\n"),
            (2, dynamic + "0x1 (NEEDED) Shared library: [ld-linux-x86-64.so.2]\n"),
            (2, dynamic + "0x2 (RUNPATH) Library runpath: [/build]"),
            (3, versions + "Name: GLIBC_2.36\n"),
            (3, versions + "Name: GLIBC_PRIVATE\n"),
            (3, ""),
            (4, "libc.so.6 => not found"),
            (4, "libc.so.6 => /build/libc.so.6 (0x0)"),
        ]:
            with self.subTest(index=index, value=value):
                bad = good.copy()
                bad[index] = value
                with self.assertRaises(ValueError):
                    AUDIT.audit(*bad)


if __name__ == "__main__":
    unittest.main()
