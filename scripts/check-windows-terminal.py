#!/usr/bin/env python3
"""Smoke-test the native menu and TUI in Windows ConPTY, without dependencies.

Uses Microsoft's CreatePseudoConsole/STARTUPINFOEX lifecycle and a dedicated
reader thread. All commands run against one disposable home with dummy keys.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time

k = c.WinDLL('kernel32', use_last_error=True)
P = c.c_void_p
SIZE = c.c_size_t


class Coord(c.Structure):
    _fields_ = [('X', w.SHORT), ('Y', w.SHORT)]


class Startup(c.Structure):
    _fields_ = [('cb', w.DWORD), ('reserved', w.LPWSTR), ('desktop', w.LPWSTR), ('title', w.LPWSTR),
                ('x', w.DWORD), ('y', w.DWORD), ('width', w.DWORD), ('height', w.DWORD),
                ('charsX', w.DWORD), ('charsY', w.DWORD), ('fill', w.DWORD), ('flags', w.DWORD),
                ('show', w.WORD), ('reservedSize', w.WORD), ('reservedBytes', P),
                ('stdin', w.HANDLE), ('stdout', w.HANDLE), ('stderr', w.HANDLE)]


class StartupEx(c.Structure):
    _fields_ = [('startup', Startup), ('attributes', P)]


class Process(c.Structure):
    _fields_ = [('process', w.HANDLE), ('thread', w.HANDLE), ('pid', w.DWORD), ('tid', w.DWORD)]


def api(name, result, *args):
    fn = getattr(k, name)
    fn.restype, fn.argtypes = result, args
    return fn


pipe = api('CreatePipe', w.BOOL, c.POINTER(w.HANDLE), c.POINTER(w.HANDLE), P, w.DWORD)
create_pty = api('CreatePseudoConsole', w.LONG, Coord, w.HANDLE, w.HANDLE, w.DWORD, c.POINTER(P))
close_pty = api('ClosePseudoConsole', None, P)
init_attrs = api('InitializeProcThreadAttributeList', w.BOOL, P, w.DWORD, w.DWORD, c.POINTER(SIZE))
set_attr = api('UpdateProcThreadAttribute', w.BOOL, P, w.DWORD, SIZE, P, SIZE, P, P)
delete_attrs = api('DeleteProcThreadAttributeList', None, P)
create_process = api('CreateProcessW', w.BOOL, w.LPCWSTR, w.LPWSTR, P, P, w.BOOL, w.DWORD, P, w.LPCWSTR, P, c.POINTER(Process))
read = api('ReadFile', w.BOOL, w.HANDLE, P, w.DWORD, c.POINTER(w.DWORD), P)
write = api('WriteFile', w.BOOL, w.HANDLE, P, w.DWORD, c.POINTER(w.DWORD), P)
close = api('CloseHandle', w.BOOL, w.HANDLE)
wait = api('WaitForSingleObject', w.DWORD, w.HANDLE, w.DWORD)
exit_code = api('GetExitCodeProcess', w.BOOL, w.HANDLE, c.POINTER(w.DWORD))
terminate = api('TerminateProcess', w.BOOL, w.HANDLE, w.UINT)


def checked(ok):
    if not ok:
        raise c.WinError(c.get_last_error())


class Terminal:
    def __init__(self, args):
        self.output = b''
        self.messages = queue.Queue()
        self.handles = [w.HANDLE() for _ in range(4)]
        input_read, self.input, self.output_read, output_write = self.handles
        checked(pipe(c.byref(input_read), c.byref(self.input), None, 0))
        checked(pipe(c.byref(self.output_read), c.byref(output_write), None, 0))
        self.pty = P()
        result = create_pty(Coord(120, 30), input_read, output_write, 0, c.byref(self.pty))
        assert result == 0, f'CreatePseudoConsole failed: {result}'
        size = SIZE()
        init_attrs(None, 1, 0, c.byref(size))
        self.attrs = c.create_string_buffer(size.value)
        checked(init_attrs(self.attrs, 1, 0, c.byref(size)))
        checked(set_attr(self.attrs, 0, 0x00020016, self.pty, c.sizeof(P), None, None))
        startup = StartupEx()
        startup.startup.cb = c.sizeof(StartupEx)
        startup.attributes = c.cast(self.attrs, P)
        self.process = Process()
        command = c.create_unicode_buffer(subprocess.list2cmdline(args))
        checked(create_process(None, command, None, None, False, 0x00080000, None, None, c.byref(startup), c.byref(self.process)))
        close(input_read); close(output_write)
        self.handles = [self.input, self.output_read, self.process.process, self.process.thread]
        self.reader = threading.Thread(target=self.drain, daemon=True)
        self.reader.start()

    def drain(self):
        buffer, count = c.create_string_buffer(65536), w.DWORD()
        while read(self.output_read, buffer, len(buffer), c.byref(count), None) and count.value:
            self.messages.put(buffer.raw[:count.value])

    def expect(self, text):
        deadline = time.monotonic() + 15
        while text.encode() not in self.output:
            if time.monotonic() > deadline:
                raise AssertionError(f'ConPTY did not render {text!r}')
            try:
                self.output += self.messages.get(timeout=0.1)
            except queue.Empty:
                pass

    def send(self, text):
        data, count = text.encode(), w.DWORD()
        checked(write(self.input, data, len(data), c.byref(count), None))

    def finish(self, keys, expected=0):
        self.send(keys)
        assert wait(self.process.process, 10000) == 0, 'Terminal process did not exit'
        code = w.DWORD()
        checked(exit_code(self.process.process, c.byref(code)))
        assert code.value == expected, f'Unexpected terminal exit: {code.value}'

    def __enter__(self):
        return self

    def __exit__(self, *_):
        if wait(self.process.process, 0) != 0:
            terminate(self.process.process, 1)
            wait(self.process.process, 5000)
        close_pty(self.pty)
        self.reader.join(timeout=3)
        delete_attrs(self.attrs)
        for handle in self.handles:
            close(handle)


def check(binary):
    with tempfile.TemporaryDirectory(prefix='leanflip-conpty-') as root:
        home = Path(root) / '用户 Home'
        args = [str(binary), '--home', str(home), '--lang', 'en']
        provider = {'id': 'windows-fixture', 'name': 'Windows Fixture', 'settingsConfig': {'env': {
            'ANTHROPIC_BASE_URL': 'https://windows.invalid', 'ANTHROPIC_AUTH_TOKEN': 'ci-dummy-key',
        }}}
        subprocess.run([*args, 'provider', 'add', 'claude', '--json'], input=json.dumps(provider), text=True, capture_output=True, check=True)
        subprocess.run([*args, 'use', 'claude', 'windows-fixture', '--json'], capture_output=True, check=True)
        before = {str(p.relative_to(home)): p.read_bytes() for p in home.rglob('*') if p.is_file()}
        with Terminal([*args, 'menu', 'claude']) as terminal:
            terminal.expect('Windows Fixture')
            terminal.finish('\x1b')
        with Terminal([*args, 'tui', 'claude']) as terminal:
            terminal.expect('LeanFlip')
            terminal.expect('Windows Fixture')
            terminal.finish('q')
        with Terminal([*args, 'tui', 'claude']) as terminal:
            terminal.expect('LeanFlip')
            terminal.finish('\x03', expected=130)
        after = {str(p.relative_to(home)): p.read_bytes() for p in home.rglob('*') if p.is_file()}
        assert before == after, 'Browsing/exit changed configuration'
    print('Windows ConPTY menu, TUI, normal exit, Ctrl+C and read-only browsing passed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    check(parser.parse_args().binary.resolve())
