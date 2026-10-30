#!/usr/bin/env python3
"""Generate synthetic, rights-clean PDF test fixtures directly from PDF syntax.

Independent of PDFium on purpose (decision D-004): the engine under test must not
also produce its own fixtures. Output is deterministic so fixtures can be
regenerated and diffed. Usage: python3 -I tools/fixtures/gen_fixtures.py [OUT_DIR]
"""
from __future__ import annotations

import sys
from pathlib import Path

LETTER = (612, 792)
A4 = (595, 842)


def esc(text: str) -> str:
    return text.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")


class PdfWriter:
    def __init__(self) -> None:
        self.objects: list[bytes | None] = []

    def reserve(self) -> int:
        self.objects.append(None)
        return len(self.objects)

    def set(self, num: int, body: bytes | str) -> None:
        self.objects[num - 1] = body.encode("latin-1") if isinstance(body, str) else body

    def add(self, body: bytes | str) -> int:
        num = self.reserve()
        self.set(num, body)
        return num

    def stream(self, data: bytes, extra: str = "") -> int:
        return self.add(b"<< /Length %d %s>>\nstream\n" % (len(data), extra.encode()) + data + b"\nendstream")

    def render(self, root: int, info: int | None = None) -> bytes:
        out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for i, body in enumerate(self.objects, start=1):
            assert body is not None, f"object {i} never set"
            offsets.append(len(out))
            out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
        xref = len(out)
        out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(self.objects) + 1)
        for off in offsets:
            out += b"%010d 00000 n \n" % off
        trailer = "<< /Size %d /Root %d 0 R" % (len(self.objects) + 1, root)
        if info:
            trailer += " /Info %d 0 R" % info
        trailer += " /ID [<70616765776561766566697874757265> <70616765776561766566697874757265>] >>"
        out += b"trailer\n" + trailer.encode() + b"\nstartxref\n%d\n%%%%EOF\n" % xref
        return bytes(out)


def text_page_content(lines: list[str], size=12, x=72, y=720, leading=18) -> bytes:
    parts = ["BT", f"/F1 {size} Tf", f"{leading} TL", f"{x} {y} Td"]
    for i, line in enumerate(lines):
        parts.append(f"({esc(line)}) Tj" if i == 0 else f"T* ({esc(line)}) Tj")
    parts.append("ET")
    return "\n".join(parts).encode("latin-1")


