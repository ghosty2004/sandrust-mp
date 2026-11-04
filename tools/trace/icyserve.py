#!/usr/bin/env python3
"""TRACE-8: tiny local audio-stream server for PlayAudioStreamForPlayer measurements (trace lane).

  icyserve.py PORT FILE.mp3 LOG

  GET /icy.mp3    Shoutcast-style answer ("ICY 200 OK", icy-name/icy-url headers) and, when the
                  client sends "Icy-MetaData: 1", a StreamTitle metadata block every 8192 bytes
                  (the title changes every 10 s: "Trace Artist - Title N").
  GET /icyh.mp3   the same with an "HTTP/1.0 200 OK" status line (Icecast style).
  GET /plain.mp3  plain HTTP/1.0 200 with the MP3 data (no ICY headers, no metadata).
  anything else   404.

The MP3 is streamed in a loop at ~24 kB/s. Every request (line + headers) and every disconnect is
appended to LOG with a monotonic timestamp, so the client's user agent, request headers and
connect/disconnect timing can be read back. Binds 127.0.0.1 only.
"""
import socket
import sys
import threading
import time

PORT = int(sys.argv[1])
DATA = open(sys.argv[2], "rb").read()
LOG = sys.argv[3]
METAINT = 8192
T0 = time.monotonic()
lock = threading.Lock()


def log(msg):
    with lock, open(LOG, "a") as f:
        f.write(f"{time.monotonic() - T0:9.3f} {msg}\n")


def meta_block(n):
    title = f"StreamTitle='Trace Artist - Title {n}';".encode()
    pad = (-len(title)) % 16
    body = title + b"\0" * pad
    return bytes([len(body) // 16]) + body


def serve(conn, addr):
    try:
        req = b""
        while b"\r\n\r\n" not in req and len(req) < 8192:
            chunk = conn.recv(1024)
            if not chunk:
                return
            req += chunk
        head = req.split(b"\r\n\r\n")[0].decode("latin-1")
        log(f"request from {addr[1]}: " + " | ".join(head.split("\r\n")))
        path = head.split(" ")[1] if " " in head else ""
        want_meta = "icy-metadata: 1" in head.lower()
        if path.startswith("/icy"):
            status = "HTTP/1.0 200 OK" if path.startswith("/icyh") else "ICY 200 OK"
            hdr = status + "\r\nicy-name:TraceStation\r\nicy-url:http://trace.invalid/\r\ncontent-type:audio/mpeg\r\n"
            if want_meta:
                hdr += f"icy-metaint:{METAINT}\r\n"
            conn.sendall((hdr + "\r\n").encode())
        elif path.startswith("/plain"):
            conn.sendall(b"HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\n\r\n")
            want_meta = False
        else:
            conn.sendall(b"HTTP/1.0 404 Not Found\r\n\r\n")
            log(f"404 {path}")
            return
        pos, since_meta, sent, start = 0, 0, 0, time.monotonic()
        while True:
            n = 4096
            if want_meta:
                n = min(n, METAINT - since_meta)
            chunk = DATA[pos:pos + n]
            if len(chunk) < n:
                chunk += DATA[: n - len(chunk)]
            pos = (pos + n) % len(DATA)
            conn.sendall(chunk)
            sent += n
            if want_meta:
                since_meta += n
                if since_meta == METAINT:
                    conn.sendall(meta_block(int((time.monotonic() - T0) // 10)))
                    since_meta = 0
            # ~24 kB/s, with a 64 kB head start so the client's prebuffer fills quickly
            ahead = sent - 65536 - (time.monotonic() - start) * 24000
            if ahead > 0:
                time.sleep(ahead / 24000)
    except OSError as e:
        log(f"closed {addr[1]}: {e.__class__.__name__}")
    finally:
        log(f"disconnect {addr[1]}")
        conn.close()


def main():
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", PORT))
    s.listen(8)
    log(f"listening on {PORT}")
    while True:
        conn, addr = s.accept()
        threading.Thread(target=serve, args=(conn, addr), daemon=True).start()


if __name__ == "__main__":
    main()
