#!/usr/bin/env python3
"""Draw PageWeave's original app icon (1024x1024 PNG). No third-party artwork is used.

A blue rounded square with a white page whose lower edge is woven by two accent strands.
Usage: python3 -I tools/make_icon.py OUT.png
"""
import sys
from PIL import Image, ImageDraw

S = 1024
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
d.rounded_rectangle([64, 64, S - 64, S - 64], radius=200, fill=(47, 111, 222, 255))
# Page with folded corner.
page = [(300, 220), (640, 220), (740, 320), (740, 800), (300, 800)]
d.polygon(page, fill=(255, 255, 255, 255))
d.polygon([(640, 220), (640, 320), (740, 320)], fill=(196, 214, 246, 255))
# Text lines.
for y in (390, 460, 530):
    d.rounded_rectangle([360, y, 680, y + 26], radius=13, fill=(160, 186, 232, 255))
# Woven strands across the lower page.
d.rounded_rectangle([250, 610, 790, 660], radius=25, fill=(23, 64, 150, 255))
d.rounded_rectangle([250, 700, 790, 750], radius=25, fill=(98, 160, 255, 255))
for x in (380, 520, 660):
    d.rectangle([x, 600, x + 46, 670], fill=(255, 255, 255, 255))
img.save(sys.argv[1])
