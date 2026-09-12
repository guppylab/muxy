import copy
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


HEADLESS = module("check-headless")
AUDIT = module("audit-linux")


class HeadlessTests(unittest.TestCase):
    def test_metadata_closure_includes_transitive_build_and_test_packages(self):
        def package(name, *dependencies):
            return {"id": name, "name": name, "dependencies": [{"name": dep} for dep in dependencies]}
        metadata = {"workspace_members": ["muxy-cli", "runtime", "protocol", "test-helper", "unused"],
                    "packages": [package("muxy-cli", "runtime"), package("runtime", "protocol", "test-helper"),
                                 package("protocol"), package("test-helper", "protocol"), package("unused")]}
        self.assertEqual(HEADLESS.packages(metadata), ["muxy-cli", "protocol", "runtime", "test-helper"])
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
        for index, value in [
            (0, header.replace("AArch64", "ARM")),
            (0, header.replace("ELF64", "ELF32")),
            (1, "[Requesting program interpreter: /tmp/loader]"),
            (2, dynamic + "0x1 (NEEDED) Shared library: [libghostty-vt.so]\n"),
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