def build_document(pages: list[dict], title: str | None = None, outline: bool = False,
                   open_action_js: str | None = None) -> bytes:
    w = PdfWriter()
    catalog = w.reserve()
    pages_obj = w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    page_nums = []
    for p in pages:
        content = w.stream(text_page_content(p["lines"]))
        mw, mh = p.get("size", LETTER)
        rot = f" /Rotate {p['rotate']}" if p.get("rotate") else ""
        page_nums.append(w.add(
            f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 {mw} {mh}]{rot} "
            f"/Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R >>"))
    kids = " ".join(f"{n} 0 R" for n in page_nums)
    w.set(pages_obj, f"<< /Type /Pages /Kids [{kids}] /Count {len(page_nums)} >>")

    extra = ""
    if outline:
        outlines = w.reserve()
        items = [w.reserve() for _ in page_nums]
        for i, (item, pg) in enumerate(zip(items, page_nums)):
            links = ""
            if i > 0:
                links += f" /Prev {items[i-1]} 0 R"
            if i < len(items) - 1:
                links += f" /Next {items[i+1]} 0 R"
            w.set(item, f"<< /Title (Chapter {i+1}) /Parent {outlines} 0 R /Dest [{pg} 0 R /Fit]{links} >>")
        w.set(outlines, f"<< /Type /Outlines /First {items[0]} 0 R /Last {items[-1]} 0 R /Count {len(items)} >>")
        extra += f" /Outlines {outlines} 0 R /PageMode /UseOutlines"
    if open_action_js:
        js = w.add(f"<< /S /JavaScript /JS ({esc(open_action_js)}) >>")
        extra += f" /OpenAction {js} 0 R"
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R{extra} >>")
    info = None
    if title:
        info = w.add(f"<< /Title ({esc(title)}) /Author (PageWeave fixtures) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def build_form_document(defaults: bool = False, xfa: str | None = None) -> bytes:
    """One page with an AcroForm: text, multiline text, checkbox (on-state named /On, not /Yes),
    a two-button radio group and a combo box, each with appearance streams. With `defaults`, the
    same form with default values (/DV) for every field except the comments, for Reset form. With
    `xfa`, an XFA layer in front of the same form: "hybrid" (like government forms: the standard
    form is complete) or "dynamic" (/NeedsRendering: only the XFA form counts) (D-038)."""
    dv = (lambda v: f" /DV {v}") if defaults else (lambda v: "")
    w = PdfWriter()
    catalog, pages_obj, page = w.reserve(), w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    labels = text_page_content(["Form fixture PWFORM", "", "Full name", "", "Subscribe", "",
                                "Plan: basic / pro", "", "Country", "", "Comments"], size=12, x=40, y=740, leading=20)
    content = w.stream(labels)

    def xobj(width: float, height: float, ops: str) -> int:
        data = ops.encode("latin-1")
        return w.add(b"<< /Type /XObject /Subtype /Form /BBox [0 0 %g %g] /Resources << /Font << /Helv %d 0 R >> >> /Length %d >>\nstream\n"
                     % (width, height, font, len(data)) + data + b"\nendstream")

    box = lambda width, height: f"0.4 0.4 0.4 RG 1 w 0.5 0.5 {width - 1} {height - 1} re S"
    cross = "0 0 0 RG 1.5 w 3 3 m 11 11 l S 3 11 m 11 3 l S"
    dot = "0 0 0 rg 4 4 6 6 re f"
    annots = []
    fields = []

    # Text field
    name_ap = xobj(250, 20, box(250, 20))
    name = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Tx /T (full_name) /TU (Full name) /Rect [150 666 400 686]{dv('(Jane Doe)')} "
                 f"/F 4 /P {page} 0 R /DA (/Helv 12 Tf 0 g) /V () /AP << /N {name_ap} 0 R >> /MK << /BC [0.4 0.4 0.4] >> >>")
    annots.append(name); fields.append(name)

    # Checkbox, on-state "On"
    cb_on, cb_off = xobj(14, 14, box(14, 14) + " " + cross), xobj(14, 14, box(14, 14))
    cb = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Btn /T (subscribe) /TU (Subscribe) /Rect [150 628 164 642]{dv('/On')} "
               f"/F 4 /P {page} 0 R /V /Off /AS /Off /AP << /N << /On {cb_on} 0 R /Off {cb_off} 0 R >> >> /MK << /CA (8) >> >>")
    annots.append(cb); fields.append(cb)

    # Radio group "plan": basic / pro
    plan = w.reserve()
    kids = []
    for i, opt in enumerate(["basic", "pro"]):
        on, off = xobj(14, 14, box(14, 14) + " " + dot), xobj(14, 14, box(14, 14))
        x = 150 + i * 60
        kid = w.add(f"<< /Type /Annot /Subtype /Widget /Parent {plan} 0 R /Rect [{x} 588 {x + 14} 602] /F 4 /P {page} 0 R "
                    f"/AS /Off /AP << /N << /{opt} {on} 0 R /Off {off} 0 R >> >> /MK << /CA (l) >> >>")
        kids.append(kid); annots.append(kid)
    w.set(plan, f"<< /FT /Btn /Ff 49152 /T (plan) /TU (Plan) /V /Off{dv('/pro')} /Kids [{' '.join(f'{k} 0 R' for k in kids)}] >>")
    fields.append(plan)

    # Combo box
    combo_ap = xobj(150, 20, box(150, 20) + " /Tx BMC BT /Helv 12 Tf 0 g 4 5 Td (Canada) Tj ET EMC")
    combo = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Ch /Ff 131072 /T (country) /TU (Country) /Rect [150 546 300 566] "
                  f"/F 4 /P {page} 0 R /Opt [(Canada) (Germany) (Japan)] /V (Canada){dv('(Japan)')} /DA (/Helv 12 Tf 0 g) "
                  f"/AP << /N {combo_ap} 0 R >> >>")
    annots.append(combo); fields.append(combo)

    # Multiline text
    notes_ap = xobj(300, 60, box(300, 60))
    notes = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 4096 /T (comments) /TU (Comments) /Rect [150 466 450 526] "
                  f"/F 4 /P {page} 0 R /DA (/Helv 10 Tf 0 g) /V () /AP << /N {notes_ap} 0 R >> >>")
    annots.append(notes); fields.append(notes)

    w.set(page, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font} 0 R >> >> "
                f"/Contents {content} 0 R /Annots [{' '.join(f'{a} 0 R' for a in annots)}] >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>")
    xfa_entry, needs = "", ""
    if xfa:
        packet = (b'<xdp:xdp xmlns:xdp="http://ns.adobe.com/xdp/"><template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">'
                  b'<subform name="form1"><field name="full_name"/></subform></template>'
                  b'<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form1><full_name>Old XFA value</full_name>'
                  b'</form1></xfa:data></xfa:datasets></xdp:xdp>')
        xfa_entry = f" /XFA {w.stream(packet)} 0 R"
        needs = " /NeedsRendering true" if xfa == "dynamic" else ""
        # Usage rights as on government forms (/Perms /UR3): its /EF is a permission list, not files.
        ur3 = w.add("<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /Contents <00> "
                    "/Reference [<< /Type /SigRef /TransformMethod /UR3 /TransformParams << /Type /TransformParams /V /2.2 "
                    "/Document [/FullSave] /Form [/FillIn /Import /Export] /EF [/Create /Delete /Import /Modify] >> >>] >>")
        needs += f" /Perms << /UR3 {ur3} 0 R >>"
    acro = f"<< /Fields [{' '.join(f'{f} 0 R' for f in fields)}] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv {font} 0 R >> >>{xfa_entry} >>"
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R /AcroForm {acro}{needs} >>")
    info = w.add("<< /Title (Form fixture) /Author (PageWeave fixtures) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def build_nested_form_document() -> bytes:
    """One page with a hierarchical AcroForm for form data import/export: applicant.name and
    applicant.address.city (terminal fields under non-terminal parents), a read-only text field
    holding R-1001, a list box (Red/Green/Blue), a checkbox whose on-state is /Yes and a push button."""
    w = PdfWriter()
    catalog, pages_obj, page = w.reserve(), w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    labels = text_page_content(["Nested form fixture PWNESTFORM", "", "Name", "", "City", "", "Reference",
                                "", "Colour", "", "", "", "Agree"], size=12, x=40, y=740, leading=20)
    content = w.stream(labels)

    def xobj(width: float, height: float, ops: str) -> int:
        data = ops.encode("latin-1")
        return w.add(b"<< /Type /XObject /Subtype /Form /BBox [0 0 %g %g] /Resources << /Font << /Helv %d 0 R >> >> /Length %d >>\nstream\n"
                     % (width, height, font, len(data)) + data + b"\nendstream")

    box = lambda width, height: f"0.4 0.4 0.4 RG 1 w 0.5 0.5 {width - 1} {height - 1} re S"
    annots = []

    applicant, address = w.reserve(), w.reserve()
    name_ap = xobj(250, 20, box(250, 20))
    name = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Tx /Parent {applicant} 0 R /T (name) /TU (Applicant name) "
                 f"/Rect [150 706 400 726] /F 4 /P {page} 0 R /DA (/Helv 12 Tf 0 g) /V () /AP << /N {name_ap} 0 R >> >>")
    city_ap = xobj(250, 20, box(250, 20))
    city = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Tx /Parent {address} 0 R /T (city) /TU (City) "
                 f"/Rect [150 666 400 686] /F 4 /P {page} 0 R /DA (/Helv 12 Tf 0 g) /V () /AP << /N {city_ap} 0 R >> >>")
    w.set(address, f"<< /T (address) /Parent {applicant} 0 R /Kids [{city} 0 R] >>")
    w.set(applicant, f"<< /T (applicant) /Kids [{name} 0 R {address} 0 R] >>")
    annots += [name, city]

    ref_ap = xobj(150, 20, box(150, 20) + " /Tx BMC BT /Helv 12 Tf 0 g 4 5 Td (R-1001) Tj ET EMC")
    ref = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 1 /T (ref_no) /TU (Reference) /Rect [150 626 300 646] "
                f"/F 4 /P {page} 0 R /DA (/Helv 12 Tf 0 g) /V (R-1001) /AP << /N {ref_ap} 0 R >> >>")
    annots.append(ref)

    list_ap = xobj(150, 60, box(150, 60))
    colours = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Ch /T (colour) /TU (Colour) /Rect [150 546 300 606] "
                    f"/F 4 /P {page} 0 R /Opt [(Red) (Green) (Blue)] /DA (/Helv 12 Tf 0 g) /AP << /N {list_ap} 0 R >> >>")
    annots.append(colours)

    on, off = xobj(14, 14, box(14, 14) + " 0 0 0 RG 1.5 w 3 3 m 11 11 l S 3 11 m 11 3 l S"), xobj(14, 14, box(14, 14))
    agree = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Btn /T (agree) /TU (Agree) /Rect [150 488 164 502] "
                  f"/F 4 /P {page} 0 R /V /Off /AS /Off /AP << /N << /Yes {on} 0 R /Off {off} 0 R >> >> >>")
    annots.append(agree)

    button_ap = xobj(80, 20, box(80, 20))
    button = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 65536 /T (submit) /Rect [150 440 230 460] "
                   f"/F 4 /P {page} 0 R /AP << /N {button_ap} 0 R >> >>")
    annots.append(button)

    w.set(page, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font} 0 R >> >> "
                f"/Contents {content} 0 R /Annots [{' '.join(f'{a} 0 R' for a in annots)}] >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>")
    fields = [applicant, ref, colours, agree, button]
    acro = f"<< /Fields [{' '.join(f'{f} 0 R' for f in fields)}] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv {font} 0 R >> >> >>"
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R /AcroForm {acro} >>")
    info = w.add("<< /Title (Nested form fixture) /Author (PageWeave fixtures) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def build_redaction_document() -> bytes:
    """Four pages exercising redaction: text runs to cut, images inside / across / outside areas
    (one shared with another page), vector art, a note with a popup, links, a thumbnail, a form
    XObject holding text, a rotated page and a page whose MediaBox does not start at 0,0. Secrets
    are marked so tests can search raw (decompressed) file content for them."""
    w = PdfWriter()
    catalog, pages_obj = w.reserve(), w.reserve()
    p1, p2, p3, p4 = w.reserve(), w.reserve(), w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")

    def image(width: int, height: int, data: bytes) -> int:
        assert len(data) == width * height * 3
        return w.stream(data, f"/Type /XObject /Subtype /Image /Width {width} /Height {height} "
                              "/ColorSpace /DeviceRGB /BitsPerComponent 8 ")

    # Distinct, deterministic pixel data (tests look for these exact bytes in the output).
    im1 = image(8, 8, bytes((i * 7 + 13) % 256 for i in range(8 * 8 * 3)))
    im2 = image(4, 4, bytes((i * 11 + 5) % 256 for i in range(4 * 4 * 3)))
    im3 = image(4, 4, bytes((i * 3 + 101) % 256 for i in range(4 * 4 * 3)))
    thumb = image(2, 2, b"PWTHUMB!PWTH")
    res = f"/Resources << /Font << /F1 {font} 0 R >> /XObject << /Im1 {im1} 0 R /Im2 {im2} 0 R /Im3 {im3} 0 R >> >>"

    c1 = "\n".join([
        "0.9 0.9 1 rg 36 36 540 720 re f",  # background: crosses every area, stays
        "0 g",
        text_page_content(["Name: Alice Example", "Account SECRET-4711 is closed",
                           "Fully hidden line PWHIDE", "Public footer text"], x=72, y=700, leading=20).decode(),
        "0 0 0 rg 400 500 20 20 re f",  # inside an area: removed
        "q 64 0 0 64 300 400 cm /Im1 Do Q",  # crosses an area edge: pixels painted
        "q 32 0 0 32 450 300 cm /Im2 Do Q",  # inside an area: removed
        "q 32 0 0 32 100 100 cm /Im3 Do Q",  # outside every area (also on page 4)
    ]).encode("latin-1")
    note = w.reserve()
    popup = w.add(f"<< /Type /Annot /Subtype /Popup /Rect [480 400 600 500] /Parent {note} 0 R /Open false >>")
    w.set(note, f"<< /Type /Annot /Subtype /Text /Rect [480 520 500 540] /Contents (secret note PWNOTE) "
                f"/Popup {popup} 0 R >>")
    secret_link = w.add("<< /Type /Annot /Subtype /Link /Rect [72 676 260 692] /Border [0 0 0] "
                        "/A << /S /URI /URI (http://example.invalid/PWLINK) >> >>")
    page_link = w.add(f"<< /Type /Annot /Subtype /Link /Rect [72 60 200 76] /Border [0 0 0] /Dest [{p2} 0 R /Fit] >>")
    w.set(p1, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] {res} "
              f"/Contents {w.stream(c1)} 0 R /Thumb {thumb} 0 R "
              f"/Annots [{secret_link} 0 R {note} 0 R {popup} 0 R {page_link} 0 R] >>")

    form_ops = text_page_content(["Inside form PWFORMX"], x=0, y=10).decode().encode("latin-1")
    fm = w.add(b"<< /Type /XObject /Subtype /Form /BBox [0 0 300 30] /Resources << /Font << /F1 %d 0 R >> >> "
               b"/Length %d >>\nstream\n" % (font, len(form_ops)) + form_ops + b"\nendstream")
    c2 = (text_page_content(["Page two visible"], x=72, y=700).decode() + "\nq 1 0 0 1 72 500 cm /Fm1 Do Q").encode()
    w.set(p2, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] "
              f"/Resources << /Font << /F1 {font} 0 R >> /XObject << /Fm1 {fm} 0 R >> >> /Contents {w.stream(c2)} 0 R >>")

    c3 = text_page_content(["Rotated page keeps this", "Rotated secret PWROT here"], x=72, y=700, leading=20)
    w.set(p3, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] /Rotate 90 "
              f"/Resources << /Font << /F1 {font} 0 R >> >> /Contents {w.stream(c3)} 0 R >>")

    c4 = (text_page_content(["Offset page PWOFF secret", "Offset page stays"], x=172, y=800, leading=20).decode()
          + "\nq 32 0 0 32 200 200 cm /Im3 Do Q").encode()
    w.set(p4, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [100 100 712 892] {res} "
              f"/Contents {w.stream(c4)} 0 R >>")

    w.set(pages_obj, f"<< /Type /Pages /Kids [{p1} 0 R {p2} 0 R {p3} 0 R {p4} 0 R] /Count 4 >>")
    outlines, item = w.reserve(), w.reserve()
    w.set(item, f"<< /Title (Chapter SECRET-4711) /Parent {outlines} 0 R /Dest [{p1} 0 R /Fit] >>")
    w.set(outlines, f"<< /Type /Outlines /First {item} 0 R /Last {item} 0 R /Count 1 >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R /Outlines {outlines} 0 R >>")
    info = w.add("<< /Title (Redaction fixture SECRET-4711) /Author (PageWeave fixtures) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def build_redaction_hard_document() -> bytes:
    """Real-world-shaped redaction cases. Page 1: Flate-compressed content; a secret split across a
    TJ kerning array (PWKE + RNSECRET); horizontally scaled text (Tz); text turned 30 degrees by its
    text matrix; an inline image (BI/ID/EI) inside an area; a scaled, rotated image crossing an area
    edge. Page 2: a form XObject nested in a form XObject, holding PWNEST. The file then gets an
    incremental update that replaces page 1's content; the PREVIOUS revision still holds
    OLDREV-SECRET in its bytes (the classic leak a fresh rewrite must not carry over)."""
    import zlib

    w = PdfWriter()
    catalog, pages_obj, p1, p2 = w.reserve(), w.reserve(), w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    img = w.stream(bytes((i * 5 + 77) % 256 for i in range(16 * 16)),
                   "/Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /DeviceGray /BitsPerComponent 8 ")

    def flate(data: bytes) -> int:
        z = zlib.compress(data, 9)
        return w.add(b"<< /Length %d /Filter /FlateDecode >>\nstream\n" % len(z) + z + b"\nendstream")

    # Revision 1 of page 1: the old content (will be superseded by the incremental update).
    old = flate(b"BT /F1 12 Tf 72 700 Td (OLDREV-SECRET old text) Tj ET")
    inline = bytes((i * 13 + 3) % 256 for i in range(8 * 8))
    c1 = (
        b"BT /F1 12 Tf 72 700 Td [(Code: PWKE) -40 (RNSECRET) 120 ( end)] TJ ET\n"
        b"BT /F1 12 Tf 160 Tz 72 650 Td (Wide PWWIDE text) Tj 100 Tz ET\n"
        b"BT /F1 12 Tf 0.866 0.5 -0.5 0.866 300 500 Tm (Tilted PWTILT words) Tj ET\n"
        b"q 40 0 0 40 72 400 cm BI /W 8 /H 8 /CS /G /BPC 8 ID " + inline + b" EI Q\n"
        b"q 0 60 -60 0 400 300 cm /Im1 Do Q\n"
        b"BT /F1 12 Tf 72 200 Td (Public line stays) Tj ET"
    )
    new = flate(c1)
    res = f"/Resources << /Font << /F1 {font} 0 R >> /XObject << /Im1 {img} 0 R >> >>"
    w.set(p1, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] {res} /Contents {old} 0 R >>")

    inner_ops = b"BT /F1 12 Tf 0 0 Td (Nested PWNEST value) Tj ET"
    inner = w.add(b"<< /Type /XObject /Subtype /Form /BBox [0 0 300 20] /Resources << /Font << /F1 %d 0 R >> >> /Length %d >>\nstream\n" % (font, len(inner_ops)) + inner_ops + b"\nendstream")
    outer_ops = b"q 1 0 0 1 10 10 cm /Fi Do Q"
    outer = w.add(b"<< /Type /XObject /Subtype /Form /BBox [0 0 400 60] /Resources << /XObject << /Fi %d 0 R >> >> /Length %d >>\nstream\n" % (inner, len(outer_ops)) + outer_ops + b"\nendstream")
    c2 = flate(b"BT /F1 12 Tf 72 700 Td (Page two visible) Tj ET q 1 0 0 1 72 500 cm /Fo Do Q")
    w.set(p2, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] "
              f"/Resources << /Font << /F1 {font} 0 R >> /XObject << /Fo {outer} 0 R >> >> /Contents {c2} 0 R >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{p1} 0 R {p2} 0 R] /Count 2 >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    base = w.render(catalog)

    # Incremental update: page 1 now points at the new content; the old stream stays in the file.
    page1 = f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] {res} /Contents {new} 0 R >>".encode()
    prev_xref = int(base.rsplit(b"startxref\n", 1)[1].split(b"\n")[0])
    body = bytearray(base)
    off = len(body)
    body += b"%d 0 obj\n" % p1 + page1 + b"\nendobj\n"
    xref = len(body)
    body += b"xref\n0 1\n0000000000 65535 f \n%d 1\n%010d 00000 n \n" % (p1, off)
    body += (b"trailer\n<< /Size %d /Root %d 0 R /Prev %d /ID [<70616765776561766566697874757265> <70616765776561766566697874757265>] >>\nstartxref\n%d\n%%%%EOF\n"
             % (len(w.objects) + 1, catalog, prev_xref, xref))
    return bytes(body)


