#!/usr/bin/env python3
"""Create an offline dual Ed25519 signature bundle for an Agent update.

``signature`` authenticates the artifact bytes for v0.2.14 and older Agents.
``manifest_signature`` authenticates the following canonical v2 input:

The signature input is UTF-8 with LF separators and a mandatory final LF:

INVENQOR-AGENT-UPDATE-MANIFEST-V2
version=<version>
channel=<stable|beta>
os=<linux|windows>
architecture=<x86_64|aarch64>
size=<unsigned base-10 artifact length>
sha256=<lowercase SHA-256 hex>
allow_downgrade=<true|false>

The artifact is bound by its size and SHA-256 digest. The helper performs no
network access and delegates only the Ed25519 private-key operation to OpenSSL.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


DOMAIN = "INVENQOR-AGENT-UPDATE-MANIFEST-V2"
MAX_ARTIFACT_SIZE = 128 * 1024 * 1024
VERSION_NUMBER = r"(?:0|[1-9][0-9]*)"
VERSION_PATTERN = re.compile(rf"{VERSION_NUMBER}\.{VERSION_NUMBER}\.{VERSION_NUMBER}\Z")


def artifact_identity(path: Path) -> tuple[int, str]:
    size = 0
    digest = hashlib.sha256()
    with path.open("rb") as artifact:
        while chunk := artifact.read(1024 * 1024):
            size += len(chunk)
            if size > MAX_ARTIFACT_SIZE:
                raise ValueError("artifact exceeds the server limit of 128 MiB")
            digest.update(chunk)
    if size == 0:
        raise ValueError("artifact is empty")
    return size, digest.hexdigest()


def canonical_message(
    *,
    version: str,
    channel: str,
    os_name: str,
    architecture: str,
    size: int,
    sha256: str,
    allow_downgrade: bool,
) -> bytes:
    lines = (
        DOMAIN,
        f"version={version}",
        f"channel={channel}",
        f"os={os_name}",
        f"architecture={architecture}",
        f"size={size}",
        f"sha256={sha256}",
        f"allow_downgrade={'true' if allow_downgrade else 'false'}",
    )
    return ("\n".join(lines) + "\n").encode("utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact", type=Path, required=True)
    parser.add_argument(
        "--private-key",
        type=Path,
        required=True,
        help="PEM Ed25519 private key accepted by openssl pkeyutl",
    )
    parser.add_argument("--version", required=True)
    parser.add_argument("--channel", choices=("stable", "beta"), default="stable")
    parser.add_argument("--os", dest="os_name", choices=("linux", "windows"), required=True)
    parser.add_argument("--architecture", choices=("x86_64", "aarch64"), required=True)
    parser.add_argument("--allow-downgrade", action="store_true")
    parser.add_argument(
        "--signature-output",
        type=Path,
        help="optional path for the raw artifact-only bridge signature",
    )
    parser.add_argument(
        "--manifest-signature-output",
        type=Path,
        help="optional path for the raw canonical v2 manifest signature",
    )
    return parser.parse_args()


def openssl_sign(private_key: Path, input_path: Path) -> bytes:
    completed = subprocess.run(
        (
            "openssl",
            "pkeyutl",
            "-sign",
            "-rawin",
            "-inkey",
            str(private_key),
            "-in",
            str(input_path),
        ),
        capture_output=True,
        check=False,
        timeout=30,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"OpenSSL could not sign {input_path.name}: {detail}")
    if len(completed.stdout) != 64:
        raise RuntimeError(
            f"OpenSSL returned {len(completed.stdout)} signature bytes; Ed25519 requires 64"
        )
    return completed.stdout


def paths_alias(left: Path, right: Path) -> bool:
    """Return true for the same lexical, symlink, or hard-linked file."""
    try:
        if left.samefile(right):
            return True
    except FileNotFoundError:
        pass
    return left.expanduser().resolve(strict=False) == right.expanduser().resolve(strict=False)


def output_destination(path: Path) -> Path:
    expanded = path.expanduser()
    if not expanded.name:
        raise ValueError(f"signature output is not a file path: {path}")
    parent = expanded.parent.resolve(strict=True)
    if not parent.is_dir():
        raise ValueError(f"signature output parent is not a directory: {parent}")
    return parent / expanded.name


def validate_output_paths(
    artifact: Path,
    private_key: Path,
    signature_output: Path | None,
    manifest_signature_output: Path | None,
) -> tuple[Path | None, Path | None]:
    outputs = [
        ("signature output", signature_output),
        ("manifest signature output", manifest_signature_output),
    ]
    protected = [("artifact", artifact), ("private key", private_key)]
    destinations: list[Path | None] = []
    for label, output in outputs:
        if output is None:
            destinations.append(None)
            continue
        destination = output_destination(output)
        for protected_label, protected_path in protected:
            if paths_alias(output, protected_path) or paths_alias(
                destination, protected_path
            ):
                raise ValueError(f"{label} must not overwrite the {protected_label}")
        destinations.append(destination)
    signature_destination, manifest_destination = destinations
    if (
        signature_destination is not None
        and manifest_destination is not None
        and (
            paths_alias(signature_output, manifest_signature_output)
            or paths_alias(signature_destination, manifest_destination)
        )
    ):
        raise ValueError("the two signature outputs must be different files")
    return signature_destination, manifest_destination


def atomic_private_write(path: Path, contents: bytes) -> None:
    """Replace a raw signature atomically with owner-only file permissions."""
    descriptor, temporary_name = tempfile.mkstemp(
        dir=path.parent, prefix=f".{path.name}.tmp-"
    )
    temporary = Path(temporary_name)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            descriptor = -1
            output.write(contents)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    except BaseException:
        if descriptor >= 0:
            os.close(descriptor)
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass
        raise


def main() -> int:
    args = parse_args()
    if not VERSION_PATTERN.fullmatch(args.version):
        raise ValueError("version must be three numbers, for example 0.2.15")
    if args.os_name == "windows" and args.architecture != "x86_64":
        raise ValueError("Windows Agent releases support x86_64 only")

    signature_output, manifest_signature_output = validate_output_paths(
        args.artifact,
        args.private_key,
        args.signature_output,
        args.manifest_signature_output,
    )

    size, digest = artifact_identity(args.artifact)
    message = canonical_message(
        version=args.version,
        channel=args.channel,
        os_name=args.os_name,
        architecture=args.architecture,
        size=size,
        sha256=digest,
        allow_downgrade=args.allow_downgrade,
    )
    # OpenSSL's Ed25519 one-shot operation needs a seekable input so it can
    # determine the complete message length; stdin pipes are rejected.
    with tempfile.TemporaryDirectory(prefix="invenqor-sign-") as temporary:
        message_path = Path(temporary, "manifest-v2.txt")
        message_path.write_bytes(message)
        # `signature` deliberately remains artifact-only for v0.2.14 and older
        # Agents. `manifest_signature` is what v2-capable Agents use to
        # authenticate metadata including allow_downgrade.
        signature = openssl_sign(args.private_key, args.artifact)
        manifest_signature = openssl_sign(args.private_key, message_path)
    if signature_output is not None:
        atomic_private_write(signature_output, signature)
    if manifest_signature_output is not None:
        atomic_private_write(manifest_signature_output, manifest_signature)

    print(
        json.dumps(
            {
                "version": args.version,
                "channel": args.channel,
                "os": args.os_name,
                "architecture": args.architecture,
                "size": size,
                "sha256": digest,
                "allow_downgrade": args.allow_downgrade,
                "signature_scheme": "ed25519",
                "signature_version": 2,
                "signature": base64.b64encode(signature).decode("ascii"),
                "manifest_signature": base64.b64encode(manifest_signature).decode("ascii"),
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
