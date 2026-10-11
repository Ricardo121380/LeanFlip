#!/usr/bin/env python3
"""Check the native TUI in a real PTY using temporary client configurations.

Uses the existing menu check's standard-library PTY/fixture helpers. Screen
layout, search and confirmation details are also covered by Rust TestBackend
tests; these checks verify real input, persisted effects and terminal cleanup.
"""
import argparse
import fcntl
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
    terminal.expect("\x1b[?1049h")
    terminal.expect("CC Switch Lite")
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

        for interrupt in (None, "process", "group"):
            with start(binary, empty) as terminal:
                assert finish(terminal, "\x03", interrupt) == 130
        assert not empty.exists()
        print("PASS: Ctrl+C and external SIGINT restore cursor, screen and input mode")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parent.parent / "target/debug/ccsl")
    args = parser.parse_args()
    check(args.binary.resolve(strict=True))
