#!/usr/bin/env python3
"""Generate TEST-ONLY signing identities (PKCS#12) for the "Sign with a certificate" tests.

These keys protect nothing: they exist so tests can sign PDFs. They are issued by a throwaway
"PageWeave Test CA" (which no system trusts) and protected with the published password
`pageweave-test`. Never use them for anything else.

  fixtures/keys/test-signer-rsa.p12          RSA-2048, AES-256 (PBES2), signer + CA certificate
  fixtures/keys/test-signer-ecdsa.p12        ECDSA P-256, AES-256 (PBES2), signer + CA certificate
  fixtures/keys/test-signer-rsa-legacy.p12   RSA-2048, legacy 3DES (PBES1), as older tools export
  fixtures/keys/test-signer-expired.p12      RSA-2048, certificate expired in 2020
  fixtures/keys/test-ca.der                  the test CA certificate (public)

Keys are random on every run, so regenerating changes these files (tests do not depend on exact
bytes). Requires the `cryptography` package.

Usage: python3 -I tools/fixtures/gen_test_identity.py
"""
from __future__ import annotations

import datetime as dt
import sys
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, rsa
from cryptography.hazmat.primitives.serialization import pkcs12
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

PASSWORD = b"pageweave-test"
OUT = Path(__file__).resolve().parent.parent.parent / "fixtures" / "keys"


def name(cn: str) -> x509.Name:
    return x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, cn), x509.NameAttribute(NameOID.ORGANIZATION_NAME, "PageWeave Test")])


def cert(subject, issuer, public_key, signing_key, not_before, not_after, ca=False):
    b = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(public_key)
        .serial_number(x509.random_serial_number())
        .not_valid_before(not_before)
        .not_valid_after(not_after)
        .add_extension(x509.BasicConstraints(ca=ca, path_length=0 if ca else None), critical=True)
    )
    if not ca:
        b = b.add_extension(
            x509.KeyUsage(True, True, False, False, False, False, False, False, False), critical=True
        ).add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.EMAIL_PROTECTION]), critical=False)
    return b.sign(signing_key, hashes.SHA256())


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    t0, t1 = dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc), dt.datetime(2036, 1, 1, tzinfo=dt.timezone.utc)
    ca_key = ec.generate_private_key(ec.SECP256R1())
    ca = cert(name("PageWeave Test CA"), name("PageWeave Test CA"), ca_key.public_key(), ca_key, t0, t1, ca=True)
    (OUT / "test-ca.der").write_bytes(ca.public_bytes(serialization.Encoding.DER))
    modern = serialization.BestAvailableEncryption(PASSWORD)
    legacy = (
        serialization.PrivateFormat.PKCS12.encryption_builder()
        .key_cert_algorithm(pkcs12.PBES.PBESv1SHA1And3KeyTripleDESCBC)
        .hmac_hash(hashes.SHA1())
        .build(PASSWORD)
    )
    specs = [
        ("test-signer-rsa.p12", rsa.generate_private_key(65537, 2048), "PageWeave Test Signer", modern, (t0, t1)),
        ("test-signer-ecdsa.p12", ec.generate_private_key(ec.SECP256R1()), "PageWeave Test Signer ECDSA", modern, (t0, t1)),
        ("test-signer-rsa-legacy.p12", rsa.generate_private_key(65537, 2048), "PageWeave Test Signer Legacy", legacy, (t0, t1)),
        (
            "test-signer-expired.p12",
            rsa.generate_private_key(65537, 2048),
            "PageWeave Test Signer Expired",
            modern,
            (dt.datetime(2018, 1, 1, tzinfo=dt.timezone.utc), dt.datetime(2020, 1, 1, tzinfo=dt.timezone.utc)),
        ),
    ]
    for file, key, cn, enc, (nb, na) in specs:
        c = cert(name(cn), ca.subject, key.public_key(), ca_key, nb, na)
        data = pkcs12.serialize_key_and_certificates(cn.encode(), key, c, [ca], enc)
        (OUT / file).write_bytes(data)
        print(f"wrote {file} ({len(data)} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
