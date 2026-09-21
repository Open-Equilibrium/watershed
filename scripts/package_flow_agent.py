"""Package already-built native artifacts; never build, sign, download or publish."""

import argparse
import gzip
import hashlib
import io
import pathlib
import re
import stat
import struct
import tarfile
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[1]
PLATFORMS = ("ubuntu-24.04-x86_64", "macos-26-aarch64")


def check_program(path, platform):
    if not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError(f"not a regular program: {path}")
    with path.open("rb") as source:
        header = source.read(20)
    if platform == "ubuntu-24.04-x86_64":
        matches = (len(header) == 20 and header[:7] == b"\x7fELF\x02\x01\x01"
                   and header[18:20] == struct.pack("<H", 62))
    else:
        matches = header[:8] == struct.pack("<II", 0xFEEDFACF, 0x0100000C)
    if not matches:
        raise ValueError(f"program does not match {platform}: {path}")


def package(binaries, output, platform):
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
        raise ValueError("workspace version is not a package-safe Cargo version")
    sources = {"install.sh": ROOT / "install/install.sh",
               "flow": binaries / "flow", "flow-executor": binaries / "flow-executor",
               "bundle-info": None, "LICENSE": ROOT / "LICENSE"}
    for name in ("flow", "flow-executor"):
        check_program(sources[name], platform)
    name = f"flow-agent-{version}-{platform}"
    # A fresh output directory prevents silently replacing a prepared release.
    output.mkdir()
    archive = output / f"{name}.tar.gz"
    with archive.open("xb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as bundle:
            directory = tarfile.TarInfo(name)
            directory.type = tarfile.DIRTYPE
            directory.mode = 0o755
            bundle.addfile(directory)
            for leaf, path in sources.items():
                entry = tarfile.TarInfo(f"{name}/{leaf}")
                entry.mode = 0o644 if leaf in ("bundle-info", "LICENSE") else 0o755
                with (path.open("rb") if path else io.BytesIO(f"{version}\n{platform}\n".encode())) as source:
                    entry.size = source.seek(0, 2)
                    source.seek(0)
                    bundle.addfile(entry, source)
    with archive.open("rb") as source:
        checksum = hashlib.file_digest(source, "sha256").hexdigest()
    # Emit verification data only after the final archive has closed successfully.
    with (output / "SHA256SUMS").open("x", encoding="ascii", newline="\n") as checksums:
        checksums.write(f"{checksum}  {archive.name}\n")
    print(archive)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=pathlib.Path, required=True)
    parser.add_argument("--platform", choices=PLATFORMS, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True,
                        help="new directory whose parent already exists")
    args = parser.parse_args()
    try:
        package(args.binaries, args.output, args.platform)
    except (OSError, ValueError) as error:
        parser.exit(1, f"flow-package: {error}\n")


if __name__ == "__main__":
    main()