HIDDEN_IMAGE = bytes((i * 17 + 9) % 256 for i in range(4 * 4 * 3))
THUMB_IMAGE = b"PWTHUMB2PWTH"


def build_hidden_info_document() -> bytes:
    """Every kind of hidden information sanitization removes, each carrying a marker tests search
    for in the (decompressed) output: document properties, XMP (declaring PDF/A), private
    application data, a thumbnail; document, page, field, outline and open-action scripts, a
    Launch link and XFA; an embedded file and a file-attachment annotation; a hidden layer holding
    text and an image, a hidden form XObject (via a membership dictionary) and a hidden annotation;
    a note with popup and a highlight; a filled text field. Also kept content: a visible layer,
    body text, a web link, a page link and an outline. An incremental update leaves an earlier
    version of page 2 (PWOLD-REVISION) in the file."""
    import zlib

    w = PdfWriter()
    catalog, pages_obj, p1, p2 = w.reserve(), w.reserve(), w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    hidden = w.add("<< /Type /OCG /Name (Draft notes) >>")
    shown = w.add("<< /Type /OCG /Name (Final) >>")
    ocmd = w.add(f"<< /Type /OCMD /OCGs [{hidden} 0 R] /P /AllOn >>")
    img = w.stream(HIDDEN_IMAGE, "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 ")
    fx_ops = zlib.compress(b"BT /F1 12 Tf 0 0 Td (Hidden form PWLAYER-XOBJ) Tj ET", 9)
    fx = w.add(b"<< /Type /XObject /Subtype /Form /BBox [0 0 300 20] /OC %d 0 R /Resources << /Font << /F1 %d 0 R >> >> "
               b"/Filter /FlateDecode /Length %d >>\nstream\n" % (ocmd, font, len(fx_ops)) + fx_ops + b"\nendstream")
    c1 = "\n".join([
        text_page_content(["Visible body PWBODY"], x=72, y=740).decode(),
        "/OC /L1 BDC",
        text_page_content(["Draft remark PWLAYER-HIDDEN"], x=72, y=700).decode(),
        "q 40 0 0 40 300 600 cm /Im2 Do Q",
        "EMC",
        "/OC /L2 BDC",
        text_page_content(["Approved PWLAYER-SHOWN"], x=72, y=660).decode(),
        "EMC",
        "q 1 0 0 1 72 620 cm /Fx Do Q",
    ]).encode("latin-1")

    def js(code: str) -> int:
        return w.add(f"<< /S /JavaScript /JS ({esc(code)}) >>")

    def embedded(data: bytes, name: str) -> int:
        ef = w.add(b"<< /Type /EmbeddedFile /Length %d >>\nstream\n" % len(data) + data + b"\nendstream")
        return w.add(f"<< /Type /Filespec /F ({name}) /UF ({name}) /EF << /F {ef} 0 R >> >>")

    note = w.reserve()
    popup = w.add(f"<< /Type /Annot /Subtype /Popup /Rect [400 600 560 700] /Parent {note} 0 R >>")
    w.set(note, f"<< /Type /Annot /Subtype /Text /Rect [500 740 520 760] /Contents (Reviewer note PWCOMMENT) /Popup {popup} 0 R >>")
    highlight = w.add("<< /Type /Annot /Subtype /Highlight /Rect [72 736 200 754] /QuadPoints [72 754 200 754 72 736 200 736] "
                      "/C [1 1 0] /Contents (PWHIGHLIGHT) >>")
    launch = w.add("<< /Type /Annot /Subtype /Link /Rect [72 560 200 576] /Border [0 0 0] "
                   "/A << /S /Launch /F (PWLAUNCH.exe) >> >>")
    web = w.add("<< /Type /Annot /Subtype /Link /Rect [72 540 200 556] /Border [0 0 0] "
                "/A << /S /URI /URI (http://example.invalid/PWWEB) >> >>")
    goto = w.add(f"<< /Type /Annot /Subtype /Link /Rect [72 520 200 536] /Border [0 0 0] /Dest [{p2} 0 R /Fit] >>")
    attach = w.add(f"<< /Type /Annot /Subtype /FileAttachment /Rect [540 500 556 520] /Name /Paperclip "
                   f"/FS {embedded(b'Attached PWATTACH-ANNOT', 'notes.txt')} 0 R /Contents (notes.txt) >>")
    hidden_annot = w.add(f"<< /Type /Annot /Subtype /Square /Rect [300 500 360 540] /OC {hidden} 0 R /C [1 0 0] "
                         f"/Contents (PWLAYER-ANNOT) >>")
    field_ap_ops = b"/Tx BMC BT /F1 12 Tf 2 5 Td (PWFORMVALUE) Tj ET EMC"
    field_ap = w.add(b"<< /Type /XObject /Subtype /Form /BBox [0 0 200 20] /Resources << /Font << /F1 %d 0 R >> >> /Length %d >>\nstream\n"
                     % (font, len(field_ap_ops)) + field_ap_ops + b"\nendstream")
    field = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Tx /T (reference) /Rect [72 460 272 480] /F 4 /P {p1} 0 R "
                  f"/DA (/F1 12 Tf 0 g) /V (PWFORMVALUE) /AP << /N {field_ap} 0 R >> "
                  f"/AA << /K {js('PWJS-FIELD')} 0 R >> >>")
    piece = "/PieceInfo << /PageWeaveFixture << /LastModified (D:20200101000000Z) /Private (PWPIECE) >> >>"
    res1 = (f"/Resources << /Font << /F1 {font} 0 R >> /XObject << /Im2 {img} 0 R /Fx {fx} 0 R >> "
            f"/Properties << /L1 {hidden} 0 R /L2 {shown} 0 R >> >>")
    annots = " ".join(f"{a} 0 R" for a in [note, popup, highlight, launch, web, goto, attach, hidden_annot, field])
    w.set(p1, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] {res1} /Contents {w.stream(c1)} 0 R "
              f"{piece} /AA << /O {js('PWJS-PAGE')} 0 R >> /Annots [{annots}] >>")

    thumb = w.stream(THUMB_IMAGE, "/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 ")
    old2 = w.stream(text_page_content(["Second page PWOLD-REVISION"], x=72, y=700))
    new2 = text_page_content(["Second page PWPAGE2"], x=72, y=700)
    res2 = f"/Resources << /Font << /F1 {font} 0 R >> >>"
    w.set(p2, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] {res2} /Contents {old2} 0 R /Thumb {thumb} 0 R >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{p1} 0 R {p2} 0 R] /Count 2 >>")

    xmp_data = (b'<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/">'
                b'<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description '
                b'xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/" pdfaid:part="2" pdfaid:conformance="B" '
                b'xmlns:dc="http://purl.org/dc/elements/1.1/" dc:creator="PWMETA-XMP"/></rdf:RDF></x:xmpmeta>'
                b'<?xpacket end="w"?>')
    xmp = w.add(b"<< /Type /Metadata /Subtype /XML /Length %d >>\nstream\n" % len(xmp_data) + xmp_data + b"\nendstream")
    xfa = w.stream(b"<xdp:xdp xmlns:xdp='http://ns.adobe.com/xdp/'><script>PWXFA</script></xdp:xdp>")
    outlines, item1, item2 = w.reserve(), w.reserve(), w.reserve()
    w.set(item1, f"<< /Title (Chapter one) /Parent {outlines} 0 R /Next {item2} 0 R /A {js('PWJS-OUTLINE')} 0 R >>")
    w.set(item2, f"<< /Title (Chapter two) /Parent {outlines} 0 R /Prev {item1} 0 R /Dest [{p2} 0 R /Fit] >>")
    w.set(outlines, f"<< /Type /Outlines /First {item1} 0 R /Last {item2} 0 R /Count 2 >>")
    names = (f"/Names << /JavaScript << /Names [(init) {js('PWJS-DOC')} 0 R] >> "
             f"/EmbeddedFiles << /Names [(secret.txt) {embedded(b'Embedded PWATTACH-DOC', 'secret.txt')} 0 R] >> >>")
    ocp = (f"/OCProperties << /OCGs [{hidden} 0 R {shown} 0 R] /D << /Order [{hidden} 0 R {shown} 0 R] "
           f"/ON [{shown} 0 R] /OFF [{hidden} 0 R] >> >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R /Metadata {xmp} 0 R {names} {ocp} "
                   f"/OpenAction {js('PWJS-OPEN')} 0 R /Outlines {outlines} 0 R "
                   f"/AcroForm << /Fields [{field} 0 R] /DA (/F1 0 Tf 0 g) /DR << /Font << /F1 {font} 0 R >> >> /XFA {xfa} 0 R >> >>")
    info = w.add("<< /Title (Hidden info PWMETA-TITLE) /Author (PWMETA-AUTHOR) /Department (PWMETA-CUSTOM) "
                 "/Producer (gen_fixtures.py) >>")
    base = w.render(catalog, info)

    # Incremental update: page 2 gets new content; the old stream stays in the file's bytes.
    content = b"<< /Length %d >>\nstream\n" % len(new2) + new2 + b"\nendstream"
    new_num = len(w.objects) + 1
    page2 = (f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] {res2} /Contents {new_num} 0 R "
             f"/Thumb {thumb} 0 R >>").encode()
    prev_xref = int(base.rsplit(b"startxref\n", 1)[1].split(b"\n")[0])
    body = bytearray(base)
    off_page = len(body)
    body += b"%d 0 obj\n" % p2 + page2 + b"\nendobj\n"
    off_content = len(body)
    body += b"%d 0 obj\n" % new_num + content + b"\nendobj\n"
    xref = len(body)
    body += b"xref\n0 1\n0000000000 65535 f \n%d 1\n%010d 00000 n \n%d 1\n%010d 00000 n \n" % (p2, off_page, new_num, off_content)
    body += (b"trailer\n<< /Size %d /Root %d 0 R /Info %d 0 R /Prev %d /ID [<70616765776561766566697874757265> <70616765776561766566697874757265>] >>\n"
             b"startxref\n%d\n%%%%EOF\n" % (new_num + 1, catalog, info, prev_xref, xref))
    return bytes(body)


