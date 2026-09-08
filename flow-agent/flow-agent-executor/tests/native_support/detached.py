"""The retained detached writer, released only after observed Tool-root exit."""
import json
import os
import socket
import sys
from pathlib import Path

reader, writer = os.pipe()
if os.fork():
    os.close(writer)
    assert os.read(reader, 1) == b"R", "child must reach its release barrier"
    os._exit(0)
os.close(reader)
os.setsid()
# This known helper must not retain the Executor's output collection pipes.
with open(os.devnull, "r+b", buffering=0) as null:
    for descriptor in (0, 1, 2):
        os.dup2(null.fileno(), descriptor)
with socket.socket(socket.AF_UNIX) as channel:
    channel.settimeout(10)
    channel.connect(sys.argv[2])
    channel.sendall(b"ready\n")
    os.write(writer, b"R")
    os.close(writer)
    assert channel.recv(1) == b"G", "controller must observe parent exit first"
    number = 0
    try:
        Path(sys.argv[1]).write_bytes(b"changed")
    except OSError as error:
        number = error.errno
    channel.sendall(json.dumps({"errno": number}).encode() + b"\n")
os._exit(0)
