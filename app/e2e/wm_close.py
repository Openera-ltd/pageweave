#!/usr/bin/env python3
"""Ask an X11 window to close the way a title-bar close button does (WM_DELETE_WINDOW).

WebDriver's "close window" tears the session down without an OS close request, so the app's
unsaved-changes guard can only be exercised end to end with a real close request. Uses libX11 via
ctypes (no extra packages). Exit codes: 0 sent, 2 no display, 3 window not found.

Usage: python3 -I e2e/wm_close.py "Window title"
"""
from __future__ import annotations

import ctypes
import ctypes.util
import sys

X = ctypes.cdll.LoadLibrary(ctypes.util.find_library("X11") or "libX11.so.6")
Window = ctypes.c_ulong
Atom = ctypes.c_ulong


class XClientMessageEvent(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int),
        ("serial", ctypes.c_ulong),
        ("send_event", ctypes.c_int),
        ("display", ctypes.c_void_p),
        ("window", Window),
        ("message_type", Atom),
        ("format", ctypes.c_int),
        ("data", ctypes.c_long * 5),
    ]


class XEvent(ctypes.Union):
    _fields_ = [("xclient", XClientMessageEvent), ("pad", ctypes.c_long * 24)]


X.XOpenDisplay.restype = ctypes.c_void_p
X.XOpenDisplay.argtypes = [ctypes.c_char_p]
X.XDefaultRootWindow.restype = Window
X.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
X.XInternAtom.restype = Atom
X.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
X.XQueryTree.argtypes = [ctypes.c_void_p, Window, ctypes.POINTER(Window), ctypes.POINTER(Window), ctypes.POINTER(ctypes.POINTER(Window)), ctypes.POINTER(ctypes.c_uint)]
X.XFetchName.argtypes = [ctypes.c_void_p, Window, ctypes.POINTER(ctypes.c_char_p)]
X.XSendEvent.argtypes = [ctypes.c_void_p, Window, ctypes.c_int, ctypes.c_long, ctypes.POINTER(XEvent)]
X.XFlush.argtypes = [ctypes.c_void_p]
X.XFree.argtypes = [ctypes.c_void_p]
X.XCloseDisplay.argtypes = [ctypes.c_void_p]

CLIENT_MESSAGE = 33


def find(dpy: int, win: int, title: bytes) -> int | None:
    name = ctypes.c_char_p()
    if X.XFetchName(dpy, win, ctypes.byref(name)) and name.value == title:
        return win
    root, parent = Window(), Window()
    children = ctypes.POINTER(Window)()
    n = ctypes.c_uint()
    if not X.XQueryTree(dpy, win, ctypes.byref(root), ctypes.byref(parent), ctypes.byref(children), ctypes.byref(n)):
        return None
    try:
        for i in range(n.value):
            hit = find(dpy, children[i], title)
            if hit:
                return hit
    finally:
        if children:
            X.XFree(children)
    return None


def main() -> int:
    title = sys.argv[1].encode() if len(sys.argv) > 1 else b"PageWeave"
    dpy = X.XOpenDisplay(None)
    if not dpy:
        print("no X display", file=sys.stderr)
        return 2
    try:
        win = find(dpy, X.XDefaultRootWindow(dpy), title)
        if not win:
            print(f"window {title.decode()!r} not found", file=sys.stderr)
            return 3
        ev = XEvent()
        ev.xclient.type = CLIENT_MESSAGE
        ev.xclient.window = win
        ev.xclient.message_type = X.XInternAtom(dpy, b"WM_PROTOCOLS", 0)
        ev.xclient.format = 32
        ev.xclient.data[0] = X.XInternAtom(dpy, b"WM_DELETE_WINDOW", 0)
        X.XSendEvent(dpy, win, 0, 0, ctypes.byref(ev))
        X.XFlush(dpy)
        return 0
    finally:
        X.XCloseDisplay(dpy)


if __name__ == "__main__":
    sys.exit(main())