def build_signature_structure_document() -> bytes:
    """One page with a signed signature field: the structure PDF readers count as a digital
    signature (/Sig value with /ByteRange and /Contents). The signature bytes are a placeholder, not
    a valid cryptographic signature; tests use it to check that rewritten copies warn that
    signatures no longer hold."""
    w = PdfWriter()
    catalog, pages_obj, page = w.reserve(), w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    content = w.stream(text_page_content(["Signed agreement PWSIGNED"], x=72, y=700))
    sig = w.add("<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached "
                "/ByteRange [0 100 200 300] /Contents <" + "00" * 64 + "> /M (D:20260101000000Z) /Name (Fixture signer) >>")
    field = w.add(f"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /Rect [0 0 0 0] /F 132 /P {page} 0 R /V {sig} 0 R >>")
    w.set(page, f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font} 0 R >> >> "
                f"/Contents {content} 0 R /Annots [{field} 0 R] >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R /AcroForm << /Fields [{field} 0 R] /SigFlags 3 >> >>")
    info = w.add("<< /Title (Signed fixture) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def numbered_pages(n: int, size=LETTER) -> list[dict]:
    return [{"lines": [f"PageWeave fixture page {i}", f"Marker token PWMARK{i:03d}",
                       "The quick brown fox jumps over the lazy dog."], "size": size}
            for i in range(1, n + 1)]


def build_uncompressed_report() -> bytes:
    """Four text-heavy pages stored without compression, each page with its own (identical) font
    object: what a naive generator writes, so "Reduce file size" has real work to do."""
    w = PdfWriter()
    catalog = w.reserve()
    pages_obj = w.reserve()
    kids = []
    for p in range(1, 5):
        font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
        lines = [f"Report page {p}, line {i}: quarterly figures were reviewed and approved. PWREPORT{p}L{i}"
                 for i in range(1, 41)]
        content = w.stream(text_page_content(lines, size=10, y=750, leading=17))
        kids.append(w.add(
            f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] "
            f"/Resources << /Font << /F1 {font} 0 R >> >> /Contents {content} 0 R >>"))
    w.set(pages_obj, "<< /Type /Pages /Kids [%s] /Count %d >>" % (" ".join(f"{k} 0 R" for k in kids), len(kids)))
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    info = w.add("<< /Title (Uncompressed report) >>")
    return w.render(catalog, info)


