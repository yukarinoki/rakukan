"""Exercise the settings bridge, RPC, real DLL and persisted history on Windows.

python scripts/test_learning_management.py --build-dir C:\rb\release
Uses an isolated APPDATA, LOCALAPPDATA and pipe name; never edits real history.
Leaves its sandbox in .build for inspection. Requires an installed system dictionary.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import time
import uuid


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--dictionary", type=Path, default=Path(os.environ["LOCALAPPDATA"]) / "rakukan/dict/rakukan.dict")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    sandbox = Path(tempfile.mkdtemp(prefix="learning-integration-", dir=root / ".build"))
    install = sandbox / "Local/rakukan"
    roaming = sandbox / "Roaming/rakukan"
    (install / "dict").mkdir(parents=True)
    roaming.mkdir(parents=True)
    shutil.copy2(args.build_dir / "rakukan-engine-host.exe", install)
    shutil.copy2(args.build_dir / "rakukan_engine.dll", install / "rakukan_engine_cpu.dll")
    shutil.copy2(args.dictionary, install / "dict/rakukan.dict")
    env = dict(os.environ, APPDATA=str(roaming.parent), LOCALAPPDATA=str(install.parent),
               USERNAME="rakukan_test_" + uuid.uuid4().hex, USERPROFILE=str(sandbox), HF_HOME=str(sandbox / "hf"))
    host_exe = install / "rakukan-engine-host.exe"
    flags = subprocess.CREATE_NO_WINDOW

    def string(value):
        value = value.encode("utf-8")
        return struct.pack("<Q", len(value)) + value

    seed = struct.pack("<IQ", 2, 2)
    for reading, surface, frequency in [("あるの", "アルノ", 5), ("べつ", "ベツ", 2)]:
        seed += string(reading) + struct.pack("<Q", 1) + string(surface)
        seed += struct.pack("<QfI", int(time.time()), frequency, 0)
    (roaming / "learn_history.bin").write_bytes(seed)

    def request(op, **fields):
        run = subprocess.run([str(host_exe), "--manage-learning"],
                             input=json.dumps(dict(op=op, **fields), ensure_ascii=False).encode("utf-8"),
                             capture_output=True, env=env, timeout=25, creationflags=flags)
        assert run.returncode == 0, run.stderr
        return json.loads(run.stdout)

    def entries():
        reply = request("list")
        assert "error" not in reply, reply
        return {(e["reading"], e["surface"]): e for e in reply["entries"]}

    def start():
        host = subprocess.Popen([str(host_exe)], env=env, creationflags=flags)
        # Wait for the test host's pipe rather than letting the bridge spawn a
        # process that this test would not own/clean up.
        deadline = time.monotonic() + 5
        pipe = "\\\\.\\pipe\\rakukan-engine-" + env["USERNAME"]
        while not os.path.exists(pipe):
            assert host.poll() is None, "test host exited"
            if time.monotonic() >= deadline:
                host.terminate()
                host.wait(timeout=5)
                raise TimeoutError("test pipe did not appear")
            time.sleep(.05)
        return host

    host = start()
    try:
        assert len(entries()) == 2
        assert request("save", reading="あるの", surface="あるの") == {"ok": True}
        listed = entries()
        assert listed[("あるの", "あるの")]["frequency"] > listed[("あるの", "アルノ")]["frequency"]
        assert request("save", original_reading="あるの", original_surface="アルノ", reading="あるの", surface="有るの") == {"ok": True}
        assert ("あるの", "アルノ") not in entries()
        assert "error" in request("save", original_reading="あるの", original_surface="有るの", reading="", surface="bad")
        assert ("あるの", "有るの") in entries()
        assert request("delete", reading="あるの", surface="有るの") == {"ok": True}
        assert ("べつ", "ベツ") in entries()
        host.terminate()
        host.wait(timeout=5)
        host = start()
        assert set(entries()) == {("あるの", "あるの"), ("べつ", "ベツ")}
        print("PASS: real settings bridge -> RPC -> DLL: list, add/prefer, edit, validation, delete, restart persistence")
        print("Sandbox:", sandbox)
    finally:
        if host.poll() is None:
            host.terminate()
            host.wait(timeout=5)


if __name__ == "__main__":
    main()
