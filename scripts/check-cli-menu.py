#!/usr/bin/env python3
"""Exercise the real CLI menu in a Unix PTY, using only temporary HOME layouts.

Build first with cargo build -p leanflip-cli, then run this script. No Python packages
are required. --binary can point to the release binary for a packaging check.
"""

import argparse
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time


class Terminal:
    def __init__(self, binary, args, width=80):
        self.output = b""
        self.pending = b""
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 24, width, 0, 0))
            os.environ["TERM"] = "xterm-256color"
            os.environ["NO_COLOR"] = "1"
            os.execv(str(binary), [str(binary), *args])

    def expect(self, text, size=65536):
        # size=1 停在匹配处，其余输出留在终端队列中。
        # size=1 stops at the match and leaves the rest of the output queued.
        token = text.encode()
        deadline = time.monotonic() + 8
        while token not in self.pending:
            if time.monotonic() > deadline:
                raise AssertionError(f"Timed out waiting for {text!r}")
            if not select.select([self.fd], [], [], 0.1)[0]:
                continue
            try:
                chunk = os.read(self.fd, size)
            except OSError as error:
                if error.errno == errno.EIO:
                    raise AssertionError(f"Terminal closed before {text!r}") from error
                raise
            if not chunk:
                raise AssertionError(f"Terminal reached EOF before {text!r}")
            self.pending += chunk
            self.output += chunk
        self.pending = self.pending[self.pending.index(token) + len(token):]

    def wait_secure_input(self):
        # The password library prints its prompt before switching ECHO off with
        # TCSAFLUSH. Sending immediately on prompt text can be discarded.
        deadline = time.monotonic() + 3
        while termios.tcgetattr(self.fd)[3] & termios.ECHO:
            if time.monotonic() > deadline:
                raise AssertionError("Password input did not enter secure mode")
            time.sleep(0.01)

    def send(self, text):
        os.write(self.fd, text.encode())

    def leave(self, ctrl_c=False, english=False, signal_target=None):
        if signal_target == "group":
            os.killpg(self.pid, signal.SIGINT)
        elif signal_target == "process":
            os.kill(self.pid, signal.SIGINT)
        else:
            self.send("\x03" if ctrl_c else "\x1b")
        self.expect("\x1b[?25h" if ctrl_c else "Menu closed." if english else "已退出菜单。")
        return self.wait_exit()

    def wait_exit(self):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                flags = termios.tcgetattr(self.fd)[3]
                os.close(self.fd); self.pid = None
                assert flags & termios.ECHO and flags & termios.ICANON, "Terminal input modes were not restored"
                return os.waitstatus_to_exitcode(status)
            time.sleep(0.02)
        raise AssertionError("Command did not exit")

    def __enter__(self):
        return self

    def __exit__(self, *_):
        if self.pid is not None:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
            os.close(self.fd)


