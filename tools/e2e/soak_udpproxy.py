#!/usr/bin/env python3
"""Userspace UDP proxy for tools/e2e/soak.sh (network-loss case).

    soak_udpproxy.py LISTEN_HOST:PORT SERVER_PORT

Forwards datagrams between clients on LISTEN_HOST:PORT and the server on 127.0.0.1:SERVER_PORT.
SA:MP's packet cipher is keyed with the server port the client dials, so the proxy listens on the
server's port number on another loopback address (soak.sh: 127.0.0.2, server bound to 127.0.0.1). Every client address gets its own upstream socket, so a client
that reconnects from a new socket also appears with a new source port at the server.
SIGUSR1 starts dropping every datagram in both directions, SIGUSR2 stops it.
Prints one line per event and per new mapping.
"""
import selectors
import signal
import socket
import sys
import time

listen_host, listen_port = sys.argv[1].rsplit(":", 1)
listen_port, server_port = int(listen_port), int(sys.argv[2])
server = ("127.0.0.1", server_port)
drop = False
stats = {"fwd": 0, "back": 0, "dropped": 0}


def log(msg):
    print("[proxy %.3f] %s" % (time.time(), msg), flush=True)


def on_usr1(*_):
    global drop
    drop = True
    log("drop on")


def on_usr2(*_):
    global drop
    drop = False
    log("drop off (dropped %d datagrams so far)" % stats["dropped"])


signal.signal(signal.SIGUSR1, on_usr1)
signal.signal(signal.SIGUSR2, on_usr2)
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))

front = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
front.bind((listen_host, listen_port))
front.setblocking(False)
sel = selectors.DefaultSelector()
sel.register(front, selectors.EVENT_READ, None)
up = {}  # client addr -> [upstream socket, last activity]
log("listening on %s:%d -> %d" % (listen_host, listen_port, server_port))

last_gc = time.time()
while True:
    try:
        events = sel.select(timeout=1.0)
    except InterruptedError:
        continue
    now = time.time()
    for key, _ in events:
        sock = key.fileobj
        try:
            data, addr = sock.recvfrom(65536)
        except (BlockingIOError, ConnectionRefusedError, InterruptedError):
            continue
        if key.data is None:
            ent = up.get(addr)
            if ent is None:
                s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                s.bind(("127.0.0.1", 0))
                s.connect(server)
                s.setblocking(False)
                sel.register(s, selectors.EVENT_READ, addr)
                ent = up[addr] = [s, now]
                log("new mapping client %d -> upstream %d" % (addr[1], s.getsockname()[1]))
            ent[1] = now
            if drop:
                stats["dropped"] += 1
                continue
            try:
                ent[0].send(data)
                stats["fwd"] += 1
            except OSError:
                pass
        else:
            caddr = key.data
            if drop:
                stats["dropped"] += 1
                continue
            try:
                front.sendto(data, caddr)
                stats["back"] += 1
            except OSError:
                pass
    if now - last_gc > 30:
        last_gc = now
        for addr, (s, t) in list(up.items()):
            if now - t > 180:
                sel.unregister(s)
                s.close()
                del up[addr]
                log("mapping %d expired" % addr[1])