def build_compare_document(pages: list[dict], title: str) -> bytes:
    """Pages of (font, text) lines with optional extra drawing, for the compare-old/new pair."""
    w = PdfWriter()
    catalog = w.reserve()
    pages_obj = w.reserve()
    f1 = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    f2 = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>")
    kids = []
    for pg in pages:
        parts = ["BT", "/F1 12 Tf", "18 TL", "72 720 Td"]
        for i, (font, line) in enumerate(pg["lines"]):
            parts.append(f"/{font} 12 Tf")
            parts.append(f"({esc(line)}) Tj" if i == 0 else f"T* ({esc(line)}) Tj")
        parts.append("ET")
        if pg.get("draw"):
            parts.append(pg["draw"])
        content = w.stream("\n".join(parts).encode("latin-1"))
        mw, mh = pg.get("size", LETTER)
        kids.append(w.add(
            f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 {mw} {mh}] "
            f"/Resources << /Font << /F1 {f1} 0 R /F2 {f2} 0 R >> >> /Contents {content} 0 R >>"))
    w.set(pages_obj, "<< /Type /Pages /Kids [%s] /Count %d >>" % (" ".join(f"{k} 0 R" for k in kids), len(kids)))
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    info = w.add(f"<< /Title ({esc(title)}) >>")
    return w.render(catalog, info)


def compare_pair() -> tuple[bytes, bytes]:
    """Old and new versions of a report. Changes (new vs old): page 1 same; page 2 one word replaced,
    words inserted, a line deleted and a line set in bold; a page inserted after page 2; old page 3
    deleted; a paragraph moved from page 4 to page 5; page 5 gains a drawn box (visual only); page 6
    changes from Letter to A4; the title changes."""
    n = lambda *lines: [("F1", t) for t in lines]
    moving = "This paragraph moves to another page entirely PWMOVE"
    old = [
        {"lines": n("Annual report PWSAME", "Nothing on this page changes at all.")},
        {"lines": n("Results PWRESULTS", "The quarterly figures were reviewed by the board.",
                    "This line will be deleted PWGONE", "Revenue grew in every region.")},
        {"lines": n("Appendix PWAPPENDIX", "This whole page is removed in the new version.")},
        {"lines": n("Outlook PWOUTLOOK", moving, "Plans for next year are ambitious.")},
        {"lines": n("Risks PWRISKS", "Currency moves remain the main risk.")},
        {"lines": n("Contacts PWSIZE", "Questions go to the investor team.")},
    ]
    new = [
        old[0],
        {"lines": [("F1", "Results PWRESULTS"), ("F1", "The annual figures were reviewed by the full board."),
                   ("F2", "Revenue grew in every region.")]},
        {"lines": n("New section PWINSERTED", "This page did not exist before.")},
        {"lines": n("Outlook PWOUTLOOK", "Plans for next year are ambitious.")},
        {"lines": n("Risks PWRISKS", "Currency moves remain the main risk.", moving),
         "draw": "q 0.8 0.1 0.1 RG 4 w 300 300 150 100 re S Q"},
        {"lines": n("Contacts PWSIZE", "Questions go to the investor team."), "size": A4},
    ]
    return build_compare_document(old, "Report v1"), build_compare_document(new, "Report v2")


