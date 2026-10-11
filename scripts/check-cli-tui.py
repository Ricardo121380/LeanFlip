#!/usr/bin/env python3
"""Check the native TUI in a real PTY using temporary client configurations.

Uses the existing menu check's standard-library PTY/fixture helpers. Screen
layout, search and confirmation details are also covered by Rust TestBackend
tests; these checks verify real input, persisted effects and terminal cleanup.
"""
import argparse
import fcntl
import json
import re
import select
import unicodedata
import os
from pathlib import Path
import signal
import struct
import sys
import tempfile
import termios
import time

sys.dont_write_bytecode = True
# The hyphenated existing script is loaded without creating cache files.
import importlib.util

spec = importlib.util.spec_from_file_location("menu_check", Path(__file__).with_name("check-cli-menu.py"))
menu = importlib.util.module_from_spec(spec)
spec.loader.exec_module(menu)


def screen_text(raw, width=120, height=24):
    """Reconstruct the CSI cursor updates used by Ratatui, including spaces.

    Matching raw bytes is insufficient: an unchanged space may be represented
    by a cursor move, so a visible phrase is not necessarily contiguous bytes.
    """
    rows = [[" "] * width for _ in range(height)]
    x = y = 0
    for token in re.findall(r"\x1b\[[0-?]*[ -/]*[@-~]|[^\x1b]", raw.decode("utf-8", errors="replace")):
        if token.startswith("\x1b["):
            body, action = token[2:-1], token[-1]
            if body.startswith("?"): continue
            values = [int(value or 0) for value in body.split(";") if value.isdigit() or not value]
            value = (values[0] if values else 0) or 1
            if action in "Hf": y, x = value - 1, ((values[1] if len(values) > 1 else 1) or 1) - 1
            elif action == "G": x = value - 1
            elif action == "d": y = value - 1
            elif action == "A": y -= value
            elif action == "B": y += value
            elif action == "C": x += value
            elif action == "D": x -= value
            elif action == "J" and values and values[0] in (2, 3): rows = [[" "] * width for _ in range(height)]
            elif action == "K" and 0 <= y < height:
                start, end = (0, width) if values and values[0] == 2 else (0, x + 1) if values and values[0] == 1 else (x, width)
                for col in range(max(0, start), min(width, end)): rows[y][col] = " "
            continue
        if token == "\r": x = 0
        elif token == "\n": y += 1
        elif ord(token) >= 32:
            cell_width = 0 if unicodedata.combining(token) else 2 if unicodedata.east_asian_width(token) in "WF" else 1
            if 0 <= y < height and 0 <= x < width:
                rows[y][x] = token
                if cell_width == 2 and x + 1 < width: rows[y][x + 1] = ""
            x += cell_width
    return "\n".join("".join(row) for row in rows)


def expect_screen(terminal, text, timeout=8):
    deadline = time.monotonic() + timeout
    while text not in screen_text(terminal.output, width=getattr(terminal, "width", 120)):
        if time.monotonic() > deadline: raise AssertionError(f"Timed out waiting for visible {text!r}")
        if select.select([terminal.fd], [], [], 0.1)[0]:
            chunk = os.read(terminal.fd, 65536)
            if not chunk: raise AssertionError("Terminal closed before the expected screen")
            terminal.output += chunk; terminal.pending += chunk


def finish(terminal, keys="q", interrupt=None):
    if interrupt == "process":
        os.kill(terminal.pid, signal.SIGINT)
    elif interrupt == "group":
        os.killpg(terminal.pid, signal.SIGINT)
    else:
        terminal.send(keys)
    terminal.expect("\x1b[?1049l")
    terminal.expect("\x1b[?25h")
    flags = termios.tcgetattr(terminal.fd)[3]
    assert flags & termios.ECHO and flags & termios.ICANON, "Terminal input modes were not restored"
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        pid, status = os.waitpid(terminal.pid, os.WNOHANG)
        if pid:
            os.close(terminal.fd)
            terminal.pid = None
            return os.waitstatus_to_exitcode(status)
        time.sleep(0.02)
    raise AssertionError("TUI did not exit")


def start(binary, home, tool=None, width=120):
    args = ["--home", str(home), "--lang", "en", "tui"]
    if tool:
        args.append(tool)
    terminal = menu.Terminal(binary, args, width=width)
    terminal.width = width
    terminal.expect("\x1b[?1049h")
    terminal.expect("LeanFlip")
    return terminal


