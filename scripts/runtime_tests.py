"""Build portable integration-test executables and run them against a given pair."""

import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
SUITES = ("commands", "tui", "lifecycle")


def build(directory, target=None):
    command = ["cargo", "test", "--locked", "--no-run", "--message-format=json",
               "-p", "muxy-cli", "-p", "muxy-server"]
    for suite in SUITES:
        command.extend(["--test", suite])
    if target:
        command.extend(["--target", target])
    output = subprocess.check_output(command, cwd=ROOT, text=True)
    executables = {}
    for line in output.splitlines():
        artifact = json.loads(line)
        if artifact.get("reason") == "compiler-artifact" and artifact.get("executable"):
            name = artifact["target"]["name"]
            if name in SUITES and artifact["target"]["kind"] == ["test"]:
                executables[name] = Path(artifact["executable"])
    if set(executables) != set(SUITES):
        raise ValueError(f"Missing runtime test executables: {sorted(set(SUITES) - executables.keys())}")
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    for name, executable in executables.items():
        shutil.copy2(executable, directory / name)


def run(directory, binary, profile=None, env=None):
    binary = Path(binary).resolve()
    executables = [Path(directory).resolve() / name for name in SUITES]
    for executable in executables:
        if not executable.is_file():
            raise ValueError(f"Missing runtime test executable: {executable}")
    test_env = dict(os.environ if env is None else env)
    test_env.update(MUXY_TEST_RUNTIME=str(binary), MUXY_TEST_SERVER=str(binary.with_name("muxy-server")))
    test_env.pop("MUXY_TEST_SERVER_PROFILE", None)
    if profile:
        test_env["MUXY_TEST_SERVER_PROFILE"] = profile
    for executable in executables:
        executable.chmod(0o755)
        print(f"+ {executable}", flush=True)
        subprocess.run([executable], cwd=ROOT, env=test_env, check=True)
