"""Offline, disk-only quickemu baselines and disposable writable overlays."""

import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import uuid


BASE = ".vmctl-base"
TRANSITION = ".vmctl-transition.json"


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)


def info(path):
    return json.loads(subprocess.check_output(
        ["qemu-img", "info", "--output=json", str(path)], text=True
    ))


@contextmanager
def storage_lock(directory):
    with (directory / ".vmctl-storage.lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError("another VM boot/storage operation is in progress") from None
        yield


def ensure_ready(directory):
    if (directory / TRANSITION).exists():
        raise RuntimeError(
            f"interrupted storage transition: inspect {directory / TRANSITION} before booting"
        )
    manifest = directory / BASE / "manifest.json"
    if manifest.exists():
        for entry in json.loads(manifest.read_text())["files"]:
            if not (directory / entry["name"]).is_file():
                raise RuntimeError(f"managed guest file is missing: {entry['name']}; recover it before booting")


def require_stopped(directory):
    ensure_ready(directory)
    pid_file = directory / f"{directory.name}.pid"
    if pid_file.exists():
        try:
            os.kill(int(pid_file.read_text().strip()), 0)
        except (ProcessLookupError, ValueError):
            pass
        else:
            raise RuntimeError("shut down the VM before changing its storage")
    qmp = directory / f"{directory.name}-qmp.sock"
    if qmp.exists():
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(1)
            try:
                connection.connect(str(qmp))
            except ConnectionRefusedError:
                pass
            else:
                raise RuntimeError("VM QMP socket is live; shut it down first")


def checked_names(directory, names):
    result = []
    for name in names:
        relative = Path(name)
        if (relative.is_absolute() or not relative.parts or ".." in relative.parts
                or any(part.startswith(".vmctl") for part in relative.parts)):
            raise ValueError(f"expected a relative guest file path: {name}")
        path = directory / relative
        if path.is_symlink() or path.resolve() != directory.resolve() / relative:
            raise ValueError(f"symlinked guest paths are unsupported: {name}")
        if not path.is_file():
            raise ValueError(f"missing guest file: {path}")
        result.append(relative.as_posix())
    if len(result) != len(set(result)):
        raise ValueError("duplicate guest file paths")
    return result


def copy_state(source, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)
    target.chmod(target.stat().st_mode | 0o200)


def baseline_file(directory, manifest, name):
    generation = manifest.get("generation")
    if generation is not None and (not isinstance(generation, str)
                                   or len(generation) != 12
                                   or any(c not in "0123456789abcdef" for c in generation)):
        raise ValueError("invalid baseline generation")
    root = directory / BASE
    if generation is not None:
        root = root / "generations" / generation
    return root / "files" / name


def stage_overlays(directory, manifest, staging):
    for entry in manifest["files"]:
        name = entry["name"]
        source = baseline_file(directory, manifest, name)
        target = staging / name
        target.parent.mkdir(parents=True, exist_ok=True)
        if entry["kind"] == "image":
            run("qemu-img", "create", "-q", "-f", "qcow2", "-F", "qcow2",
                "-b", source.resolve(), "-o", "lazy_refcounts=off", target)
        else:
            copy_state(source, target)


def install(directory, manifest, staging, retired, publish_manifest=False):
    # Retain every original until all replacements are installed. If interrupted,
    # the journal blocks vmctl up/reset and names both locations for recovery.
    journal = directory / TRANSITION
    journal.write_text(json.dumps({
        "staging": str(staging), "retired": str(retired),
        "files": [entry["name"] for entry in manifest["files"]],
        "new_manifest": manifest if publish_manifest else None,
    }, indent=2) + "\n")
    for entry in manifest["files"]:
        name = entry["name"]
        previous = retired / name
        previous.parent.mkdir(parents=True, exist_ok=True)
        os.replace(directory / name, previous)
        os.replace(staging / name, directory / name)
    if publish_manifest:
        pending = directory / BASE / "manifest.pending.json"
        pending.write_text(json.dumps(manifest, indent=2) + "\n")
        os.replace(pending, directory / BASE / "manifest.json")
    journal.unlink()
    shutil.rmtree(staging)


def initialize(directory, images, states, compress=True):
    require_stopped(directory)
    if (directory / BASE).exists():
        raise RuntimeError("baseline already exists; use reset for a fresh test run")
    images = checked_names(directory, images)
    states = checked_names(directory, states)
    names = checked_names(directory, images + states)
    token = uuid.uuid4().hex[:12]
    staging = directory / f".vmctl-base-staging-{token}"
    retired = directory / f".vmctl-retired-{token}"
    manifest = {"version": 1, "retired": retired.name, "files": []}
    for name in names:
        source = directory / name
        target = staging / "files" / name
        target.parent.mkdir(parents=True, exist_ok=True)
        if name in images:
            fmt = info(source)["format"]
            args = ["qemu-img", "convert", "-m", "2", "-f", fmt, "-O", "qcow2"]
            if compress:
                args += ["-c", "-o", "compression_type=zstd,lazy_refcounts=off"]
            else:
                args += ["-o", "lazy_refcounts=off"]
            print(f"Converting and verifying {name}", flush=True)
            run(*args, source, target)
            run("qemu-img", "check", "-q", target)
            run("qemu-img", "compare", "-q", "-f", fmt, "-F", "qcow2", source, target)
            kind = "image"
        else:
            copy_state(source, target)
            kind = "state"
        target.chmod(target.stat().st_mode & ~0o222)
        manifest["files"].append({"name": name, "kind": kind})
    (staging / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    staging.rename(directory / BASE)
    overlays = directory / f".vmctl-overlay-staging-{token}"
    overlays.mkdir()
    stage_overlays(directory, manifest, overlays)
    install(directory, manifest, overlays, retired)
    print(f"Baseline ready. Original files retained at {retired}")
    print('For quickemu, set preallocation="metadata" so tiny existing overlays are recognized as installed disks.')
    print("Boot and verify the guest before removing the retired files.")


def verify_managed_images(directory, manifest):
    checked_names(directory, [entry["name"] for entry in manifest["files"]])
    for entry in manifest["files"]:
        if entry["kind"] == "image":
            details = info(directory / entry["name"])
            expected = baseline_file(directory, manifest, entry["name"]).resolve()
            actual = details.get("full-backing-filename")
            if not actual or Path(actual).resolve() != expected:
                raise RuntimeError(f"unmanaged active image: {entry['name']}")


def refresh(directory, compress=True):
    """Publish a new generation without moving or editing existing backing files."""
    require_stopped(directory)
    previous = json.loads((directory / BASE / "manifest.json").read_text())
    if previous["version"] != 1:
        raise RuntimeError("unsupported baseline manifest version")
    verify_managed_images(directory, previous)
    token = uuid.uuid4().hex[:12]
    root = directory / BASE / "generations" / token
    root.mkdir(parents=True)
    (root / "previous-manifest.json").write_text(json.dumps(previous, indent=2) + "\n")
    manifest = {"version": 1, "generation": token,
                "retired": f".vmctl-retired-{token}", "files": previous["files"]}
    for entry in manifest["files"]:
        source = directory / entry["name"]
        target = baseline_file(directory, manifest, entry["name"])
        target.parent.mkdir(parents=True, exist_ok=True)
        if entry["kind"] == "image":
            args = ["qemu-img", "convert", "-m", "2", "-f", "qcow2", "-O", "qcow2"]
            if compress:
                args += ["-c", "-o", "compression_type=zstd,lazy_refcounts=off"]
            print(f"Converting and verifying {entry['name']}", flush=True)
            run(*args, source, target)
            run("qemu-img", "check", "-q", target)
            run("qemu-img", "compare", "-q", "-f", "qcow2", "-F", "qcow2", source, target)
        else:
            copy_state(source, target)
        target.chmod(target.stat().st_mode & ~0o222)
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    staging = directory / f".vmctl-overlay-staging-{token}"
    staging.mkdir()
    stage_overlays(directory, manifest, staging)
    install(directory, manifest, staging, directory / manifest["retired"], publish_manifest=True)
    print(f"Baseline generation {token} ready; previous baselines and overlays retained.")


def reset(directory):
    require_stopped(directory)
    manifest = json.loads((directory / BASE / "manifest.json").read_text())
    if manifest["version"] != 1:
        raise RuntimeError("unsupported baseline manifest version")
    # Refuse to overwrite a disk somebody has replaced outside this workflow.
    verify_managed_images(directory, manifest)
    token = uuid.uuid4().hex[:12]
    staging = directory / f".vmctl-overlay-staging-{token}"
    discarded = directory / f".vmctl-discarded-{token}"
    staging.mkdir()
    stage_overlays(directory, manifest, staging)
    install(directory, manifest, staging, discarded)
    shutil.rmtree(discarded)
    print("Fresh disk overlays and firmware/TPM state restored; test changes discarded.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vm", default=os.environ.get("VMCTL_VM", "windows-11"))
    sub = parser.add_subparsers(dest="command", required=True)
    init = sub.add_parser("init", help="flatten current disks into a verified immutable baseline")
    init.add_argument("--image", action="append", required=True,
                      help="relative writable disk/firmware image path; repeat for every image")
    init.add_argument("--state", action="append", default=[],
                      help="relative raw firmware/TPM file to restore on reset")
    init.add_argument("--no-compress", action="store_true")
    renew = sub.add_parser("refresh", help="retain current guest changes in a new verified baseline generation")
    renew.add_argument("--no-compress", action="store_true")
    discard = sub.add_parser("reset", help="discard all test changes with the VM stopped")
    discard.add_argument("--discard", action="store_true", required=True)
    sub.add_parser("status")
    args = parser.parse_args()
    if Path(args.vm).name != args.vm or args.vm in (".", ".."):
        parser.error("--vm must be a VM name, not a path")
    directory = Path(os.environ.get("VMCTL_DIR", Path.home() / "Machines")) / args.vm
    try:
        with storage_lock(directory):
            if args.command == "init":
                initialize(directory, args.image, args.state, not args.no_compress)
            elif args.command == "reset":
                reset(directory)
            elif args.command == "refresh":
                refresh(directory, not args.no_compress)
            else:
                ensure_ready(directory)
                path = directory / BASE / "manifest.json"
                print(path.read_text() if path.exists() else "No managed baseline")
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"vm-storage: {error}\n")


if __name__ == "__main__":
    main()