def check(binary):
    with tempfile.TemporaryDirectory(prefix="ccsl-tui-test-") as root:
        home = Path(root) / "home"
        home.mkdir()
        for ident in ("alpha", "beta"):
            menu.command(binary, home, ["provider", "add", "claude", "--json"], {
                "id": ident, "name": "Same name 工作配置",
                "settingsConfig": {"env": {"ANTHROPIC_BASE_URL": "https://example.com", "ANTHROPIC_AUTH_TOKEN": "fixture-tui-hidden-token"}},
            })
        menu.command(binary, home, ["use", "claude", "alpha", "--json"])
        before = menu.snapshot(home)
        with start(binary, home, "claude") as terminal:
            # Search Beta, preview, accept the default Cancel, then quit.
            assert finish(terminal, "/beta\r\r\rq") == 0
        assert menu.snapshot(home) == before, "Cancelled action wrote configuration"
        with start(binary, home, "claude") as terminal:
            assert finish(terminal, "/beta\r\ryq") == 0
            assert b"fixture-tui-hidden-token" not in terminal.output
        providers = menu.command(binary, home, ["list", "claude", "--json"])["providers"]
        assert next(provider["id"] for provider in providers if provider["current"]) == "beta"
        print("PASS: full-screen search, default cancellation and exact-ID switching")

        menu.command(binary, home, ["provider", "add", "opencode", "--json"], {
            "id": "relay", "name": "Relay",
            "settingsConfig": {"npm": "@ai-sdk/openai-compatible", "options": {"baseURL": "https://example.com", "apiKey": "fixture-tui-hidden-token"}, "models": {"model": {}}},
        })
        for enabled in (True, False):
            with start(binary, home, "opencode", width=80) as terminal:
                assert finish(terminal, "\ryq") == 0
            assert menu.command(binary, home, ["list", "opencode", "--json"])["providers"][0]["enabled"] is enabled
        path = Path(menu.command(binary, home, ["paths", "opencode", "--json"])["tools"]["opencode"][0])
        path.write_text("{malformed-fixture-tui-hidden-token")
        before = menu.snapshot(home)
        with start(binary, home, "opencode") as terminal:
            assert finish(terminal, "\ryq") == 0
            assert b"fixture-tui-hidden-token" not in terminal.output
        assert menu.snapshot(home) == before
        print("PASS: coexist toggles and unreadable configuration stay safe")

        gemini = home / ".gemini" / ".env"
        gemini.parent.mkdir()
        gemini.write_text("GEMINI_API_KEY=fixture-tui-hidden-token\n")
        with start(binary, home, "gemini", width=52) as terminal:
            assert finish(terminal, "iyq") == 0
        assert menu.command(binary, home, ["list", "gemini", "--json"])["providers"]
        assert gemini.read_text() == "GEMINI_API_KEY=fixture-tui-hidden-token\n"
        print("PASS: narrow-screen import saves cards without switching the client")

        empty = Path(root) / "not-created"
        with start(binary, empty) as terminal:
            assert finish(terminal, "?\rp\rr\t/nonexistent\rq") == 0
        assert not empty.exists()
        with start(binary, empty) as terminal:
            fcntl.ioctl(terminal.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 8, 30, 0, 0))
            os.kill(terminal.pid, signal.SIGWINCH)
            assert finish(terminal) == 0
        assert not empty.exists()
        print("PASS: help, paths, refresh, empty search and terminal resize are read-only")

        with start(binary, home, "claude", width=52) as terminal:
            terminal.send("/beta\re")
            expect_screen(terminal, "Edit provider")
            before = menu.snapshot(home)
            terminal.send("\t\x15\x1b[200~https://edited.example/v1\x1b[201~\t\x1b[200~fixture-edited-private-key\n\x1b[201~")
            terminal.send("\r")
            expect_screen(terminal, "Confirm save")
            assert menu.snapshot(home) == before
            terminal.send("\r")
            expect_screen(terminal, "Edit provider")
            assert menu.snapshot(home) == before
            terminal.send("\ry")
            expect_screen(terminal, "Provider connection saved")
            assert finish(terminal, "q") == 0
            assert b"fixture-edited-private-key" not in terminal.output
        store = json.loads((home / ".cc-switch-lite/providers.json").read_text())
        edited = next(card for card in store["apps"]["claude"]["providers"] if card["id"] == "beta")
        assert edited["settingsConfig"]["env"]["ANTHROPIC_BASE_URL"] == "https://edited.example/v1"
        assert edited["settingsConfig"]["env"]["ANTHROPIC_AUTH_TOKEN"] == "fixture-edited-private-key"
        print("PASS: narrow-screen connection editing, bracketed paste, hidden keys and default cancellation")

        source = home / ".cc-switch"
        source.mkdir()
        (source / "config.json").write_text(json.dumps({"claude": {"providers": {"local": {"name": "Local migration", "settingsConfig": {"env": {"ANTHROPIC_AUTH_TOKEN": "fixture-tui-hidden-token"}}}}}}))
        with start(binary, home) as terminal:
            terminal.send("b\r")
            expect_screen(terminal, "Local migration")
            before = menu.snapshot(home)
            terminal.send("\r\r")
            assert menu.snapshot(home) == before
            terminal.send("\ry")
            expect_screen(terminal, "Cards saved 1")
            assert finish(terminal, "\rqq") == 0
        print("PASS: local CC Switch detection, preview, cancel and confirmed migration")

        backup = Path(root) / "roundtrip.leanflip"
        password = "fixture-passphrase-only"
        with start(binary, home) as terminal:
            terminal.send("bjj\r\x15" + str(backup) + "\t" + password + "\t" + password + "\r")
            expect_screen(terminal, "Encrypted backup saved", timeout=30)
            assert finish(terminal, "\rqq") == 0
            assert password.encode() not in terminal.output
        assert backup.read_bytes().startswith(b"age-encryption.org/v1")
        restored = Path(root) / "restored"
        with start(binary, restored) as terminal:
            terminal.send("bj\r" + str(backup) + "\t" + password + "\r")
            expect_screen(terminal, "Provider cards", timeout=30)
            assert not restored.exists(), "Decrypt/preview wrote the target home"
            terminal.send("\ry")
            expect_screen(terminal, "Restore complete")
            assert finish(terminal, "\rqq") == 0
            assert password.encode() not in terminal.output
        assert menu.command(binary, restored, ["list", "claude", "--json"])["providers"]
        assert not (restored / ".claude").exists(), "Backup restore switched the client"
        print("PASS: encrypted export and restore through real TUI forms, masked passwords, and read-only preview")

        for interrupt in (None, "process", "group"):
            with start(binary, empty) as terminal:
                assert finish(terminal, "\x03", interrupt) == 130
        assert not empty.exists()
        print("PASS: Ctrl+C and external SIGINT restore cursor, screen and input mode")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parent.parent / "target/debug/leanflip")
    args = parser.parse_args()
    check(args.binary.resolve(strict=True))
