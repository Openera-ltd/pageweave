"""Synthetic text for OCR corpora (PRD 15 "synthetic documents"): authored vocabulary and
generators, no third-party text. Deterministic for a given `random.Random`."""
from __future__ import annotations

import random

# Common English function and content words, authored for this project (not copied from a list).
WORDS = """
the of and to in is that for it as was with on be at by this had not are but from or have an they
which one you were her all she there would their we him been has when who will more no if out so
said what up its about into than them can only other new some could time these two may then do
first any my now such like our over man me even most made after also did many before must through
back years where much your way well down should because each just those people how too little
state good very make world still own see men work long get here between both life being under
never day same another know while last might us great old year off come since against go came
right used take three states himself few house use during without again place american around
however home small found thought went say part once general high upon school every does got
united left number course war until always away something fact though water less public put think
almost hand enough far took head yet government system better set told nothing night end why
called didn't eyes find going look asked later knew point next program city business give group
toward young days let room president side social given present several order national possible
rather second face per among form important often things looked early white case john become large
big need four within felt children along saw best church ever least power development light
thing seemed family interest want members mind country area others although turned done open god
service certain kind problem began different door thus help sense means whole matter perhaps itself
york times human law line above name example action company hands local show whether five history
gave today either act feet across taken past quite anything seen having death week experience body
word half really field am car words already themselves information tell together college shall
money period held keep sure probably free seems political real behind cannot miss question air
office making brought whose special heard major problems ago became federal moment study available
known result street economic boy position reason change south board individual job areas society
west close turn love community true court force full seem am policy care wife future age wanted
department voice center common control necessary following front sometimes six woman else clear
student report schedule invoice account balance payment received shipping address delivery
customer quantity amount subtotal reference contract signature approved document section page
""".split()

# Latin-1 accented words, so the corpus is not ASCII-only.
ACCENTED = "café naïve résumé façade über straße niño señor déjà coöperate jalapeño piñata crème brûlée Zürich Ångström Øresund São Bogotá Málaga Göteborg".split()

FIRST = "Ada Grace Alan Linus Barbara Edsger Donald Frances Ken Margaret Dennis Radia Niklaus Leslie Tim Hedy Katherine Claude John Joan".split()
LAST = "Lovelace Hopper Turing Torvalds Liskov Dijkstra Knuth Allen Thompson Hamilton Ritchie Perlman Wirth Lamport Berners-Lee Lamarr Johnson Shannon Backus Clarke".split()
STREETS = "Oak Maple Cedar Elm Pine Birch Willow Harbor Station Mill River Market Church Bridge".split()
STREET_KINDS = ["Street", "Avenue", "Road", "Lane", "Drive", "Way", "Court", "Place"]
CITIES = "Springfield Riverton Fairview Greenville Bristol Madison Clinton Franklin Georgetown Salem Ashland Oxford".split()
MONTHS = "January February March April May June July August September October November December".split()


def word(r: random.Random) -> str:
    if r.random() < 0.04:
        return r.choice(ACCENTED)
    return r.choice(WORDS)


def sentence(r: random.Random, min_words: int = 6, max_words: int = 18) -> str:
    n = r.randint(min_words, max_words)
    ws = [word(r) for _ in range(n)]
    # Occasional numbers, capitalised names and punctuation inside sentences.
    for i in range(len(ws)):
        x = r.random()
        if x < 0.05:
            ws[i] = str(r.randint(2, 9999))
        elif x < 0.08:
            ws[i] = r.choice(FIRST)
        elif x < 0.11 and i < len(ws) - 1:
            ws[i] += ","
    ws[0] = ws[0][:1].upper() + ws[0][1:]
    end = r.choices([".", ".", ".", "?", "!", ";", ":"], k=1)[0]
    return " ".join(ws) + end


def paragraph(r: random.Random, sentences: int) -> str:
    return " ".join(sentence(r) for _ in range(sentences))


def date(r: random.Random) -> str:
    y, m, d = r.randint(1990, 2030), r.randint(1, 12), r.randint(1, 28)
    style = r.randint(0, 3)
    if style == 0:
        return f"{y:04d}-{m:02d}-{d:02d}"
    if style == 1:
        return f"{d:02d}/{m:02d}/{y:04d}"
    if style == 2:
        return f"{MONTHS[m - 1]} {d}, {y}"
    return f"{d} {MONTHS[m - 1][:3]} {y}"


def money(r: random.Random) -> str:
    v = r.randint(1, 9_999_999) / 100
    sym = r.choice(["$", "€", "£", ""])
    s = f"{v:,.2f}"
    return f"{sym}{s}" if sym else f"{s} EUR"


def ident(r: random.Random) -> str:
    letters = "ABCDEFGHJKLMNPQRSTUVWXYZ"
    return f"{''.join(r.choice(letters) for _ in range(r.randint(2, 4)))}-{r.randint(100, 999999)}"


def name(r: random.Random) -> str:
    return f"{r.choice(FIRST)} {r.choice(LAST)}"


def email(r: random.Random) -> str:
    return f"{r.choice(FIRST).lower()}.{r.choice(LAST).lower()}@example.{r.choice(['com', 'org', 'net'])}"


def address(r: random.Random) -> str:
    return f"{r.randint(1, 9999)} {r.choice(STREETS)} {r.choice(STREET_KINDS)}, {r.choice(CITIES)}"


def phone(r: random.Random) -> str:
    return f"+1 ({r.randint(200, 989)}) 555-{r.randint(1000, 9999)}"


FIELDS = [
    ("Invoice number", ident), ("Date", date), ("Due date", date), ("Customer", name), ("Contact", email),
    ("Address", address), ("Phone", phone), ("Reference", ident), ("Amount due", money), ("Account", ident),
]


def item_row(r: random.Random) -> list[str]:
    desc = " ".join(word(r) for _ in range(r.randint(2, 4))).capitalize()
    qty = r.randint(1, 250)
    price = r.randint(100, 500_000) / 100
    return [desc, str(qty), f"{price:,.2f}", f"{qty * price:,.2f}"]
