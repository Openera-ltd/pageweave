#!/usr/bin/env python3
"""Create password-protected fixtures from generated plain fixtures using pypdf (BSD-3-Clause).

Requires `pip install pypdf`. Test-only passwords; these files contain no real content.
Usage: python3 -I tools/fixtures/gen_encrypted.py [FIXTURE_DIR]
"""
import sys
from pathlib import Path

from pypdf import PdfReader, PdfWriter

USER_PASSWORD = "pw-user-test"
OWNER_PASSWORD = "pw-owner-test"


def main() -> int:
    d = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent.parent / "fixtures" / "pdf"
    src = PdfReader(str(d / "five-pages.pdf"))
    w = PdfWriter()
    for p in src.pages:
        w.add_page(p)
    # RC4-128 (security handler revision 3) needs no extra crypto dependency.
    w.encrypt(user_password=USER_PASSWORD, owner_password=OWNER_PASSWORD, algorithm="RC4-128")
    w.add_metadata({"/Title": "Encrypted five pages"})
    with open(d / "encrypted-rc4-user-password.pdf", "wb") as f:
        w.write(f)
    print("wrote encrypted-rc4-user-password.pdf")
    return 0


if __name__ == "__main__":
    sys.exit(main())