def build_blank_pages_document() -> bytes:
    """Six pages for blank-page detection: (1) text, (2) truly empty, (3) a scanned "blank": an
    off-white image with a few dust specks, (4) only a page number, (5) faint grey table lines (real
    content), (6) text."""
    import zlib
    import random
    w = PdfWriter()
    catalog, pages_obj = w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    # Off-white 100 dpi Letter scan with deterministic specks.
    iw, ih = 850, 1100
    px = bytearray([242]) * (iw * ih)
    rnd = random.Random(7)
    for _ in range(60):
        x, y = rnd.randrange(iw - 2), rnd.randrange(ih - 2)
        for dy in range(2):
            for dx in range(2):
                px[(y + dy) * iw + x + dx] = 40
    data = zlib.compress(bytes(px), 9)
    scan = w.add(b"<< /Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceGray /BitsPerComponent 8 "
                 b"/Filter /FlateDecode /Length %d >>\nstream\n" % (iw, ih, len(data)) + data + b"\nendstream")
    table = "0.82 G 0.6 w " + " ".join(f"72 {700 - 30 * i} m 540 {700 - 30 * i} l S" for i in range(16)) + \
            " " + " ".join(f"{72 + 117 * j} 700 m {72 + 117 * j} 250 l S" for j in range(5))
    contents = [
        text_page_content(["Chapter one PWBLANK1", "This page has text."]),
        b"",
        b"q 612 0 0 792 0 0 cm /Scan Do Q",
        b"BT /F1 10 Tf 300 30 Td (4) Tj ET",
        table.encode(),
        text_page_content(["The end PWBLANK6"]),
    ]
    kids = []
    for c in contents:
        cid = w.stream(c)
        kids.append(w.add(f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] "
                          f"/Resources << /Font << /F1 {font} 0 R >> /XObject << /Scan {scan} 0 R >> >> /Contents {cid} 0 R >>"))
    w.set(pages_obj, "<< /Type /Pages /Kids [%s] /Count %d >>" % (" ".join(f"{k} 0 R" for k in kids), len(kids)))
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    return w.render(catalog, w.add("<< /Title (Blank pages) >>"))


