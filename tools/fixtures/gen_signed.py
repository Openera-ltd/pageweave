#!/usr/bin/env python3
"""Generate digitally signed PDF fixtures for signature verification tests.

A throwaway test CA and signer keys are created in memory for each run and never written to disk;
only the signed PDFs are kept (they contain public certificates only). Signatures are detached CMS
(adbe.pkcs7.detached) with signed attributes, made by the `cryptography` package, independent of
the engine under test (decision D-004).

  signed-rsa.pdf          RSA-2048 / SHA-256, untouched
  signed-ecdsa.pdf        ECDSA P-256 / SHA-256, untouched
  signed-then-edited.pdf  signed-rsa.pdf plus an incremental update that adds a note afterwards
  signed-tampered.pdf     signed-rsa.pdf with one byte of page text changed inside the signed range

Usage: python3 -I tools/fixtures/gen_signed.py [OUT_DIR]
"""
from __future__ import annotations

import datetime as dt
import importlib.util
import sys
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, rsa
from cryptography.hazmat.primitives.serialization import pkcs7
from cryptography.x509.oid import NameOID

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("gen_fixtures", HERE / "gen_fixtures.py")
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)

CONTENTS_BYTES = 8192  # room for the CMS blob (hex doubles it)
BYTE_RANGE_PLACEHOLDER = "/ByteRange [0 0000000000 0000000000 0000000000]"
NOT_BEFORE = dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc)
NOT_AFTER = dt.datetime(2036, 1, 1, tzinfo=dt.timezone.utc)


def name(cn: str) -> x509.Name:
    return x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, cn), x509.NameAttribute(NameOID.ORGANIZATION_NAME, "PageWeave Test")])


def certs(signer_key):
    ca_key = ec.generate_private_key(ec.SECP256R1())
    ca = (
        x509.CertificateBuilder()
        .subject_name(name("PageWeave Test CA"))
        .issuer_name(name("PageWeave Test CA"))
        .public_key(ca_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(NOT_BEFORE)
        .not_valid_after(NOT_AFTER)
        .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
        .sign(ca_key, hashes.SHA256())
    )
    signer = (
        x509.CertificateBuilder()
        .subject_name(name("PageWeave Test Signer"))
        .issuer_name(ca.subject)
        .public_key(signer_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(NOT_BEFORE)
        .not_valid_after(NOT_AFTER)
        .sign(ca_key, hashes.SHA256())
    )
    return ca, signer


def unsigned_document(text: str) -> tuple[bytes, int, str]:
    """The document with placeholders, the page object number and the page dictionary."""
    w = gen.PdfWriter()
    catalog, pages_obj, page = w.reserve(), w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    content = w.stream(gen.text_page_content([text, "The parties agree to the terms above."], x=72, y=700))
    sig = w.add("<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached " + BYTE_RANGE_PLACEHOLDER
                + " /Contents <" + "0" * (2 * CONTENTS_BYTES) + "> /M (D:20260301120000Z) /Reason (Approval) /Location (Test lab) >>")
    field = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Approval) /Rect [0 0 0 0] /F 132 /P {page} 0 R /V {sig} 0 R >>")
    page_dict = (f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font} 0 R >> >> "
                 f"/Contents {content} 0 R /Annots [{field} 0 R] >>")
    w.set(page, page_dict)
    w.set(pages_obj, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R /AcroForm << /Fields [{field} 0 R] /SigFlags 3 >> >>")
    info = w.add("<< /Title (Signed agreement) >>")
    return w.render(catalog, info), page, page_dict


def sign(doc: bytes, key, cert, ca) -> bytes:
    start = doc.index(b"/Contents <" + b"0" * 16) + len(b"/Contents ")
    end = doc.index(b">", start) + 1
    ranges = [0, start, end, len(doc) - end]
    placeholder = BYTE_RANGE_PLACEHOLDER.encode()
    filled = ("/ByteRange [%d %d %d %d]" % tuple(ranges)).encode().ljust(len(placeholder))
    doc = doc.replace(placeholder, filled, 1)
    signed = doc[:start] + doc[end:]
    cms = (
        pkcs7.PKCS7SignatureBuilder()
        .set_data(signed)
        .add_signer(cert, key, hashes.SHA256())
        .add_certificate(ca)
        .sign(serialization.Encoding.DER, [pkcs7.PKCS7Options.DetachedSignature, pkcs7.PKCS7Options.Binary])
    )
    assert len(cms) <= CONTENTS_BYTES, len(cms)
    hexed = cms.hex().encode().ljust(2 * CONTENTS_BYTES, b"0")
    return doc[:start + 1] + hexed + doc[end - 1:]


def append_note(doc: bytes, page: int, page_dict: str) -> bytes:
    """An incremental update after signing: a text note added to the page."""
    size = int(doc.rsplit(b"/Size ", 1)[1].split()[0])
    prev = int(doc.rsplit(b"startxref", 1)[1].split()[0])
    root = doc.rsplit(b"/Root ", 1)[1].split(b" R")[0].decode() + " R"
    note = size
    out = bytearray(doc)
    offsets = {}
    offsets[note] = len(out)
    out += b"%d 0 obj\n<< /Type /Annot /Subtype /Text /Rect [300 700 320 720] /Contents (Added after signing PWLATER) >>\nendobj\n" % note
    offsets[page] = len(out)
    updated = page_dict.replace(" 0 R] >>", f" 0 R {note} 0 R] >>")
    out += b"%d 0 obj\n" % page + updated.encode() + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 1\n0000000000 65535 f \n"
    for num in sorted(offsets):
        out += b"%d 1\n%010d 00000 n \n" % (num, offsets[num])
    out += b"trailer\n<< /Size %d /Root %s /Prev %d >>\nstartxref\n%d\n%%%%EOF\n" % (note + 1, root.encode(), prev, xref)
    return bytes(out)


def main() -> int:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE.parent.parent / "fixtures" / "pdf"
    rsa_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    ca, cert = certs(rsa_key)
    doc, page, page_dict = unsigned_document("Signed agreement PWSIGNED")
    signed = sign(doc, rsa_key, cert, ca)
    (out / "signed-rsa.pdf").write_bytes(signed)
    (out / "signed-then-edited.pdf").write_bytes(append_note(signed, page, page_dict))
    i = signed.index(b"PWSIGNED")
    (out / "signed-tampered.pdf").write_bytes(signed[:i] + b"PWSIGNEX" + signed[i + 8:])

    ec_key = ec.generate_private_key(ec.SECP256R1())
    ca2, cert2 = certs(ec_key)
    doc2, _, _ = unsigned_document("Signed agreement PWECDSA")
    (out / "signed-ecdsa.pdf").write_bytes(sign(doc2, ec_key, cert2, ca2))
    print(f"wrote signed fixtures to {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
