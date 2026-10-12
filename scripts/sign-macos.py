"""Sign Mac bundles with a persistent local identity or a supplied Apple identity.

The local key lives in a dedicated encrypted keychain outside the repository.
It is never installed as a trusted root. The signing keychain is discoverable
only during signing, then removed from the user's search list again.
"""
from pathlib import Path
import argparse
import fcntl
import os
import plistlib
import re
import secrets
import shlex
import subprocess
import sys
import tempfile


def run(arguments):
    result = subprocess.run(arguments, capture_output=True, text=True)
    if result.returncode:
        # Do not include argv: security/openssl arguments contain keychain passwords.
        raise RuntimeError(f"{Path(arguments[0]).name}: {result.stderr.strip()}")
    return result.stdout


def search_list():
    return shlex.split(run(["/usr/bin/security", "list-keychains", "-d", "user"]))


def local_identity():
    directory = Path.home() / "Library/Application Support/Yingtie/Signing"
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    directory.chmod(0o700)
    keychain = directory / "local-signing.keychain-db"
    certificate = directory / "local-signing.pem"
    password_file = directory / "keychain.password"
    with (directory / "creation.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if not keychain.exists():
            password = secrets.token_hex(32)
            descriptor = os.open(password_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "w") as output:
                output.write(password)
            previous = search_list()
            try:
                run(["/usr/bin/security", "create-keychain", "-p", password, str(keychain)])
                run(["/usr/bin/security", "unlock-keychain", "-p", password, str(keychain)])
                run(["/usr/bin/security", "set-keychain-settings", "-lut", "21600", str(keychain)])
                with tempfile.TemporaryDirectory(prefix="yingtie-signing-") as temporary:
                    work = Path(temporary)
                    config = work / "certificate.cnf"
                    config.write_text("""[req]
distinguished_name = dn
x509_extensions = signing
prompt = no
[dn]
CN = Yingtie Local Code Signing
[signing]
basicConstraints = critical,CA:TRUE
keyUsage = critical,digitalSignature,keyCertSign
extendedKeyUsage = codeSigning
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always
""")
                    run(["/usr/bin/openssl", "req", "-new", "-x509", "-newkey", "rsa:3072",
                         "-nodes", "-days", "3650", "-sha256", "-config", str(config),
                         "-keyout", str(work / "private.pem"), "-out", str(certificate)])
                    run(["/usr/bin/openssl", "pkcs12", "-export", "-inkey", str(work / "private.pem"),
                         "-in", str(certificate), "-out", str(work / "identity.p12"),
                         "-passout", f"pass:{password}", "-keypbe", "PBE-SHA1-3DES",
                         "-certpbe", "PBE-SHA1-3DES", "-macalg", "sha1"])
                    run(["/usr/bin/security", "import", str(work / "identity.p12"),
                         "-k", str(keychain), "-P", password, "-T", "/usr/bin/codesign"])
                    run(["/usr/bin/security", "set-key-partition-list", "-S", "apple-tool:,codesign:",
                         "-s", "-k", password, str(keychain)])
            finally:
                if search_list() != previous:
                    run(["/usr/bin/security", "list-keychains", "-d", "user", "-s", *previous])
        if not certificate.exists() or not password_file.exists():
            raise RuntimeError("本机签名文件不完整；请恢复 Yingtie/Signing 的备份后再构建")
        password = password_file.read_text()
        run(["/usr/bin/security", "unlock-keychain", "-p", password, str(keychain)])
        output = run(["/usr/bin/openssl", "x509", "-in", str(certificate), "-noout", "-fingerprint", "-sha1"])
        fingerprint = output.strip().split("=", 1)[1].replace(":", "")
        if not re.fullmatch(r"[0-9A-Fa-f]{40}", fingerprint):
            raise RuntimeError("无法读取本机签名证书指纹")
        return fingerprint, keychain


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path)
    args = parser.parse_args()
    app = args.app.resolve()
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    identifier = info["CFBundleIdentifier"]
    identity = os.environ.get("MACOS_SIGN_IDENTITY") or None
    if identity is None and os.environ.get("GITHUB_ACTIONS") == "true":
        identity = "-"
    command = ["/usr/bin/codesign", "--force", "--identifier", identifier]
    if identity is None:
        identity, keychain = local_identity()
        command += ["--keychain", str(keychain), "--timestamp=none"]
        mode = "持久本机证书"
    elif identity != "-":
        command += ["--options", "runtime", "--timestamp"]
        mode = "指定证书"
    else:
        mode = "ad-hoc（重新构建可能需要再次授权）"
    if mode == "持久本机证书":
        with (keychain.parent / "signing.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            previous = search_list()
            added = str(keychain) not in previous
            try:
                # codesign searches this list while resolving the certificate chain.
                if added:
                    run(["/usr/bin/security", "list-keychains", "-d", "user", "-s", *previous, str(keychain)])
                run([*command, "--sign", identity, str(app)])
            finally:
                if added:
                    current = [path for path in search_list() if path != str(keychain)]
                    run(["/usr/bin/security", "list-keychains", "-d", "user", "-s", *current])
    else:
        run([*command, "--sign", identity, str(app)])
    run(["/usr/bin/codesign", "--verify", "--strict", str(app)])
    print(f"签名通过：{mode}；{app}")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, KeyError) as error:
        print(f"签名失败：{error}", file=sys.stderr)
        sys.exit(1)
