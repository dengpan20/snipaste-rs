"""Create a Mac source bundle with LF scripts and executable Unix permissions."""
from pathlib import Path
import hashlib
import stat
import tomllib
import zipfile

root = Path(__file__).resolve().parent.parent
version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
output = root / "dist" / f"yingtie-{version}-macos-arm64-source.zip"
output.parent.mkdir(exist_ok=True)
prefix = f"yingtie-{version}/"
files = [root / name for name in ("Cargo.toml", "Cargo.lock", "README.md", ".gitignore", "Build macOS.command")]
for folder in ("crates", "docs", "scripts", "packaging", ".cargo", ".github"):
    files.extend(p for p in (root / folder).rglob("*") if p.is_file() and "__pycache__" not in p.parts)
text_extensions = {".rs", ".toml", ".md", ".sh", ".ps1", ".command", ".m", ".py", ".yml", ".plist", ".json", ".csv"}

def write_file(archive, name, data, executable=False):
    info = zipfile.ZipInfo(prefix + name)
    info.create_system = 3
    info.external_attr = (stat.S_IFREG | (0o755 if executable else 0o644)) << 16
    info.compress_type = zipfile.ZIP_DEFLATED
    archive.writestr(info, data)

with zipfile.ZipFile(output, "w") as archive:
    for path in sorted(set(files)):
        data = path.read_bytes()
        if path.suffix in text_extensions or path.name == ".gitignore":
            data = data.replace(b"\r\n", b"\n")
        write_file(archive, path.relative_to(root).as_posix(), data, path.suffix in {".sh", ".command"})
    write_file(archive, "Mac 使用说明.md", (root / "docs/macos.md").read_bytes().replace(b"\r\n", b"\n"))

# Inspect the actual archive, not just the source files.
with zipfile.ZipFile(output) as archive:
    assert archive.testzip() is None
    for name in ("Build macOS.command", "scripts/build-macos.sh", "scripts/verify-macos.sh"):
        info = archive.getinfo(prefix + name)
        assert (info.external_attr >> 16) & stat.S_IXUSR
        assert b"\r" not in archive.read(info)
    assert prefix + "crates/platform/src/macos/native.m" in archive.namelist()
    assert archive.read(prefix + "packaging/macos/AppIcon.icns").startswith(b"icns")
    assert archive.read(prefix + "packaging/macos/AppIcon.png").startswith(b"\x89PNG\r\n\x1a\n")
digest = hashlib.sha256(output.read_bytes()).hexdigest()
output.with_suffix(".zip.sha256").write_text(f"{digest}  {output.name}\n", encoding="ascii")
print(f"{output}\n{output.stat().st_size:,} bytes; archive/permissions/LF: PASS\nSHA256: {digest}")