def command(binary, home, args, provider=None):
    result = subprocess.run(
        [str(binary), "--home", str(home), *args],
        input=None if provider is None else json.dumps(provider),
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(result.stdout)["data"]


def snapshot(home):
    return {str(path.relative_to(home)): path.read_bytes() for path in home.rglob("*") if path.is_file()}


def check(binary):
    with tempfile.TemporaryDirectory(prefix="leanflip-menu-test-") as root:
        home = Path(root) / "home"
        home.mkdir()
        for ident in ("alpha", "beta"):
            command(binary, home, ["provider", "add", "claude", "--json"], {
                "id": ident, "name": ident.title() + " 工作配置",
                "settingsConfig": {"env": {
                    "ANTHROPIC_BASE_URL": f"https://{ident}.example.com",
                    "ANTHROPIC_AUTH_TOKEN": "fixture-secret-do-not-print",
                }},
            })
        command(binary, home, ["use", "claude", "alpha", "--json"])
        command(binary, home, ["provider", "add", "opencode", "--json"], {
            "id": "relay", "name": "Relay",
            "settingsConfig": {"npm": "@ai-sdk/openai-compatible", "options": {
                "baseURL": "https://example.com", "apiKey": "fixture-secret-do-not-print",
            }, "models": {"model": {}}},
        })
        review = command(binary, home, ["provider", "trust-commands", "opencode", "relay", "--json"])
        command(binary, home, ["provider", "trust-commands", "opencode", "relay", "--approve", review["fingerprint"], "--json"])
        with Terminal(binary, ["--home", str(home)]) as terminal:
            terminal.expect("选择工具")
            terminal.send("claude\r")
            terminal.expect("选择供应商或操作")
            before = snapshot(home)
            terminal.send("\x1b[B\r")  # Down arrow selects Beta.
            terminal.expect("确认操作")
            terminal.send("\x1b")
            terminal.expect("选择供应商或操作")
            assert snapshot(home) == before, "Cancelled selection wrote files"
            terminal.send("beta\r")
            terminal.expect("确认操作")
            terminal.send("\r")
            terminal.expect("供应商已切换。")
            terminal.expect("选择供应商或操作")
            providers = command(binary, home, ["list", "claude", "--json"])["providers"]
            assert next(item["id"] for item in providers if item["current"]) == "beta"
            terminal.send("\x1b")
            terminal.expect("选择工具")
            terminal.send("opencode\r")
            terminal.expect("选择供应商或操作")
            for expected, message in ((True, "供应商已启用。"), (False, "供应商已停用")):
                terminal.send("relay\r")
                terminal.expect("确认操作")
                terminal.send("\r")
                terminal.expect(message)
                terminal.expect("选择供应商或操作")
                providers = command(binary, home, ["list", "opencode", "--json"])["providers"]
                assert providers[0]["enabled"] is expected
            terminal.send("\x1b")
            terminal.expect("选择工具")
            assert terminal.leave() == 0
            assert b"fixture-secret-do-not-print" not in terminal.output
        print("PASS: automatic menu, arrows, search, cancel, switch and coexist toggles")

        for save in (False, True):
            before = snapshot(home)
            with Terminal(binary, ["--home", str(home), "--lang", "en", "provider", "edit", "claude", "alpha"]) as terminal:
                terminal.expect("Name")
                terminal.send("\r")
                terminal.expect("Base URL")
                terminal.send("\r")
                terminal.expect("API Key")
                terminal.send("\x1b[B\r")
                terminal.expect("New API Key")
                terminal.wait_secure_input()
                terminal.send("fixture-new-editor-key\r")
                terminal.expect("Save these changes?")
                terminal.send("y\r" if save else "\r")
                terminal.expect("Provider connection saved." if save else "Cancelled; nothing changed.")
                assert terminal.wait_exit() == 0
                assert b"fixture-new-editor-key" not in terminal.output
            if not save:
                assert snapshot(home) == before
            else:
                store = json.loads((home / ".leanflip/providers.json").read_text())
                card = next(card for card in store["apps"]["claude"]["providers"] if card["id"] == "alpha")
                assert card["settingsConfig"]["env"]["ANTHROPIC_AUTH_TOKEN"] == "fixture-new-editor-key"
                assert command(binary, home, ["list", "claude", "--json"])["providers"][1]["current"]
        with Terminal(binary, ["--home", str(home), "--lang", "en", "provider", "edit", "claude", "alpha"]) as terminal:
            terminal.expect("Name")
            terminal.send("\x03")
            terminal.expect("\x1b[?25h")
            assert terminal.wait_exit() == 130
        print("PASS: CLI connection editor hides keys, defaults to cancel, saves only the selected provider, and restores terminal modes")

        # Ctrl+C 在 cooked 模式下由终端直接发出 SIGINT：密码输入时 ECHO 已关闭；两次按键之间，
        # 切换 raw 模式的 TCSADRAIN 在 macOS 上要等这边读完排队的输出。在另一个线程退出会偶尔
        # 留下 raw 模式，所以重复这个时序敏感的场景。
        # Ctrl+C in cooked mode makes the tty raise SIGINT itself: password input has
        # ECHO off, and between key reads the TCSADRAIN switch to raw mode waits (on
        # macOS) until this side reads the queued output. Exiting from another thread
        # occasionally left raw mode behind, so repeat the timing-sensitive case.
        edit = ["--home", str(home), "--lang", "en", "provider", "edit", "claude", "alpha"]
        with Terminal(binary, edit) as terminal:
            terminal.expect("Name")
            terminal.send("\r")
            terminal.expect("Base URL")
            terminal.send("\r")
            terminal.expect("API Key")
            terminal.send("\x1b[B\r")
            terminal.expect("New API Key")
            terminal.wait_secure_input()
            terminal.send("\x03")
            terminal.expect("\x1b[?25h")
            assert terminal.wait_exit() == 130
        for attempt in range(24):
            with Terminal(binary, edit) as terminal:
                terminal.expect("Name", size=1)  # The initial text stays queued.
                time.sleep(attempt % 4 * 0.01)
                if sys.platform == "darwin":
                    flags = termios.tcgetattr(terminal.fd)[3]
                    assert flags & termios.ICANON, "Ctrl+C would not be sent in cooked mode"
                terminal.send("\x03")
                terminal.expect("\x1b[?25h")
                assert terminal.wait_exit() == 130
        print("PASS: Ctrl+C during password input and between key reads restores terminal modes and exits with 130")

        # A malformed client configuration must not be mistaken for disabled.
        path = Path(command(binary, home, ["paths", "opencode", "--json"])["tools"]["opencode"][0])
        path.write_text("{malformed-fixture-secret")
        before = snapshot(home)
        with Terminal(binary, ["--home", str(home), "menu", "opencode"], width=60) as terminal:
            terminal.expect("选择供应商或操作")
            terminal.send("\r")
            terminal.expect("未修改任何配置")
            terminal.expect("选择供应商或操作")
            terminal.send("\x1b")
            terminal.expect("选择工具")
            assert terminal.leave() == 0
            assert b"malformed-fixture-secret" not in terminal.output
        assert snapshot(home) == before
        print("PASS: unreadable configuration is blocked and its contents stay hidden")

        gemini = home / ".gemini" / ".env"
        gemini.parent.mkdir()
        gemini.write_text("GEMINI_API_KEY=fixture-secret-do-not-print\n")
        with Terminal(binary, ["--home", str(home), "menu", "gemini"]) as terminal:
            terminal.expect("还没有供应商")
            terminal.expect("选择供应商或操作")
            terminal.send("\x1b[B\r")
            terminal.expect("确认操作")
            terminal.send("\r")
            terminal.expect("已导入")
            terminal.expect("选择供应商或操作")
            terminal.send("\x1b")
            terminal.expect("选择工具")
            assert terminal.leave() == 0
        assert command(binary, home, ["list", "gemini", "--json"])["providers"]
        assert gemini.read_text() == "GEMINI_API_KEY=fixture-secret-do-not-print\n"
        print("PASS: empty-state import saves providers without changing the client")

        wizard = Path(root) / "wizard"
        wizard.mkdir()
        for accept in (False, True):
            with Terminal(binary, ["--home", str(wizard), "--lang", "en", "menu", "claude"]) as terminal:
                terminal.expect("Choose a provider or action")
                terminal.send("\r")
                terminal.expect("Name")
                terminal.send("Team Relay\r")
                terminal.expect("team-relay")
                terminal.send("\r")
                terminal.expect("Base URL")
                terminal.send("https://example.com/v1\r")
                terminal.expect("API Key (hidden; optional)")
                terminal.wait_secure_input()
                terminal.send("fixture-wizard-hidden-key\r")
                terminal.expect("Model (optional)")
                terminal.send("fixture-model\r")
                terminal.expect("Activate after saving?")
                terminal.send("\r")
                terminal.expect("Save provider?")
                terminal.send("y" if accept else "\r")
                terminal.expect("Provider created." if accept else "Cancelled; nothing changed.")
                terminal.expect("Choose a provider or action")
                terminal.send("\x1b")
                terminal.expect("Choose a tool")
                assert terminal.leave(english=True) == 0
                assert b"fixture-wizard-hidden-key" not in terminal.output
            if not accept: assert not list(wizard.iterdir()), "Cancelled creation wrote data"
        cards = command(binary, wizard, ["list", "claude", "--json"])["providers"]
        assert cards[0]["id"] == "team-relay" and not cards[0]["current"]
        assert not (wizard / ".claude").exists()
        print("PASS: menu creation generates editable IDs, masks keys, previews and saves without activation")

        empty = Path(root) / "empty"
        empty.mkdir()
        with Terminal(binary, ["--home", str(empty), "menu", "--lang", "en"]) as terminal:
            terminal.expect("Choose a tool")
            assert terminal.leave(english=True) == 0
        assert not list(empty.iterdir())
        print("PASS: English menu and read-only open/exit")
        with Terminal(binary, ["--home", str(empty), "menu"]) as terminal:
            terminal.expect("选择工具")
            terminal.expect("12. 退出")  # Wait until the menu is accepting keys.
            assert terminal.leave(ctrl_c=True) == 130
            assert terminal.output.rfind(b"\x1b[?25h") > terminal.output.rfind(b"\x1b[?25l")
        print("PASS: Ctrl+C exits with code 130 and restores the cursor")

        # Package launchers must let the native menu restore the terminal even
        # when a signal reaches the whole foreground group or only the launcher.
        for target in ("group", "process"):
            with Terminal(binary, ["--home", str(empty), "menu"]) as terminal:
                terminal.expect("选择工具")
                terminal.expect("12. 退出")
                assert terminal.leave(ctrl_c=True, signal_target=target) == 130
                assert terminal.output.rfind(b"\x1b[?25h") > terminal.output.rfind(b"\x1b[?25l")
        print("PASS: process-group and launcher SIGINT preserve cleanup and exit code")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parent.parent / "target/debug/leanflip")
    args = parser.parse_args()
    check(args.binary.resolve(strict=True))
