#!/usr/bin/env python3
"""A virtual mouse for the Wayland compositor at $WAYLAND_DISPLAY, moved
and pressed as told, a line at a time, through the FIFO named as its one
argument. Started by scripts/e2e.sh, which runs it until the end.

    move X Y WIDTH HEIGHT   to X Y of a layout WIDTH by HEIGHT
    press left|right
    release left|right

A headless sway has no input devices, so its seat offers no pointer at
all. This mouse is one, and what it does goes through sway as a hand's
mouse would: sway's own `seat cursor` commands move nothing while a
button is held, so they can't drag.

Speaks the Wayland wire protocol itself, so it needs nothing beyond
Python: a registry, a seat, and zwlr_virtual_pointer_manager_v1.
"""
import os
import select
import socket
import struct
import sys
import time

DISPLAY, REGISTRY, SYNCED, SEAT, MANAGER, MOUSE, PLUGGED = range(1, 8)
BUTTONS = {"left": 0x110, "right": 0x111}
# Places go to the compositor in eighths of a pixel.
FINE = 8


def string(text):
    data = text.encode() + b"\0"
    return struct.pack("<I", len(data)) + data + b"\0" * (-len(data) % 4)


def send(sock, obj, opcode, payload=b""):
    sock.sendall(struct.pack("<II", obj, (8 + len(payload)) << 16 | opcode) + payload)


def read_string(payload, at):
    (length,) = struct.unpack_from("<I", payload, at)
    text = payload[at + 4 : at + 4 + length - 1].decode()
    return text, at + 4 + length + (-length % 4)


class Events:
    """The compositor's events, as they come."""

    def __init__(self, sock):
        self.sock, self.buffer = sock, b""

    def read(self):
        data = self.sock.recv(4096)
        if not data:
            sys.exit("mouse.py: the compositor hung up")
        self.buffer += data
        while len(self.buffer) >= 8:
            obj, word = struct.unpack_from("<II", self.buffer)
            size = word >> 16
            if len(self.buffer) < size:
                break
            event, self.buffer = (obj, word & 0xFFFF, self.buffer[8:size]), self.buffer[size:]
            if obj == DISPLAY and event[1] == 0:
                (_, code) = struct.unpack_from("<II", event[2])
                message, _ = read_string(event[2], 8)
                sys.exit(f"mouse.py: the compositor refused: {message} ({code})")
            yield event

    def until(self, callback, each=lambda event: None):
        while True:
            for event in self.read():
                if event[0] == callback and event[1] == 0:
                    return
                each(event)


def act(sock, words):
    now = int(time.monotonic() * 1000) & 0xFFFFFFFF
    if len(words) == 5 and words[0] == "move":
        x, y = (round(float(word) * FINE) for word in words[1:3])
        width, height = (int(word) * FINE for word in words[3:5])
        send(sock, MOUSE, 1, struct.pack("<IIIII", now, x, y, width, height))
    elif len(words) == 2 and words[0] in ("press", "release") and words[1] in BUTTONS:
        send(sock, MOUSE, 2, struct.pack("<III", now, BUTTONS[words[1]], words[0] == "press"))
    else:
        print(f"mouse.py: no such thing to do: {' '.join(words)}", flush=True)
        return
    send(sock, MOUSE, 4)


def main():
    fifo = sys.argv[1]
    name = os.environ["WAYLAND_DISPLAY"]
    path = name if name.startswith("/") else os.path.join(os.environ["XDG_RUNTIME_DIR"], name)
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(path)
    events = Events(sock)
    found = {}

    def note_global(event):
        obj, opcode, payload = event
        if obj == REGISTRY and opcode == 0:
            (global_name,) = struct.unpack_from("<I", payload)
            interface, _ = read_string(payload, 4)
            found.setdefault(interface, global_name)

    send(sock, DISPLAY, 1, struct.pack("<I", REGISTRY))
    send(sock, DISPLAY, 0, struct.pack("<I", SYNCED))
    events.until(SYNCED, note_global)
    for interface in ("wl_seat", "zwlr_virtual_pointer_manager_v1"):
        if interface not in found:
            sys.exit(f"mouse.py: the compositor has no {interface}")
    for interface, new in (("wl_seat", SEAT), ("zwlr_virtual_pointer_manager_v1", MANAGER)):
        bind = struct.pack("<I", found[interface]) + string(interface) + struct.pack("<II", 1, new)
        send(sock, REGISTRY, 0, bind)
    send(sock, MANAGER, 0, struct.pack("<II", SEAT, MOUSE))
    send(sock, DISPLAY, 0, struct.pack("<I", PLUGGED))
    events.until(PLUGGED)
    print("mouse.py: plugged in", flush=True)
    # Read and written both, so it waits for lines and never sees the end
    # of them when a writer closes it.
    told = os.open(fifo, os.O_RDWR)
    pending = b""
    while True:
        ready, _, _ = select.select([sock, told], [], [])
        if sock in ready:
            for _ in events.read():
                pass
        if told in ready:
            pending += os.read(told, 4096)
            while b"\n" in pending:
                line, pending = pending.split(b"\n", 1)
                act(sock, line.decode().split())


if __name__ == "__main__":
    main()