def build_tables_document() -> bytes:
    """Two pages of ruled tables for table export: a header cell merged across two columns,
    values a spreadsheet could corrupt (leading zeros, long identifiers, grouped amounts, dates,
    trailing zeros), a cell merged down two rows, and body text around the tables."""
    w = PdfWriter()
    catalog, pages_obj = w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")

    def text(x: float, y: float, s: str, size: float = 10) -> str:
        return f"BT /F1 {size} Tf {x} {y} Td ({esc(s)}) Tj ET\n"

    def grid(xs: list[float], ys: list[float], skip_v: set = frozenset(), skip_h: set = frozenset()) -> str:
        """Lines of a grid; skip_v holds (column edge index, row) pairs left open, skip_h (row edge, column)."""
        ops = "0.6 w 0 G\n"
        for i, y in enumerate(ys):
            for c in range(len(xs) - 1):
                if (i, c) not in skip_h:
                    ops += f"{xs[c]} {y} m {xs[c + 1]} {y} l S\n"
        for j, x in enumerate(xs):
            for r in range(len(ys) - 1):
                if (j, r) not in skip_v:
                    ops += f"{x} {ys[r]} m {x} {ys[r + 1]} l S\n"
        return ops

    page_ids = []
    # Page 1: title, paragraph, table A (header "Account" spans columns 1-2).
    xs = [72, 192, 282, 392, 512]
    ys = [680, 660, 640, 620, 600]
    ops = text(72, 740, "Account summary PWTABLES", 16) + text(72, 712, "Balances at the end of the quarter.")
    ops += grid(xs, ys, skip_v={(1, 0)})
    rows = [["Account", None, "Opened", "Balance"],
            ["007", "Savings", "2026-03-12", "1,234.50"],
            ["123456789012345678", "Current", "12/03/2026", "-4.5"],
            ["00042", "Bonus", "2025-11-30", "12.40"]]
    for r, cells in enumerate(rows):
        for c, v in enumerate(cells):
            if v is not None:
                ops += text(xs[c] + 4, ys[r] - 14, v)
    ops += text(72, 570, "Figures are unaudited.")
    c1 = w.stream(ops.encode("latin-1"))
    page_ids.append(w.add(f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] "
                          f"/Resources << /Font << /F1 {font} 0 R >> >> /Contents {c1} 0 R >>"))
    # Page 2: table B with "North" merged down two rows (no rule between its rows in column 0).
    xs = [72, 200, 330]
    ys = [700, 680, 660, 640]
    ops = text(72, 740, "Stations", 14)
    ops += grid(xs, ys, skip_h={(2, 0)})
    rows = [["Region", "Station"], ["North", "N1"], [None, "N2"]]
    for r, cells in enumerate(rows):
        for c, v in enumerate(cells):
            if v is not None:
                ops += text(xs[c] + 4, ys[r] - 14, v)
    c2 = w.stream(ops.encode("latin-1"))
    page_ids.append(w.add(f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] "
                          f"/Resources << /Font << /F1 {font} 0 R >> >> /Contents {c2} 0 R >>"))
    w.set(pages_obj, f"<< /Type /Pages /Kids [{' '.join(f'{p} 0 R' for p in page_ids)}] /Count {len(page_ids)} >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    info = w.add("<< /Title (Tables fixture) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def build_poster_document() -> bytes:
    """A flat flyer for the Design Reconstruction Studio: tinted background, a dark banner with
    white bold title, a heading, body lines, a red rule, an orange button with white wording, a
    gradient picture and a small footer. Rendered to pixels by the tests, then rebuilt; the expected
    wording, boxes and colours are listed in crates/pw-pdf/tests/studio.rs."""
    w = PdfWriter()
    catalog, pages_obj = w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    bold = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>")
    side = 64
    pixels = bytes(v for y in range(side) for x in range(side) for v in (x * 4, y * 4, 160))
    picture = w.stream(pixels, f"/Type /XObject /Subtype /Image /Width {side} /Height {side} "
                               "/ColorSpace /DeviceRGB /BitsPerComponent 8 ")

    def text(f: str, size: float, x: float, y: float, s: str) -> str:
        return f"BT /{f} {size} Tf {x} {y} Td ({esc(s)}) Tj ET\n"

    ops = "0.98 0.96 0.90 rg 0 0 400 500 re f\n"
    ops += "0.12 0.23 0.48 rg 0 400 400 100 re f\n"
    ops += "1 g " + text("F2", 32, 40, 440, "SPRING FAIR")
    ops += "0 g " + text("F2", 18, 40, 360, "Saturday 14 May")
    ops += "0.25 g " + text("F1", 12, 40, 330, "Music, food and crafts")
    ops += text("F1", 12, 40, 312, "Riverside Park, 10:00 to 18:00")
    ops += "0.8 0.1 0.1 rg 40 292 320 3 re f\n"
    ops += "0.95 0.55 0.1 rg 40 230 150 40 re f\n"
    ops += "1 g " + text("F2", 16, 62, 244, "Free entry")
    ops += "q 110 0 0 110 250 170 cm /Im1 Do Q\n"
    ops += "0.25 g " + text("F1", 9, 40, 40, "Bring a friend")
    content = w.stream(ops.encode("latin-1"))
    page = w.add(f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 400 500] "
                 f"/Resources << /Font << /F1 {font} 0 R /F2 {bold} 0 R >> /XObject << /Im1 {picture} 0 R >> >> "
                 f"/Contents {content} 0 R >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    info = w.add("<< /Title (Poster fixture) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def build_badges_document() -> bytes:
    """Shapes the Design Studio rebuilds as vectors (D-031): a circle, an ellipse, a rounded
    rectangle with white bold wording on it, and a plain rectangle, on a tinted page. Curves are
    four Bezier quarters (k = 0.5523). Expected forms and sizes are in crates/pw-pdf/tests/studio.rs."""
    w = PdfWriter()
    catalog, pages_obj = w.reserve(), w.reserve()
    bold = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>")
    k = 0.5523

    def ellipse(cx: float, cy: float, rx: float, ry: float) -> str:
        return (f"{cx + rx} {cy} m "
                f"{cx + rx} {cy + k * ry} {cx + k * rx} {cy + ry} {cx} {cy + ry} c "
                f"{cx - k * rx} {cy + ry} {cx - rx} {cy + k * ry} {cx - rx} {cy} c "
                f"{cx - rx} {cy - k * ry} {cx - k * rx} {cy - ry} {cx} {cy - ry} c "
                f"{cx + k * rx} {cy - ry} {cx + rx} {cy - k * ry} {cx + rx} {cy} c f\n")

    def rounded(x: float, y: float, rw: float, rh: float, r: float) -> str:
        c = r * (1 - k)
        return (f"{x + r} {y} m {x + rw - r} {y} l {x + rw - c} {y} {x + rw} {y + c} {x + rw} {y + r} c "
                f"{x + rw} {y + rh - r} l {x + rw} {y + rh - c} {x + rw - c} {y + rh} {x + rw - r} {y + rh} c "
                f"{x + r} {y + rh} l {x + c} {y + rh} {x} {y + rh - c} {x} {y + rh - r} c "
                f"{x} {y + r} l {x} {y + c} {x + c} {y} {x + r} {y} c f\n")

    ops = "0.98 0.96 0.90 rg 0 0 300 200 re f\n"
    ops += "0.15 0.35 0.75 rg " + ellipse(60, 140, 40, 40)
    ops += "0.20 0.60 0.30 rg " + ellipse(200, 150, 70, 30)
    ops += "0.95 0.55 0.10 rg " + rounded(20, 30, 140, 50, 16)
    ops += "1 g BT /F1 20 Tf 40 48 Td (SALE 50%) Tj ET\n"
    ops += "0.45 0.45 0.50 rg 190 30 90 60 re f\n"
    content = w.stream(ops.encode("latin-1"))
    page = w.add(f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 300 200] "
                 f"/Resources << /Font << /F1 {bold} 0 R >> >> /Contents {content} 0 R >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    info = w.add("<< /Title (Badges fixture) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def build_caption_document(words: bool = True) -> bytes:
    """White words over a gradient picture (Design Studio, painting words out of a picture). With
    words=False the same page without them: what the picture should look like once the words are
    painted out (crates/pw-pdf/tests/studio.rs compares the two)."""
    w = PdfWriter()
    catalog, pages_obj = w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    bold = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>")
    pw_, ph = 130, 80
    pixels = bytes(v for y in range(ph) for x in range(pw_) for v in (40 + x, 70 + y, 150 - y))
    picture = w.stream(pixels, f"/Type /XObject /Subtype /Image /Width {pw_} /Height {ph} "
                               "/ColorSpace /DeviceRGB /BitsPerComponent 8 ")
    ops = "0.98 0.96 0.90 rg 0 0 300 200 re f\n"
    ops += "q 260 0 0 160 20 20 cm /Im1 Do Q\n"
    if words:
        ops += "1 g BT /F2 30 Tf 45 110 Td (OPEN DAY) Tj ET\n"
        ops += "1 g BT /F1 14 Tf 45 75 Td (Saturday from ten) Tj ET\n"
    content = w.stream(ops.encode("latin-1"))
    page = w.add(f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 300 200] "
                 f"/Resources << /Font << /F1 {font} 0 R /F2 {bold} 0 R >> /XObject << /Im1 {picture} 0 R >> >> "
                 f"/Contents {content} 0 R >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    info = w.add("<< /Title (Caption fixture) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


FONT_LINES = [
    ("Times-Roman", 12, "Minutes of the annual meeting"),
    ("Times-Roman", 18, "The quick brown fox jumps"),
    ("Times-Bold", 24, "Grand Opening Night"),
    ("Times-Italic", 16, "An evening of chamber music"),
    ("Times-Roman", 36, "Certificate"),
    ("Times-Bold", 14, "Hand made in small batches"),
    ("Helvetica", 12, "Minutes of the annual meeting"),
    ("Helvetica", 18, "The quick brown fox jumps"),
    ("Helvetica-Bold", 24, "Grand Opening Night"),
    ("Helvetica-Oblique", 16, "An evening of chamber music"),
    ("Helvetica", 36, "Certificate"),
    ("Helvetica-Bold", 14, "Hand made in small batches"),
]


HOLDOUT_FONT_LINES = [
    ("Times-Roman", 10, "Please reply by the first of June"),
    ("Times-Bold", 20, "Summer Menu"),
    ("Times-Roman", 28, "Winners announced at noon"),
    ("Times-Italic", 22, "with music and dancing"),
    ("Times-Roman", 48, "Diploma"),
    ("Helvetica", 10, "Please reply by the first of June"),
    ("Helvetica-Bold", 20, "Summer Menu"),
    ("Helvetica", 28, "Winners announced at noon"),
    ("Helvetica-Oblique", 22, "with music and dancing"),
    ("Helvetica", 48, "Diploma"),
]


def build_fonts_document(lines=None) -> bytes:
    """Serif (Times) and sans (Helvetica) lines of several sizes and weights, for measuring how well
    the Design Studio tells the two families apart (crates/pw-pdf/tests/studio.rs, FONT_LINES)."""
    w = PdfWriter()
    catalog, pages_obj = w.reserve(), w.reserve()
    lines = lines or FONT_LINES
    names = sorted({f for f, _, _ in lines})
    fonts = {n: w.add(f"<< /Type /Font /Subtype /Type1 /BaseFont /{n} /Encoding /WinAnsiEncoding >>") for n in names}
    ops, y = "", 800
    for f, size, s in lines:
        y -= size * 1.6 + 8
        ops += f"BT /F{names.index(f)} {size} Tf 40 {y} Td ({esc(s)}) Tj ET\n"
    content = w.stream(ops.encode("latin-1"))
    res = " ".join(f"/F{i} {fonts[n]} 0 R" for i, n in enumerate(names))
    page = w.add(f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 420 842] "
                 f"/Resources << /Font << {res} >> >> /Contents {content} 0 R >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    info = w.add("<< /Title (Fonts fixture) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def build_frames_document() -> bytes:
    """Outlined shapes the Design Studio rebuilds as vector frames (D-036): a 3 pt dark blue border
    inset 10 pt, a 2 pt red elliptical ring with "SEAL" inside it, and a heading, on a tinted page.
    Expected forms and widths are in crates/pw-pdf/tests/studio.rs."""
    w = PdfWriter()
    catalog, pages_obj = w.reserve(), w.reserve()
    bold = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>")
    k = 0.5523
    cx, cy, rx, ry = 225, 70, 45, 30
    ring = (f"{cx + rx} {cy} m "
            f"{cx + rx} {cy + k * ry} {cx + k * rx} {cy + ry} {cx} {cy + ry} c "
            f"{cx - k * rx} {cy + ry} {cx - rx} {cy + k * ry} {cx - rx} {cy} c "
            f"{cx - rx} {cy - k * ry} {cx - k * rx} {cy - ry} {cx} {cy - ry} c "
            f"{cx + k * rx} {cy - ry} {cx + rx} {cy - k * ry} {cx + rx} {cy} c S\n")
    ops = "0.98 0.96 0.90 rg 0 0 300 200 re f\n"
    ops += "0.10 0.20 0.50 RG 3 w 11.5 11.5 277 177 re S\n"
    ops += "0.80 0.10 0.10 RG 2 w " + ring
    ops += "0.1 g BT /F1 18 Tf 40 140 Td (Certificate) Tj ET\n"
    ops += "0.8 0.1 0.1 rg BT /F1 14 Tf 207 65 Td (SEAL) Tj ET\n"
    content = w.stream(ops.encode("latin-1"))
    page = w.add(f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 300 200] "
                 f"/Resources << /Font << /F1 {bold} 0 R >> >> /Contents {content} 0 R >>")
    w.set(pages_obj, f"<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    info = w.add("<< /Title (Frames fixture) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def build_borderless_tables_document() -> bytes:
    """Tables without drawn grids for table export (D-020): page 1 has a table of aligned columns
    with no lines at all and a table with only horizontal rules above and below its header and at
    its end, with body text around both; page 2 is two columns of prose (not a table)."""
    w = PdfWriter()
    catalog, pages_obj = w.reserve(), w.reserve()
    font = w.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")

    def text(x: float, y: float, s: str, size: float = 10) -> str:
        return f"BT /F1 {size} Tf {x} {y} Td ({esc(s)}) Tj ET\n"

    page_ids = []
    ops = text(72, 740, "Harvest report PWBORDERLESS", 16)
    ops += text(72, 712, "Crops picked this season are listed below without any lines.")
    xs = [72, 220, 320, 420]
    rows = [["Crop", "Rows", "Yield", "Grade"],
            ["Apples", "120", "0.50", "A"],
            ["Pears", "80", "0.75", "B"],
            ["Plums and damsons", "007", "1,250.00", "A"],
            ["Quince", "12", "3.10", "C"]]
    for r, cells in enumerate(rows):
        for c, v in enumerate(cells):
            ops += text(xs[c], 684 - 16 * r, v)
    ops += text(72, 590, "Yields are in tonnes per row and were measured at the packing house.")
    # A table with horizontal rules only.
    ops += "0.6 w 0 G\n72 552 m 480 552 l S\n72 532 m 480 532 l S\n72 470 m 480 470 l S\n"
    xs = [76, 260, 380]
    rows = [["Station", "Rain mm", "Days"], ["Upper farm", "412", "61"], ["Lower farm", "388", "58"],
            ["River field", "455", "64"]]
    for r, cells in enumerate(rows):
        for c, v in enumerate(cells):
            ops += text(xs[c], 538 - 18 * r - (4 if r else 0), v)
    ops += text(72, 440, "Rain was recorded each morning.")
    c1 = w.stream(ops.encode("latin-1"))
    page_ids.append(w.add(f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] "
                          f"/Resources << /Font << /F1 {font} 0 R >> >> /Contents {c1} 0 R >>"))
    left = ["The river rose slowly through the wet spring and the", "lower fields stayed under water for most of April while",
            "the upper farm dried out early enough to plant beans", "and the first of the summer barley before the rains",
            "returned in May and kept the soil soft until harvest", "began in the last week of the long and warm summer"]
    right = ["Wetlands along the bank stored the floods and let", "them go again over the dry months so the streams",
             "never ran empty and the cattle had water all year", "which the farmers counted as the best outcome of the",
             "work done on the ditches and hedges in the winter", "before the season when the new plan came into use"]
    ops = text(72, 740, "Notes", 14)
    for i, (a, b) in enumerate(zip(left, right)):
        ops += text(72, 712 - 13 * i, a, 9) + text(320, 712 - 13 * i, b, 9)
    c2 = w.stream(ops.encode("latin-1"))
    page_ids.append(w.add(f"<< /Type /Page /Parent {pages_obj} 0 R /MediaBox [0 0 612 792] "
                          f"/Resources << /Font << /F1 {font} 0 R >> >> /Contents {c2} 0 R >>"))
    w.set(pages_obj, f"<< /Type /Pages /Kids [{' '.join(f'{p} 0 R' for p in page_ids)}] /Count {len(page_ids)} >>")
    w.set(catalog, f"<< /Type /Catalog /Pages {pages_obj} 0 R >>")
    info = w.add("<< /Title (Borderless tables fixture) /Producer (gen_fixtures.py) >>")
    return w.render(catalog, info)


def main() -> int:
    if len(sys.argv) == 4 and sys.argv[1] == "--big":
        # Large document for benchmarks (not committed): gen_fixtures.py --big PAGES OUT.pdf
        Path(sys.argv[3]).write_bytes(build_document(numbered_pages(int(sys.argv[2])), title="Benchmark"))
        print(f"wrote {sys.argv[2]} pages to {sys.argv[3]}")
        return 0
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent.parent / "fixtures" / "pdf"
    out.mkdir(parents=True, exist_ok=True)
    files: dict[str, bytes] = {}

    files["single-page.pdf"] = build_document(numbered_pages(1), title="Single page")
    files["five-pages.pdf"] = build_document(numbered_pages(5), title="Five pages", outline=True)
    mixed = numbered_pages(3)
    mixed[1]["size"] = A4
    mixed[2]["rotate"] = 90
    files["mixed-sizes-rotated.pdf"] = build_document(mixed, title="Mixed sizes")
    files["twenty-pages.pdf"] = build_document(numbered_pages(20), title="Twenty pages")
    files["open-action-javascript.pdf"] = build_document(
        numbered_pages(1), title="Has JavaScript", open_action_js="app.alert('should never run');")
    search = [{"lines": ["Alpha beta gamma", "alpha ALPHA alphabet", "delta"]},
              {"lines": ["No match here"]},
              {"lines": ["Gamma and alpha again"]}]
    files["search-terms.pdf"] = build_document(search, title="Search terms")

    files["form-fields.pdf"] = build_form_document()
    files["form-defaults.pdf"] = build_form_document(defaults=True)
    files["tables.pdf"] = build_tables_document()
    files["borderless-tables.pdf"] = build_borderless_tables_document()
    files["poster.pdf"] = build_poster_document()
    files["badges.pdf"] = build_badges_document()
    files["caption.pdf"] = build_caption_document()
    files["fonts.pdf"] = build_fonts_document()
    files["frames.pdf"] = build_frames_document()
    # A word hyphenated across lines (the engine joins it and marks the hyphen), large enough to be
    # recognised again by OCR with --all-pages.
    files["hyphenated.pdf"] = build_document([{"lines": ["Workers paid in cash must report all wa-", "ges by the end of the month.", "Line three ends normally."]}], title="Hyphenated")
    files["form-xfa-hybrid.pdf"] = build_form_document(xfa="hybrid")
    files["form-xfa-dynamic.pdf"] = build_form_document(xfa="dynamic")
    files["fonts-holdout.pdf"] = build_fonts_document(HOLDOUT_FONT_LINES)
    files["caption-clean.pdf"] = build_caption_document(words=False)
    files["form-nested.pdf"] = build_nested_form_document()
    files["redaction.pdf"] = build_redaction_document()
    files["redaction-hard.pdf"] = build_redaction_hard_document()
    files["hidden-info.pdf"] = build_hidden_info_document()
    files["signature-structure.pdf"] = build_signature_structure_document()
    files["uncompressed-report.pdf"] = build_uncompressed_report()
    files["compare-old.pdf"], files["compare-new.pdf"] = compare_pair()
    files["blank-pages.pdf"] = build_blank_pages_document()
    sensitive = [
        {"lines": ["Customer record PWSENS", "Email: ada.lovelace@example.org", "Phone: +44 20 7946 0958",
                   "Card: 4111 1111 1111 1111", "IBAN: GB82 WEST 1234 5698 7654 32", "SSN: 123-45-6789",
                   "Not sensitive: order 2026-10-21, invoice 12345, card 4111 1111 1111 1112"]},
        {"lines": ["Turned page PWSENS2", "Contact: grace@example.com"], "rotate": 90},
    ]
    files["sensitive-info.pdf"] = build_document(sensitive, title="Sensitive info")

    good = files["five-pages.pdf"]
    files["corrupt-truncated.pdf"] = good[: len(good) // 2]
    files["not-a-pdf.pdf"] = b"This is plain text with a .pdf extension.\n"
    files["empty.pdf"] = b""

    for name, data in sorted(files.items()):
        (out / name).write_bytes(data)
    print(f"wrote {len(files)} fixtures to {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
