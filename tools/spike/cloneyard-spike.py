#!/usr/bin/env python3
"""Checks the Claude Code behaviour Cloneyard relies on; README.md next to this file says how to run it."""

import argparse
import base64
import ctypes
import gzip
import json
import os
import platform
import re
import select
import shlex
import shutil
import signal
import struct
import subprocess
import sys
import threading
import time
import uuid
import warnings
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

SPIKE_VERSION = "0.3.0"
MIN_CC_FOR_OSC7501 = (2, 1, 295)
PONG_PROMPT = "Reply with exactly one word: pong"
MARK = "CY-MARK-7"
REFRESH_ERR = "failed to refresh oauth token"
TELEMETRY_ENV = (
    "CLAUDE_CODE_ENABLE_TELEMETRY", "OTEL_METRICS_EXPORTER", "OTEL_LOGS_EXPORTER", "OTEL_TRACES_EXPORTER",
    "OTEL_EXPORTER_OTLP_PROTOCOL", "OTEL_EXPORTER_OTLP_ENDPOINT", "DISABLE_TELEMETRY",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
)
AUTH_ACCOUNT_IDS = {"orgId": "<org id>", "orgName": "<org name>"}
OTEL_ACCOUNT_IDS = {
    "organization.id": "<org id>", "user.account_uuid": "<account uuid>",
    "user.account_id": "<account id>", "user.id": "<user id>",
}

# --------------------------------------------------------------------------- results


class Result:
    def __init__(self, test_id):
        self.id = test_id
        self.status = "PASS"
        self.summary = ""
        self.checks = []  # (name, ok|None, note, critical)
        self.data = {}
        self.started = time.time()
        self.seconds = 0.0

    def check(self, name, ok, note="", critical=True):
        self.checks.append({"name": name, "ok": ok, "note": note, "critical": critical})
        return ok

    def finish(self, summary=None):
        self.seconds = round(time.time() - self.started, 1)
        if self.status in ("SKIP", "INFO"):
            pass
        elif any(c["ok"] is False and c["critical"] for c in self.checks):
            self.status = "FAIL"
        elif any(c["ok"] is False for c in self.checks):
            self.status = "WARN"
        if summary is not None:
            self.summary = summary
        elif not self.summary:
            bad = [c["name"] for c in self.checks if c["ok"] is False]
            self.summary = ("failed: " + ", ".join(bad)) if bad else "all checks passed"
        return self

    def skip(self, why):
        self.status = "SKIP"
        self.summary = why
        return self.finish()

    def as_dict(self):
        return {
            "id": self.id,
            "status": self.status,
            "summary": self.summary,
            "seconds": self.seconds,
            "checks": self.checks,
            "data": self.data,
        }


# --------------------------------------------------------------------------- context


class Ctx:
    def __init__(self, args):
        self.args = args
        self.claude = args.claude or shutil.which("claude")
        self.model = args.model
        self.config_dir = os.path.abspath(os.path.expanduser(args.config_dir)) if args.config_dir else None
        stamp = time.strftime("%Y%m%d-%H%M%S")
        self.out = Path(args.out or f"cloneyard-spike-{stamp}").resolve()
        self.work = self.out / "work"
        self.out.mkdir(parents=True, exist_ok=True)
        self.work.mkdir(exist_ok=True)
        self.version = None
        self.help_main = ""
        self.help_agents = ""
        self.manual_mode = "manual"
        self.logged_in = None  # None = unknown
        self.model_ok = None  # set by the model_probe test
        self.model_err = ""
        self.secrets = []
        self.account_ids = {}
        self.repos = []  # (path, config_dir) to purge from Claude Code at the end
        tok = os.environ.get("CLAUDE_CODE_OAUTH_TOKEN")
        if tok:
            self.secrets.append(tok)
        key = os.environ.get("ANTHROPIC_API_KEY")
        if key:
            self.secrets.append(key)

    def env(self, extra=None, config_dir=None, pty=False):
        """The user's env without what makes Claude Code think it runs inside another session, or bill an API key."""
        env = {}
        for k, v in os.environ.items():
            if k.startswith("CLAUDE_CODE_") and k != "CLAUDE_CODE_OAUTH_TOKEN":
                continue
            if k in ("CLAUDECODE", "CLAUDE_PID", "CLAUDE_CONFIG_DIR") or k.startswith("OTEL_"):
                continue
            if k == "ANTHROPIC_API_KEY" and not self.args.keep_api_key:
                continue
            if pty and k in (
                "TMUX", "TMUX_PANE", "STY", "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "ZELLIJ",
                "KITTY_WINDOW_ID", "WEZTERM_PANE", "VTE_VERSION", "WT_SESSION",
            ):
                continue
            env[k] = v
        env["DISABLE_AUTOUPDATER"] = "1"
        for k in ("NO_PROXY", "no_proxy"):  # the OTel receiver is local; keep proxies out of it
            cur = env.get(k, "")
            env[k] = ",".join(x for x in (cur, "127.0.0.1", "localhost") if x)
        cfg = config_dir or self.config_dir
        if cfg:
            env["CLAUDE_CONFIG_DIR"] = cfg
        if pty:
            env["TERM"] = "xterm-256color"
            env["COLORTERM"] = "truecolor"
        if extra:
            env.update(extra)
        return env

    def repo(self, name, config_dir=None):
        d = self.work / name
        self.repos.append((d, config_dir))
        if d.exists():
            shutil.rmtree(d)
        d.mkdir(parents=True)
        (d / "README.md").write_text("# cloneyard spike scratch repo\n")
        if shutil.which("git"):
            subprocess.run(["git", "init", "-q"], cwd=d, check=False)
        return d

    def mask(self, text):
        for value, label in self.account_ids.items():
            text = text.replace(value, label)
        for s in self.secrets:
            if s and len(s) > 8:
                text = text.replace(s, s[:6] + "***")
        text = re.sub(r"sk-ant-[A-Za-z0-9_\-]{8,}", "sk-ant-***", text)
        text = re.sub(
            r"([A-Za-z0-9._%+\-])[A-Za-z0-9._%+\-]*@([A-Za-z0-9.\-]+\.[A-Za-z]{2,})",
            r"\1***@\2",
            text,
        )
        return text


def run(argv, env=None, cwd=None, timeout=60, stdin_data=None):
    t0 = time.time()
    try:
        p = subprocess.run(
            argv, env=env, cwd=cwd, timeout=timeout, capture_output=True,
            input=stdin_data.encode() if stdin_data is not None else None,
        )
        return p.returncode, p.stdout.decode(errors="replace"), p.stderr.decode(errors="replace"), time.time() - t0
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode(errors="replace")
        err = (e.stderr or b"").decode(errors="replace")
        return None, out, err + "\n[spike] timed out", time.time() - t0
    except FileNotFoundError as e:
        return 127, "", str(e), 0.0


def tail(text, n=1200):
    text = text or ""
    return text if len(text) <= n else "..." + text[-n:]


def parse_version(s):
    m = re.search(r"(\d+)\.(\d+)\.(\d+)", s or "")
    return tuple(int(x) for x in m.groups()) if m else None


def wait_until(pred, timeout, step=0.25):
    end = time.time() + timeout
    while time.time() < end:
        v = pred()
        if v:
            return v
        time.sleep(step)
    return pred()


# --------------------------------------------------------------------------- hooks plugin

HOOK_PY = r'''#!/usr/bin/env python3
import json, os, sys, time
src, log = sys.argv[1], sys.argv[2]
TELEMETRY = %s
try:
    data = json.load(sys.stdin)
except Exception:
    data = {}
rec = {
    "t": time.time(), "source": src, "event": data.get("hook_event_name"),
    "session_id": data.get("session_id"), "transcript_path": data.get("transcript_path"),
    "cwd": data.get("cwd"), "tool_name": data.get("tool_name"), "keys": sorted(data.keys()),
    "config_dir_env": os.environ.get("CLAUDE_CONFIG_DIR"),
    "telemetry_env": {k: (v if k != "OTEL_EXPORTER_OTLP_ENDPOINT" or v.startswith("http://127.0.0.1") else "<not ours>")
                      for k, v in os.environ.items() if k in TELEMETRY},
}
with open(log, "a") as f:
    f.write(json.dumps(rec) + "\n")
'''

HOOK_PY = HOOK_PY % (repr(TELEMETRY_ENV),)

HOOK_EVENTS = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Notification", "Stop", "SessionEnd"]
TOOL_EVENTS = ("PreToolUse", "PostToolUse")


def hook_block(command):
    hooks = {}
    for ev in HOOK_EVENTS:
        entry = {"hooks": [{"type": "command", "command": command}]}
        if ev in TOOL_EVENTS:
            entry["matcher"] = "*"
        hooks[ev] = [entry]
    return hooks


def make_plugin(ctx, name, log_path):
    root = ctx.work / name
    if root.exists():
        shutil.rmtree(root)
    (root / ".claude-plugin").mkdir(parents=True)
    (root / "hooks").mkdir()
    hook_py = root / "hook.py"
    hook_py.write_text(HOOK_PY)
    hook_py.chmod(0o755)
    (root / ".claude-plugin" / "plugin.json").write_text(json.dumps(
        {"name": "cy-spike", "version": "0.0.1", "description": "Cloneyard spike hooks"}, indent=2))
    cmd = f"{shlex.quote(sys.executable)} {shlex.quote(str(hook_py))} plugin {shlex.quote(str(log_path))}"
    (root / "hooks" / "hooks.json").write_text(json.dumps({"hooks": hook_block(cmd)}, indent=2))
    return root, hook_py


def settings_hooks(hook_py, log_path):
    cmd = f"{shlex.quote(sys.executable)} {shlex.quote(str(hook_py))} settings {shlex.quote(str(log_path))}"
    return {"hooks": hook_block(cmd)}


def read_jsonl(path):
    out = []
    try:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line:
                    try:
                        out.append(json.loads(line))
                    except ValueError:
                        pass
    except FileNotFoundError:
        pass
    return out


# --------------------------------------------------------------------------- OTel receiver


def _attr_value(v):
    for k in ("stringValue", "intValue", "doubleValue", "boolValue"):
        if k in v:
            return v[k]
    if "arrayValue" in v:
        return [_attr_value(x) for x in v["arrayValue"].get("values", [])]
    return v


def _attrs(lst):
    return {a.get("key"): _attr_value(a.get("value", {})) for a in (lst or [])}


class OtelReceiver:
    """Minimal OTLP/HTTP receiver: stores every POST, understands the JSON encoding."""

    def __init__(self, path):
        self.path = path
        self.posts = []
        self.lock = threading.Lock()
        rec = self

        class H(BaseHTTPRequestHandler):
            def do_POST(self):
                n = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(n)
                if (self.headers.get("Content-Encoding") or "").lower() == "gzip":
                    try:
                        body = gzip.decompress(body)
                    except OSError:
                        pass
                ctype = self.headers.get("Content-Type") or ""
                try:
                    data = json.loads(body)
                except ValueError:
                    data = None
                item = {"t": time.time(), "path": self.path, "ctype": ctype, "bytes": len(body), "data": data}
                with rec.lock:
                    rec.posts.append(item)
                    with open(rec.path, "a") as f:
                        f.write(json.dumps(item) + "\n")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(b"{}")

            def log_message(self, *a):
                pass

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.port = self.srv.server_address[1]
        self.thread = threading.Thread(target=self.srv.serve_forever, daemon=True)
        self.thread.start()

    def stop(self):
        self.srv.shutdown()
        self.srv.server_close()

    def env(self, traces=True):
        e = {
            "CLAUDE_CODE_ENABLE_TELEMETRY": "1",
            "OTEL_METRICS_EXPORTER": "otlp",
            "OTEL_LOGS_EXPORTER": "otlp",
            "OTEL_EXPORTER_OTLP_PROTOCOL": "http/json",
            "OTEL_EXPORTER_OTLP_ENDPOINT": f"http://127.0.0.1:{self.port}",
            "OTEL_METRIC_EXPORT_INTERVAL": "2000",
            "OTEL_LOGS_EXPORT_INTERVAL": "1000",
            "OTEL_LOG_TOOL_DETAILS": "1",
        }
        if traces:
            e["CLAUDE_CODE_ENHANCED_TELEMETRY_BETA"] = "1"
            e["OTEL_TRACES_EXPORTER"] = "otlp"
        return e

    def _snapshot(self):
        with self.lock:
            return list(self.posts)

    def events(self):
        out = []
        for p in self._snapshot():
            d = p["data"]
            if not p["path"].endswith("/v1/logs") or not isinstance(d, dict):
                continue
            for rl in d.get("resourceLogs", []):
                for sl in rl.get("scopeLogs", []):
                    for lr in sl.get("logRecords", []):
                        a = _attrs(lr.get("attributes"))
                        body = lr.get("body", {})
                        name = a.get("event.name") or (body.get("stringValue") if isinstance(body, dict) else None)
                        out.append((name, a))
        return out

    def metric_names(self):
        names = set()
        for p in self._snapshot():
            d = p["data"]
            if not p["path"].endswith("/v1/metrics") or not isinstance(d, dict):
                continue
            for rm in d.get("resourceMetrics", []):
                for sm in rm.get("scopeMetrics", []):
                    for m in sm.get("metrics", []):
                        names.add(m.get("name"))
        return names

    def count(self, suffix):
        return sum(1 for p in self._snapshot() if p["path"].endswith(suffix))

    def non_json(self):
        return [p["ctype"] for p in self._snapshot() if p["data"] is None]


# --------------------------------------------------------------------------- stream-json session


class StreamSession:
    def __init__(self, argv, env, cwd, out_path, err_path):
        self.out_path = out_path
        self.msgs = []
        self.lock = threading.Lock()
        self.err_f = open(err_path, "wb")
        self.p = subprocess.Popen(
            argv, env=env, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.err_f,
        )
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()

    def _read(self):
        with open(self.out_path, "a") as f:
            for raw in self.p.stdout:
                line = raw.decode(errors="replace").strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except ValueError:
                    obj = {"type": "_unparsed", "line": line}
                f.write(json.dumps(redact_init(obj)) + "\n")
                f.flush()
                with self.lock:
                    self.msgs.append((time.time(), obj))

    def send_user(self, text):
        msg = {"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}}
        self.p.stdin.write((json.dumps(msg) + "\n").encode())
        self.p.stdin.flush()

    def snapshot(self):
        with self.lock:
            return [m for _, m in self.msgs]

    def of_type(self, t, subtype=None):
        return [m for m in self.snapshot() if m.get("type") == t and (subtype is None or m.get("subtype") == subtype)]

    def wait_results(self, n, timeout):
        return wait_until(lambda: len(self.of_type("result")) >= n or self.p.poll() is not None, timeout)

    def close(self, timeout=30):
        try:
            self.p.stdin.close()
        except OSError:
            pass
        try:
            self.p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.p.terminate()
            try:
                self.p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.p.kill()
        self.thread.join(timeout=5)
        self.err_f.close()
        return self.p.returncode


def redact_init(obj):
    """system/init lists the user's own commands, skills, agents and plugins: keep only their counts."""
    if obj.get("type") != "system" or obj.get("subtype") != "init":
        return obj
    out = dict(obj)
    for k in ("slash_commands", "skills", "agents", "plugins", "mcp_servers", "terminal_slash_commands", "memory_paths"):
        if isinstance(out.get(k), (list, dict)):
            out[k] = f"<{len(out[k])} items>"
    return out


def refresh_error(text):
    return REFRESH_ERR in (text or "").lower()


def tool_results(msgs):
    """(is_error, text) for every tool_result block the CLI echoed back."""
    out = []
    for m in msgs:
        if m.get("type") != "user":
            continue
        content = (m.get("message") or {}).get("content")
        if not isinstance(content, list):
            continue
        for b in content:
            if isinstance(b, dict) and b.get("type") == "tool_result":
                c = b.get("content")
                if isinstance(c, list):
                    c = " ".join(x.get("text", "") for x in c if isinstance(x, dict))
                out.append((bool(b.get("is_error")), str(c)[:400]))
    return out


# --------------------------------------------------------------------------- PTY driver

ANSI_CUF = re.compile(rb"\x1b\[\d*C")
ANSI_ALL = re.compile(
    rb"\x1b\[[0-?]*[ -/]*[@-~]"  # CSI
    rb"|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)"  # OSC
    rb"|\x1b[P_^][^\x1b]*\x1b\\"  # DCS, APC (kitty graphics), PM
    rb"|\x1b[@-Z\\-_]"
)
OSC7501 = re.compile(rb"\x1b\]7501;([^\x07\x1b]*)(?:\x07|\x1b\\)")
QUERIES = re.compile(
    rb"\x1b\]7501;\?(?:\x07|\x1b\\)"  # program status probe
    rb"|\x1b\[0?c"  # DA1
    rb"|\x1b\[>0?q"  # XTVERSION
    rb"|\x1b\[6n"  # cursor position
    rb"|\x1b\]1[01];\?(?:\x07|\x1b\\)"  # fg/bg colour
    rb"|\x1b\[\?(\d+)\$p"  # DECRQM
)


def decode_7501(payload):
    rec = {}
    for part in payload.decode(errors="replace").split(":"):
        if "=" not in part:
            continue
        k, v = part.split("=", 1)
        if k in ("msg", "title") and v:
            try:
                v = base64.b64decode(v + "=" * (-len(v) % 4)).decode(errors="replace")
            except ValueError:
                pass
        rec[k] = v
    return rec


class Pty:
    """Runs a program on a PTY and answers the terminal queries a TUI sends at startup, OSC 7501 included."""

    def __init__(self, argv, env, cwd, log_path, cols=120, rows=40, answer_7501=True):
        import pty
        import fcntl
        import termios

        self.answer_7501 = answer_7501
        self.buf = bytearray()
        self.lock = threading.Lock()
        self.reports = []  # (t, dict)
        self.probe_seen = False
        self.log = open(log_path, "wb")
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            pid, fd = pty.fork()
        if pid == 0:  # child
            try:
                os.chdir(cwd)
                os.execvpe(argv[0], argv, env)
            finally:
                os._exit(127)
        self.pid, self.fd = pid, fd
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self._q_pos = 0
        self._r_pos = 0
        self.exit_status = None
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()

    def _read(self):
        while True:
            try:
                r, _, _ = select.select([self.fd], [], [], 0.2)
                if not r:
                    if self._exited():
                        break
                    continue
                chunk = os.read(self.fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            self.log.write(chunk)
            self.log.flush()
            with self.lock:
                self.buf += chunk
                self._answer()
                self._parse_reports()
        self._exited(block=True)

    def _exited(self, block=False):
        if self.exit_status is not None:
            return True
        try:
            pid, status = os.waitpid(self.pid, 0 if block else os.WNOHANG)
        except ChildProcessError:
            self.exit_status = -1
            return True
        if pid:
            self.exit_status = status
            return True
        return False

    def _answer(self):
        start = max(0, self._q_pos)
        last = None
        for m in QUERIES.finditer(self.buf, start):
            q = m.group(0)
            last = m.end()
            reply = None
            if q.startswith(b"\x1b]7501"):
                self.probe_seen = True
                if self.answer_7501:
                    reply = b"\x1b]7501;?\x1b\\"
            elif q.endswith(b"c"):
                reply = b"\x1b[?62;22c"
            elif q.endswith(b"q"):
                reply = b"\x1bP>|cloneyard-spike " + SPIKE_VERSION.encode() + b"\x1b\\"
            elif q == b"\x1b[6n":
                reply = b"\x1b[1;1R"
            elif q.startswith(b"\x1b]11"):
                reply = b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\"
            elif q.startswith(b"\x1b]10"):
                reply = b"\x1b]10;rgb:d4d4/d4d4/d4d4\x1b\\"
            elif q.endswith(b"$p"):
                reply = b"\x1b[?" + m.group(1) + b";2$y"
            if reply:
                self._write(reply)
        self._q_pos = last if last is not None else max(self._q_pos, len(self.buf) - 32)

    def _parse_reports(self):
        last = None
        for m in OSC7501.finditer(self.buf, self._r_pos):
            last = m.end()
            payload = m.group(1)
            if payload.startswith(b"?"):
                continue
            self.reports.append((time.time(), decode_7501(payload)))
        self._r_pos = last if last is not None else max(self._r_pos, len(self.buf) - 4096)

    def _write(self, data):
        try:
            os.write(self.fd, data)
        except OSError:
            pass

    def type(self, text):
        self._write(text.encode())

    def paste(self, text, enter=True):
        self._write(b"\x1b[200~" + text.encode() + b"\x1b[201~")
        if enter:
            time.sleep(0.4)
            self._write(b"\r")

    def screen(self, last_bytes=20000):
        with self.lock:
            raw = bytes(self.buf[-last_bytes:])
        raw = ANSI_CUF.sub(b" ", raw)
        return ANSI_ALL.sub(b"", raw).decode(errors="replace")

    def raw(self):
        with self.lock:
            return bytes(self.buf)

    def states(self, since=0.0):
        with self.lock:
            return [r for t, r in self.reports if t >= since and not r.get("id")]

    def timed_states(self, since=0.0):
        with self.lock:
            return [(t, r) for t, r in self.reports if t >= since and not r.get("id")]

    def wait_state_t(self, pred, timeout, since=0.0):
        """First root report matching pred at or after `since`, as (time, report)."""
        return wait_until(lambda: next(((t, r) for t, r in self.timed_states(since) if pred(r)), None), timeout)

    def wait_state(self, pred, timeout, since=0.0):
        hit = self.wait_state_t(pred, timeout, since)
        return hit[1] if hit else None

    def idle_for(self, seconds):
        """True when the latest root reports are all idle, the first of them at least `seconds` ago."""
        start = None
        for t, r in reversed(self.timed_states()):
            if r.get("state") != "idle":
                break
            start = t
        return start is not None and time.time() - start >= seconds

    def size(self):
        with self.lock:
            return len(self.buf)

    def screen_since(self, offset):
        with self.lock:
            raw = bytes(self.buf[offset:])
        raw = ANSI_CUF.sub(b" ", raw)
        return ANSI_ALL.sub(b"", raw).decode(errors="replace")

    def close(self, grace=8):
        if self.exit_status is None:
            self.type("/exit")
            time.sleep(0.5)
            self._write(b"\r")
            wait_until(lambda: self.exit_status is not None, grace)
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGKILL):
            if self.exit_status is not None:
                break
            try:
                os.kill(self.pid, sig)
            except ProcessLookupError:
                break
            wait_until(lambda: self.exit_status is not None, 3)
        self.thread.join(timeout=3)
        self.log.close()
        try:
            os.close(self.fd)
        except OSError:
            pass


DIALOGS = [
    # (name, words that must all be on screen ignoring spaces and case, the row to pick or None for the default)
    ("trust", ("trust", "folder"), "yes"),
    ("trust", ("trust", "proceed"), "yes"),
    ("theme", ("theme", "dark"), None),
    ("renderer", ("fullscreenrenderer", "notnow"), "notnow"),
    ("remote-control", ("remotecontrol", "enable"), None),
    ("continue", ("pressenter",), None),
]
CURSOR = ("\u276f", ">")  # the selected row of a menu starts with one of these


def _selected_row(norm):
    """Text right after the last menu cursor without its number, e.g. 'no,exit' or 'notnow'."""
    best = None
    for c in CURSOR:
        i = norm.rfind(c)
        if i >= 0 and (best is None or i > best):
            best = i
    return re.sub(r"^\d+\.", "", norm[best + 1:best + 16]) if best is not None else ""


def _pick_row(p, mark, row):
    """Moves the menu cursor down to the row starting with `row`, then presses Enter."""
    for _ in range(4):
        if _selected_row(re.sub(r"\s+", "", p.screen_since(mark).lower())).startswith(row):
            break
        mark = p.size()
        p._write(b"\x1b[B")  # down
        time.sleep(0.7)
    time.sleep(0.3)
    p._write(b"\r")


def handle_startup_dialogs(p, timeout, ready):
    """Answers first-run screens until `ready()`, reading only what was printed since the last key press."""
    pressed, mark, last = [], 0, 0.0
    end = time.time() + timeout
    while time.time() < end:
        if ready():
            return True, pressed
        if p.exit_status is not None:
            return False, pressed
        if time.time() - last > 1.5:
            norm = re.sub(r"\s+", "", p.screen_since(mark).lower())
            for name, words, row in DIALOGS:
                if all(w in norm for w in words) and pressed.count(name) < 2:
                    time.sleep(0.5)
                    start = mark
                    mark = p.size()
                    if row:
                        _pick_row(p, start, row)
                        mark = p.size()
                    else:
                        p._write(b"\r")
                    pressed.append(name)
                    last = time.time()
                    break
        time.sleep(0.3)
    return bool(ready()), pressed


def trust_folder(ctx, folder, config_dir=None):
    """Accepts the trust dialog once, because `claude --bg` refuses an untrusted folder; no model call."""
    p = Pty([ctx.claude], ctx.env(pty=True, config_dir=config_dir), folder, ctx.out / "trust-pty.log")
    try:
        ok, pressed = handle_startup_dialogs(p, 45, lambda: p.idle_for(2))
    finally:
        p.close()
    return ok, pressed


# --------------------------------------------------------------------------- Landlock

LL_EXECUTE, LL_WRITE_FILE, LL_READ_FILE, LL_READ_DIR = 1 << 0, 1 << 1, 1 << 2, 1 << 3
LL_FS_V1 = (1 << 13) - 1
LL_REFER, LL_TRUNCATE = 1 << 13, 1 << 14
LL_FILE_RIGHTS = LL_EXECUTE | LL_WRITE_FILE | LL_READ_FILE | LL_TRUNCATE
SYS_CREATE, SYS_ADD, SYS_RESTRICT = 444, 445, 446  # same on x86_64, aarch64, riscv64


def _libc():
    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long
    return libc


def landlock_abi():
    try:
        libc = _libc()
        v = libc.syscall(SYS_CREATE, None, ctypes.c_size_t(0), ctypes.c_uint32(1))
        if v < 0:
            return -ctypes.get_errno()
        return v
    except (OSError, AttributeError):
        return None


def landlock_apply(ro_paths, rw_paths):
    abi = landlock_abi()
    if not abi or abi < 1:
        raise OSError(f"Landlock unavailable (abi={abi})")
    handled = LL_FS_V1 | (LL_REFER if abi >= 2 else 0) | (LL_TRUNCATE if abi >= 3 else 0)

    class RulesetAttr(ctypes.Structure):
        _fields_ = [("handled_access_fs", ctypes.c_uint64)]

    class PathBeneath(ctypes.Structure):
        _pack_ = 1
        _fields_ = [("allowed_access", ctypes.c_uint64), ("parent_fd", ctypes.c_int32)]

    libc = _libc()
    attr = RulesetAttr(handled)
    rfd = libc.syscall(SYS_CREATE, ctypes.byref(attr), ctypes.c_size_t(ctypes.sizeof(attr)), ctypes.c_uint32(0))
    if rfd < 0:
        raise OSError(ctypes.get_errno(), "landlock_create_ruleset")
    ro = LL_EXECUTE | LL_READ_FILE | LL_READ_DIR
    for paths, access in ((ro_paths, ro), (rw_paths, handled)):
        for path in paths:
            if not os.path.exists(path):
                continue
            acc = access & handled
            if not os.path.isdir(path):
                acc &= LL_FILE_RIGHTS
            pfd = os.open(path, os.O_PATH | os.O_CLOEXEC)
            rule = PathBeneath(acc, pfd)
            r = libc.syscall(SYS_ADD, ctypes.c_int(rfd), ctypes.c_int(1), ctypes.byref(rule), ctypes.c_uint32(0))
            os.close(pfd)
            if r < 0:
                raise OSError(ctypes.get_errno(), f"landlock_add_rule {path}")
    if libc.prctl(38, 1, 0, 0, 0) != 0:  # PR_SET_NO_NEW_PRIVS
        raise OSError(ctypes.get_errno(), "prctl(NO_NEW_PRIVS)")
    if libc.syscall(SYS_RESTRICT, ctypes.c_int(rfd), ctypes.c_uint32(0)) < 0:
        raise OSError(ctypes.get_errno(), "landlock_restrict_self")
    return abi


def landlock_exec_main(argv):
    """Internal mode: --_landlock-exec [--rw P]... -- CMD ARGS..."""
    rw, i = [], 0
    while i < len(argv) and argv[i] != "--":
        if argv[i] == "--rw":
            rw.append(argv[i + 1])
            i += 2
        else:
            i += 1
    cmd = argv[i + 1:]
    try:
        landlock_apply(["/"], rw)
    except OSError as e:
        sys.stderr.write(f"[spike] landlock: {e}\n")
        os._exit(97)
    os.execvp(cmd[0], cmd)


def landlock_wrap(rw_paths, cmd):
    argv = [sys.executable, os.path.abspath(__file__), "--_landlock-exec"]
    for p in rw_paths:
        argv += ["--rw", str(p)]
    return argv + ["--"] + list(cmd)


# --------------------------------------------------------------------------- tests


def t_preflight(ctx, r):
    if not ctx.claude:
        r.check("claude on PATH", False, "install Claude Code or pass --claude")
        return r.finish("claude not found")
    r.data["claude"] = ctx.claude
    rc, out, err, _ = run([ctx.claude, "--version"], env=ctx.env())
    ctx.version = parse_version(out)
    r.data["version"] = out.strip()
    r.check("claude --version", rc == 0 and ctx.version is not None, tail(err))
    r.data["python"] = sys.version.split()[0]
    r.data["kernel"] = platform.release()
    r.data["arch"] = platform.machine()
    try:
        osr = dict(
            line.split("=", 1) for line in Path("/etc/os-release").read_text().splitlines() if "=" in line
        )
        r.data["os"] = osr.get("PRETTY_NAME", "").strip('"')
    except OSError:
        pass
    _, ctx.help_main, _, _ = run([ctx.claude, "--help"], env=ctx.env())
    _, ctx.help_agents, _, _ = run([ctx.claude, "agents", "--help"], env=ctx.env())
    if '"manual"' not in ctx.help_main and '"default"' in ctx.help_main:
        ctx.manual_mode = "default"
    r.data["manual_permission_mode"] = ctx.manual_mode
    flags = [
        "--bg", "--print", "--input-format", "--output-format", "--verbose", "--session-id", "--settings",
        "--plugin-dir", "--json-schema", "--permission-prompts", "--permission-mode", "--model", "--tools",
        "--allowedTools", "--remote-control", "--include-hook-events", "--append-system-prompt", "--worktree",
    ]
    missing = [f for f in flags if f not in ctx.help_main]
    r.check("flags in claude --help", not missing, "missing: " + ", ".join(missing) if missing else "")
    if "--append-system-prompt-file" not in ctx.help_main:
        r.data["note_hidden_flags"] = "--append-system-prompt-file is not listed in --help (documented, tested in headless)"
    cmds = ["agents", "attach", "logs", "stop", "rm", "setup-token", "auth"]
    missing_cmds = [c for c in cmds if not re.search(rf"^\s+{re.escape(c)}\b", ctx.help_main, re.M)]
    r.check("subcommands", not missing_cmds, "missing: " + ", ".join(missing_cmds) if missing_cmds else "")
    amissing = [f for f in ("--json", "--all", "--cwd") if f not in ctx.help_agents]
    r.check("claude agents flags", not amissing, ", ".join(amissing), critical=False)
    if ctx.version:
        r.check(
            f"version >= {'.'.join(map(str, MIN_CC_FOR_OSC7501))} (OSC 7501)",
            ctx.version >= MIN_CC_FOR_OSC7501, "", critical=False,
        )
    rc, out, err, _ = run([ctx.claude, "agents", "--json"], env=ctx.env(), timeout=30)
    n_sessions = 0
    try:
        lst = json.loads(out)
        r.data["agents_json_fields"] = sorted({k for e in lst if isinstance(e, dict) for k in e})
        n_sessions = len(lst) if isinstance(lst, list) else 0
        r.check("claude agents --json parses", isinstance(lst, list), f"{len(lst)} active session(s)", critical=False)
    except ValueError:
        r.check("claude agents --json parses", False, tail(out + err, 300), critical=False)
    r.data["proxy_env_set"] = sorted(k for k in os.environ if k.lower() in ("http_proxy", "https_proxy", "all_proxy", "no_proxy"))
    cfg = Path(ctx.config_dir) if ctx.config_dir else Path.home() / ".claude"
    seen = {}
    for f in (cfg / "settings.json", Path("/etc/claude-code/managed-settings.json")):
        try:
            env_keys = sorted(k for k in (json.loads(f.read_text()).get("env") or {}) if k in TELEMETRY_ENV)
            seen[str(f)] = env_keys
        except (OSError, ValueError, AttributeError):
            pass
    r.data["telemetry_keys_in_settings_env"] = seen
    if ctx.config_dir is None:
        n = n_sessions
        r.check("own config dir for tests", n == 0,
                f"{n} Claude Code session(s) share ~/.claude with the spike; OAuth refresh can clash. "
                "Use --config-dir ~/.local/share/cloneyard/accounts/<name>/claude", critical=False)
    return r.finish(f"Claude Code {r.data.get('version', '?')}, {r.data.get('os', platform.system())}")


def t_auth(ctx, r):
    rc, out, err, _ = run([ctx.claude, "auth", "status", "--json"], env=ctx.env(), timeout=30)
    try:
        st = json.loads(out)
    except ValueError:
        r.check("auth status --json", False, tail(out + err, 400), critical=False)
        return r.finish("could not read auth status")
    r.data["status"] = st
    ctx.logged_in = bool(st.get("loggedIn"))
    r.check("logged in", ctx.logged_in, "run `claude auth login` or set CLAUDE_CODE_OAUTH_TOKEN")
    key_src = st.get("apiKeySource")
    r.check("uses the plan, not an API key", not key_src, f"apiKeySource={key_src}" if key_src else "", critical=False)
    if os.environ.get("ANTHROPIC_API_KEY") and not ctx.args.keep_api_key:
        r.data["note"] = "ANTHROPIC_API_KEY is set in your shell; the spike removed it for child processes"
    return r.finish(f"{st.get('authMethod', '?')} via {st.get('apiProvider', '?')}, config {st.get('configDirectory', '?')}")


def t_model_probe(ctx, r):
    """One tiny call first, so an auth problem shows up once instead of in every test."""
    repo = ctx.repo("model-probe")
    argv = [ctx.claude, "-p", PONG_PROMPT, "--model", ctx.model, "--output-format", "json", "--tools", ""]
    for attempt in (1, 2):
        rc, out, err, secs = run(argv, env=ctx.env(), cwd=repo, timeout=ctx.args.timeout)
        try:
            res = json.loads(out)
        except ValueError:
            res = {"is_error": True, "result": tail(out + err, 300)}
        text = str(res.get("result", ""))
        r.data[f"attempt{attempt}"] = {"rc": rc, "is_error": res.get("is_error"), "subtype": res.get("subtype"),
                                       "terminal_reason": res.get("terminal_reason"), "result": text[:300],
                                       "seconds": round(secs, 1)}
        if not res.get("is_error") and "pong" in text.lower():
            ctx.model_ok = True
            r.check("model call works", True)
            return r.finish(f"{ctx.model} answers" + (" (after a retry)" if attempt == 2 else ""))
        if attempt == 1 and refresh_error(text):
            print(" OAuth refresh busy, retrying in 60 s...", end="", flush=True)
            time.sleep(60)
            continue
        break
    ctx.model_ok = False
    ctx.model_err = text[:160]
    hint = ""
    if refresh_error(text):
        hint = (" Another Claude Code process holds the OAuth refresh for this config dir. "
                "Close it or run with --config-dir pointing at a separate account dir.")
    r.check("model call works", False, text[:300] + hint)
    r.data["note"] = "is_error=true arrives with subtype=success: check is_error, not subtype"
    return r.finish("model call failing: " + ("OAuth refresh blocked" if hint else text[:80]))


def t_landlock(ctx, r):
    lsm = ""
    try:
        lsm = Path("/sys/kernel/security/lsm").read_text().strip()
    except OSError:
        pass
    r.data["lsm"] = lsm
    abi = landlock_abi()
    r.data["abi"] = abi
    if abi is None or abi <= 0:
        why = {-38: "kernel without Landlock (ENOSYS)", -95: "disabled at boot (EOPNOTSUPP)"}.get(abi, f"abi={abi}")
        r.check("Landlock available", False, why + "; fallback on the VPS: separate system user only")
        return r.finish(why)
    r.check("Landlock available", True, f"ABI v{abi}")
    r.check("ABI >= 3 (truncate)", abi >= 3, "older kernels miss truncate control", critical=False)
    box = ctx.work / "landlock-box"
    box.mkdir(exist_ok=True)
    outside = ctx.out / "landlock-outside.txt"
    probe = (
        "import sys\n"
        f"open({str(box / 'inside.txt')!r},'w').write('ok')\n"
        "try:\n"
        f"    open({str(outside)!r},'w').write('leak'); sys.exit(3)\n"
        "except PermissionError:\n"
        "    sys.exit(0)\n"
    )
    rc, out, err, _ = run(landlock_wrap([box], [sys.executable, "-I", "-c", probe]), timeout=30)
    r.check("write inside allowed dir works", (box / "inside.txt").exists(), tail(err, 300))
    r.check("write outside is blocked", rc == 0 and not outside.exists(), f"rc={rc} {tail(err, 300)}")
    if outside.exists():
        outside.unlink()
    return r.finish(f"ABI v{abi}, enforcement works" if rc == 0 else None)


def t_seccomp(ctx, r):
    try:
        status = Path("/proc/self/status").read_text()
        m = re.search(r"^Seccomp:\s*(\d+)", status, re.M)
        r.data["seccomp_field"] = m.group(1) if m else None
        r.check("seccomp present", m is not None, "", critical=False)
    except OSError as e:
        r.check("seccomp present", False, str(e), critical=False)
    r.status = "INFO" if r.checks and r.checks[0]["ok"] else r.status
    return r.finish("seccomp filter mode available" if r.data.get("seccomp_field") is not None else None)


def t_bwrap(ctx, r):
    bw, so = shutil.which("bwrap"), shutil.which("socat")
    r.data.update({"bwrap": bw, "socat": so})
    for k in (
        "/proc/sys/kernel/apparmor_restrict_unprivileged_userns",
        "/proc/sys/kernel/unprivileged_userns_clone",
        "/proc/sys/user/max_user_namespaces",
    ):
        try:
            r.data[os.path.basename(k)] = Path(k).read_text().strip()
        except OSError:
            pass
    r.check("bubblewrap installed", bool(bw), "apt install bubblewrap")
    r.check("socat installed", bool(so), "apt install socat")
    if bw:
        rc, out, err, _ = run([bw, "--unshare-all", "--share-net", "--ro-bind", "/", "/", "--dev", "/dev",
                               "--proc", "/proc", "true"], timeout=20)
        hint = ""
        if rc != 0 and r.data.get("apparmor_restrict_unprivileged_userns") == "1":
            hint = " (Ubuntu 24.04+: AppArmor blocks unprivileged user namespaces; needs a bwrap profile)"
        r.check("bwrap can create a sandbox", rc == 0, tail(err, 300) + hint)
    return r.finish()


def t_headless(ctx, r):
    repo = ctx.repo("headless")
    hook_log = ctx.out / "headless-hooks.jsonl"
    plugin, hook_py = make_plugin(ctx, "plugin-headless", hook_log)
    recv = OtelReceiver(ctx.out / "headless-otel.jsonl")
    append = ctx.work / "append.md"
    append.write_text(f"Cloneyard spike rule: end every reply with the exact token {MARK}.\n")
    sid = str(uuid.uuid4())
    argv = [
        ctx.claude, "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
        "--include-hook-events", "--model", ctx.model, "--session-id", sid,
        "--append-system-prompt-file", str(append),
        "--plugin-dir", str(plugin), "--settings", json.dumps(settings_hooks(hook_py, hook_log)),
        "--permission-prompts", "none", "--tools", "Bash", "--allowedTools", "Bash(echo *)",
    ]
    r.data["argv"] = argv
    s = StreamSession(argv, ctx.env(recv.env()), repo, ctx.out / "headless-stream.jsonl", ctx.out / "headless-stderr.txt")
    try:
        s.send_user("Use the Bash tool to run exactly: echo cy-spike-ok. Then reply with only the command output.")
        s.wait_results(1, ctx.args.timeout)
        alive_after_turn1 = s.p.poll() is None
        s.send_user("Now use the Bash tool to run exactly: touch cy-spike-denied.txt. Then reply in one word: did it work?")
        s.wait_results(2, ctx.args.timeout)
    finally:
        rc = s.close()
    wait_until(lambda: recv.count("/v1/logs") > 0, 8)
    time.sleep(2)
    recv.stop()
    msgs = s.snapshot()
    init = next((m for m in msgs if m.get("type") == "system" and m.get("subtype") == "init"), None)
    results = [m for m in msgs if m.get("type") == "result"]
    err_text = Path(ctx.out / "headless-stderr.txt").read_text(errors="replace")
    r.check("system/init received", init is not None, tail(err_text, 500))
    if init:
        r.data["init_keys"] = sorted(init.keys())
        r.data["init"] = {k: init.get(k) for k in ("model", "apiKeySource", "permissionMode", "cwd", "claude_code_version") if k in init}
        r.check("--session-id honoured", init.get("session_id") == sid, f"got {init.get('session_id')}")
        aks = init.get("apiKeySource")
        r.check("plan auth (apiKeySource none)", aks in (None, "none"), f"apiKeySource={aks}", critical=False)
    r.check("two turns on one process", len(results) >= 2 and alive_after_turn1, f"results={len(results)} rc={rc}")
    if results:
        r.data["result_keys"] = sorted(results[0].keys())
        r.data["turns"] = [
            {k: res.get(k) for k in ("subtype", "is_error", "num_turns", "total_cost_usd", "duration_ms", "result")}
            for res in results
        ]
        r.check("no error results", not any(x.get("is_error") for x in results), "", critical=False)
        r.check("--append-system-prompt-file applied", MARK in str(results[0].get("result", "")),
                f"turn 1 reply: {str(results[0].get('result', ''))[:120]!r}")
    tr = tool_results(msgs)
    r.data["tool_results"] = tr
    r.check("allowed Bash ran (echo)", any("cy-spike-ok" in t for _, t in tr), "")
    r.check("denied Bash did not run and did not hang", not (repo / "cy-spike-denied.txt").exists() and len(results) >= 2, "")
    rl = [m for m in msgs if m.get("type") == "rate_limit_event"]
    r.data["rate_limit_events"] = [m.get("rate_limit_info") for m in rl][:5]
    r.check("rate_limit_event in stream", bool(rl), "plan limit info for budgets", critical=False)
    retry = [m for m in msgs if m.get("type") == "system" and m.get("subtype") == "api_retry"]
    r.data["api_retry_events"] = retry[:3]
    hook_msgs = [m for m in msgs if m.get("type") == "system" and "hook" in str(m.get("subtype", ""))]
    r.data["hook_event_subtypes"] = sorted({m.get("subtype") for m in hook_msgs})
    r.check("--include-hook-events shows hooks in stream", bool(hook_msgs), "", critical=False)
    if any(refresh_error(str(x.get("result"))) for x in results):
        r.check("no OAuth refresh clash", False, "see model_probe")
    hooks = read_jsonl(hook_log)
    tenv = next((h.get("telemetry_env") for h in hooks if h.get("telemetry_env") is not None), None)
    r.data["telemetry_env_seen_by_hooks"] = tenv
    by_src = {}
    for h in hooks:
        by_src.setdefault(h["source"], set()).add(h["event"])
    r.data["hooks_fired"] = {k: sorted(v) for k, v in by_src.items()}
    for src in ("plugin", "settings"):
        got = by_src.get(src, set())
        r.check(f"{src} hooks fire (PreToolUse, Stop)", {"PreToolUse", "Stop"} <= got, f"got {sorted(got)}")
    if hooks:
        r.data["hook_input_keys"] = sorted({k for h in hooks for k in h.get("keys", [])})
        tp = next((h["transcript_path"] for h in hooks if h.get("transcript_path")), None)
        r.data["transcript_path"] = tp
        r.check("transcript file exists", bool(tp) and os.path.exists(tp), str(tp), critical=False)
        r.check("hook session_id matches", all(h.get("session_id") in (sid, None) for h in hooks), "", critical=False)
    evs = recv.events()
    names = sorted({n for n, _ in evs if n})
    r.data["otel_events"] = names
    r.data["otel_metrics"] = sorted(n for n in recv.metric_names() if n)
    r.data["otel_traces_posts"] = recv.count("/v1/traces")
    if recv.non_json():
        r.data["otel_non_json"] = recv.non_json()[:3]
    r.check("OTel events arrive (api_request)", any(n.endswith("api_request") for n in names), f"events={names}")
    tr_attrs = [a for n, a in evs if n and n.endswith("tool_result")]
    if tr_attrs:
        r.data["otel_tool_result_attrs"] = sorted(tr_attrs[0].keys())
    r.check("tool_result has size attributes", any("tool_result_size_bytes" in a for a in tr_attrs), "", critical=False)
    r.check("OTel metrics arrive", bool(r.data["otel_metrics"]), "", critical=False)
    r.check("OTel traces arrive (beta)", r.data["otel_traces_posts"] > 0, "", critical=False)
    return r.finish()


def t_json_schema(ctx, r):
    repo = ctx.repo("json-schema")
    schema = {"type": "object", "properties": {"ok": {"type": "boolean"}, "word": {"type": "string"}},
              "required": ["ok", "word"]}
    argv = [ctx.claude, "-p", "Return ok=true and word=pong.", "--output-format", "json", "--model", ctx.model,
            "--json-schema", json.dumps(schema), "--tools", ""]
    rc, out, err, secs = run(argv, env=ctx.env(), cwd=repo, timeout=ctx.args.timeout)
    (ctx.out / "json-schema.out.json").write_text(out)
    try:
        res = json.loads(out)
    except ValueError:
        r.check("JSON result", False, tail(out + err, 500))
        return r.finish()
    r.data["result_keys"] = sorted(res.keys())
    so = res.get("structured_output")
    r.data["structured_output"] = so
    r.check("structured_output matches schema", isinstance(so, dict) and so.get("ok") is True and "word" in so,
            f"structured_output={so!r}")
    return r.finish()


def t_cc_sandbox(ctx, r):
    if not shutil.which("bwrap") or not shutil.which("socat"):
        return r.skip("bubblewrap/socat missing (see bwrap test)")
    repo = ctx.repo("cc-sandbox")
    outside = Path.home() / ".cy-spike-outside"
    if outside.exists():
        outside.unlink()
    settings = {"sandbox": {"enabled": True, "autoAllowBashIfSandboxed": True}}
    argv = [ctx.claude, "-p", "--output-format", "stream-json", "--verbose", "--model", ctx.model,
            "--settings", json.dumps(settings), "--permission-prompts", "none", "--tools", "Bash"]
    prompt = (f"Use the Bash tool to run these two commands as two separate tool calls: "
              f"`touch inside.txt` and `touch {outside}`. Then reply with one word: done.")
    rc, out, err, secs = run(argv, env=ctx.env(), cwd=repo, timeout=ctx.args.timeout, stdin_data=prompt)
    (ctx.out / "cc-sandbox-stream.jsonl").write_text(out)
    msgs = []
    for line in out.splitlines():
        try:
            msgs.append(json.loads(line))
        except ValueError:
            pass
    tr = tool_results(msgs)
    r.data["tool_results"] = tr
    r.check("sandboxed Bash auto-allowed inside cwd", (repo / "inside.txt").exists(),
            "inside.txt missing: sandbox not active or Bash denied; " + tail(err, 300))
    leaked = outside.exists()
    if leaked:
        outside.unlink()
    r.check("write outside cwd blocked", not leaked, "the file outside the repo was created!")
    return r.finish()


def t_osc7501(ctx, r):
    repo = ctx.repo("osc7501")
    argv = [ctx.claude, "--model", ctx.model, "--permission-mode", ctx.manual_mode]
    p = Pty(argv, ctx.env(pty=True), repo, ctx.out / "osc7501-pty.log")
    try:
        ready, pressed = handle_startup_dialogs(
            p, 60, lambda: p.idle_for(2))
        r.data["startup_screens_answered"] = pressed
        r.check("claude stayed up during startup", p.exit_status is None,
                "exited early, see the screen below and osc7501-screen.txt")
        r.check("CC sent the OSC 7501 probe", p.probe_seen, "no `OSC 7501 ; ?` seen: version too old?")
        r.check("idle reported at the prompt", bool(ready), tail(p.screen(3000), 600))
        if ready:
            t1 = time.time()
            p.paste(PONG_PROMPT)
            w = p.wait_state_t(lambda s: s.get("state") == "working", 30, since=t1)
            d = p.wait_state_t(lambda s: s.get("state") in ("done", "idle"), ctx.args.timeout, since=w[0]) if w else None
            r.check("working reported", bool(w), "")
            r.check("done/idle after the turn", bool(d), "")
            if w and d:
                r.data["turn_seconds"] = round(d[0] - w[0], 1)
            r.check("reply visible on screen", "pong" in p.screen(30000).lower(), "", critical=False)
            t2 = time.time()
            p.paste("Use the Bash tool to run exactly: touch cy-spike-perm.txt")
            b = p.wait_state(lambda s: s.get("state") == "blocked", ctx.args.timeout, since=t2)
            r.data["blocked_report"] = b
            r.check("blocked kind=permission reported", bool(b) and b.get("kind") == "permission",
                    f"got {b}", critical=False)
            if b:
                time.sleep(0.5)
                p._write(b"\x1b")  # Esc = reject
                p.wait_state(lambda s: s.get("state") in ("idle", "done"), 30, since=time.time())
            r.check("rejected command did not run", not (repo / "cy-spike-perm.txt").exists(),
                    "file exists: permission mode let it through", critical=False)
    finally:
        t3 = time.time()
        p.close()
    r.check("clear on exit", any(s.get("state") == "clear" for s in p.states(t3 - 0.01)), "", critical=False)
    r.data["states_seen"] = [str(s.get("state")) + (":" + s["kind"] if s.get("kind") else "") for s in p.states()][:60]
    r.data["app"] = next((s.get("app") for s in p.states() if s.get("app")), None)
    (ctx.out / "osc7501-screen.txt").write_text(p.screen(200000))
    return r.finish()


def _agents(ctx, env, cwd):
    rc, out, err, _ = run([ctx.claude, "agents", "--json", "--all", "--cwd", str(cwd)], env=env, timeout=30)
    try:
        return json.loads(out)
    except ValueError:
        return None


def t_background(ctx, r):
    repo = ctx.repo("background")
    hook_log = ctx.out / "background-hooks.jsonl"
    plugin, _ = make_plugin(ctx, "plugin-background", hook_log)
    recv = OtelReceiver(ctx.out / "background-otel.jsonl")
    env = ctx.env(recv.env(traces=False))
    name = f"cy-spike-bg-{uuid.uuid4().hex[:6]}"
    argv = [ctx.claude, "--bg", PONG_PROMPT, "--model", ctx.model, "--permission-mode", ctx.manual_mode,
            "--plugin-dir", str(plugin), "-n", name]
    trusted, pressed = trust_folder(ctx, repo)
    r.data["trust_step"] = {"ok": trusted, "screens": pressed}
    r.check("workspace trusted via the dialog", trusted and "trust" in pressed,
            "could not accept the trust dialog (see trust-pty.log)", critical=False)
    rc, out, err, secs = run(argv, env=env, cwd=repo, timeout=90)
    r.data["bg_stdout"] = tail(out, 600)
    r.check("claude --bg returns quickly", rc == 0 and secs < 60, f"rc={rc} {secs:.1f}s {tail(err, 300)}")
    entry = wait_until(lambda: next((e for e in (_agents(ctx, env, repo) or []) if name in json.dumps(e)), None), 30)
    r.data["agents_entry"] = entry
    r.check("listed by claude agents --json", entry is not None, "")
    sid = None
    if entry:
        tokens = set(re.findall(r"[A-Za-z0-9_\-]{4,}", out))
        cands = [entry.get(k) for k in ("id", "shortId", "short_id")] + list(entry.values())
        sid = next((str(v) for v in cands if isinstance(v, (str, int)) and str(v) in tokens), None)
        sid = sid or entry.get("id") or entry.get("shortId") or name
    r.data["id_used"] = sid
    statuses = []

    def got_pong():
        e = next((e for e in (_agents(ctx, env, repo) or []) if name in json.dumps(e)), None)
        if e:
            statuses.append(e.get("status"))
        _, lo, _, _ = run([ctx.claude, "logs", sid or name], env=env, timeout=30)
        return "pong" in lo.lower()

    ok = wait_until(got_pong, ctx.args.timeout, step=3) if sid else False
    r.data["statuses_seen"] = sorted({s for s in statuses if s})
    r.check("session answered (claude logs)", bool(ok), "")
    hooks = read_jsonl(hook_log)
    r.data["hooks_fired"] = sorted({h["event"] for h in hooks})
    r.check("--plugin-dir hooks fire in bg session", bool(hooks), "", critical=False)
    wait_until(lambda: recv.count("/v1/logs") > 0, 10)
    r.data["otel_posts"] = len(recv._snapshot())
    r.check("OTel env reaches bg session", r.data["otel_posts"] > 0,
            "env given to `claude --bg` did not reach the session; set telemetry via --settings env instead", critical=False)
    recv.stop()
    if sid:
        rc1, o1, e1, _ = run([ctx.claude, "stop", sid], env=env, timeout=30)
        r.check("claude stop", rc1 == 0, tail(o1 + e1, 300), critical=False)
        rc2, o2, e2, _ = run([ctx.claude, "rm", sid], env=env, timeout=30)
        r.check("claude rm", rc2 == 0, tail(o2 + e2, 300), critical=False)
        gone = wait_until(lambda: not any(name in json.dumps(e) for e in (_agents(ctx, env, repo) or [])), 15)
        r.check("gone after rm", bool(gone), "", critical=False)
    return r.finish()


def _landlock_rw(ctx, repo, config_dir=None):
    home = Path.home()
    rw = [repo, "/tmp", "/dev", home / ".cache", home / ".local" / "state", home / ".local" / "share" / "claude"]
    if os.environ.get("TMPDIR"):
        rw.append(os.environ["TMPDIR"])
    if config_dir:
        rw.append(config_dir)
    else:
        rw += [home / ".claude", home / ".claude.json", home / ".claude.json.backup"]
    return [str(p) for p in rw]


def t_landlock_claude(ctx, r):
    abi = landlock_abi()
    if not abi or abi < 1:
        return r.skip("Landlock unavailable")
    repo = ctx.repo("landlock-claude")
    cmd = [ctx.claude, "-p", PONG_PROMPT, "--model", ctx.model, "--output-format", "json", "--tools", ""]
    rw = _landlock_rw(ctx, repo, ctx.config_dir)
    r.data["rw"] = rw
    rc, out, err, _ = run(landlock_wrap(rw, cmd), env=ctx.env(), cwd=repo, timeout=ctx.args.timeout)
    ok = rc == 0 and "pong" in out.lower()
    r.data["attempt1"] = {"rc": rc, "stderr": tail(err, 800), "stdout": tail(out, 400)}
    if ok:
        r.check("claude -p works under Landlock (narrow rw list)", True)
        return r.finish("works with: repo, config dir, /tmp, /dev, caches")
    r.check("claude -p works under Landlock (narrow rw list)", False, tail(err, 400), critical=False)
    rc2, out2, err2, _ = run(landlock_wrap(rw + [str(Path.home())], cmd), env=ctx.env(), cwd=repo, timeout=ctx.args.timeout)
    ok2 = rc2 == 0 and "pong" in out2.lower()
    r.data["attempt2_home_rw"] = {"rc": rc2, "stderr": tail(err2, 800), "stdout": tail(out2, 400)}
    r.check("works with $HOME writable", ok2, tail(err2, 400))
    return r.finish("needs more than the narrow list (see attempt1 stderr)" if ok2 else None)


def t_remote_control(ctx, r):
    repo = ctx.repo("remote-control")
    argv = [ctx.claude, "--remote-control", "cy-spike", "--model", ctx.model, "--permission-mode", ctx.manual_mode]
    p = Pty(argv, ctx.env(pty=True), repo, ctx.out / "remote-control-pty.log")
    url_re = re.compile(rb"https://claude\.ai/code[^\s\x07\x1b\"'<>]*")
    try:
        _, pressed = handle_startup_dialogs(p, 60, lambda: url_re.search(p.raw()) or url_re.search(p.screen(50000).encode()))
        r.data["startup_screens_answered"] = pressed
        m = url_re.search(p.raw()) or url_re.search(p.screen(50000).encode())
    finally:
        p.close()
    r.data["url"] = m.group(0).decode() if m else None
    r.check("Remote Control URL printed", m is not None, tail(p.screen(4000), 500))
    return r.finish()


def accounts_dirs(ctx):
    return [os.path.abspath(os.path.expanduser(d)) for d in (ctx.args.accounts or "").split(",") if d.strip()]


def t_accounts(ctx, r):
    dirs = accounts_dirs(ctx)
    if len(dirs) < 2:
        return r.skip("pass two config dirs: --accounts DIR_A,DIR_B")
    infos = []
    for d in dirs:
        rc, out, err, _ = run([ctx.claude, "auth", "status", "--json"], env=ctx.env(config_dir=d), timeout=30)
        try:
            st = json.loads(out)
        except ValueError:
            st = {"error": tail(out + err, 200)}
        infos.append(st)
        r.check(f"logged in: {d}", bool(st.get("loggedIn")), f"CLAUDE_CONFIG_DIR={d} claude auth login")
    r.data["accounts"] = infos
    ident = [json.dumps({k: v for k, v in i.items() if k not in ("configDirectory", "projectsDirectory")}, sort_keys=True)
             for i in infos]
    r.check("two different identities", ident[0] != ident[1], "both dirs look like the same account", critical=False)

    def count_transcripts(d):
        return sum(1 for _ in Path(d, "projects").rglob("*.jsonl")) if Path(d, "projects").exists() else 0

    before = [count_transcripts(d) for d in dirs]
    procs = []
    for i, d in enumerate(dirs[:2]):
        repo = ctx.repo(f"account-{i}", config_dir=d)
        argv = [ctx.claude, "-p", PONG_PROMPT, "--model", ctx.model, "--output-format", "json", "--tools", ""]
        procs.append((time.time(), subprocess.Popen(argv, env=ctx.env(config_dir=d), cwd=repo,
                                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)))
    runs = []
    for t0, p in procs:
        try:
            out, err = p.communicate(timeout=ctx.args.timeout)
        except subprocess.TimeoutExpired:
            p.kill()
            out, err = p.communicate()
        t1 = time.time()
        try:
            res = json.loads(out)
        except ValueError:
            res = {"is_error": True, "result": tail((out + err).decode(errors="replace"), 300)}
        runs.append({"start": t0, "end": t1, "is_error": res.get("is_error"), "result": str(res.get("result"))[:80],
                     "cost": res.get("total_cost_usd")})
    r.data["runs"] = runs
    for i, x in enumerate(runs):
        r.check(f"account {i} answered", not x["is_error"] and "pong" in x["result"].lower(), x["result"])
    overlap = max(x["start"] for x in runs) < min(x["end"] for x in runs)
    r.check("ran in parallel", overlap, "")
    after = [count_transcripts(d) for d in dirs]
    r.data["transcripts_before_after"] = list(zip(before, after))
    r.check("each transcript landed in its own config dir", all(a > b for b, a in zip(before, after)), "", critical=False)
    return r.finish()


TESTS = [
    # id, function, needs_model, opt_in, what it checks
    ("preflight", t_preflight, False, False, "Claude Code version, flags and subcommands Cloneyard uses; agents --json"),
    ("auth", t_auth, False, False, "logged in, subscription rather than API key"),
    ("model_probe", t_model_probe, True, False, "one tiny call; gates the other model tests (retries an OAuth refresh clash once)"),
    ("landlock", t_landlock, False, False, "Landlock ABI and real enforcement in a child process"),
    ("seccomp", t_seccomp, False, False, "seccomp available"),
    ("bwrap", t_bwrap, False, False, "bubblewrap + socat for the native CC sandbox, user namespaces"),
    ("headless", t_headless, True, False,
     "stream-json multi-turn on one process, --session-id, --append-system-prompt-file, plugin + --settings hooks, "
     "--permission-prompts none, rate_limit_event, local OTLP/HTTP JSON receiver"),
    ("json_schema", t_json_schema, True, False, "--json-schema structured output"),
    ("cc_sandbox", t_cc_sandbox, True, False, "native CC sandbox: Bash auto-allowed inside cwd, blocked outside"),
    ("osc7501", t_osc7501, True, False, "interactive session in a PTY: OSC 7501 probe, idle/working/done/blocked, clear"),
    ("background", t_background, True, False, "claude --bg, agents --json, logs, stop, rm; hooks and OTel env in bg"),
    ("landlock_claude", t_landlock_claude, True, False, "claude -p under Landlock with a narrow writable list"),
    ("remote_control", t_remote_control, True, True, "--remote-control prints a claude.ai/code URL (opt-in: --with-rc)"),
    ("accounts", t_accounts, True, True, "two CLAUDE_CONFIG_DIR accounts in parallel (opt-in: --accounts A,B)"),
]


# --------------------------------------------------------------------------- main


def _otel_account_ids(obj, found):
    if isinstance(obj, dict):
        key = obj.get("key")
        label = OTEL_ACCOUNT_IDS.get(key) if isinstance(key, str) else None
        if label and isinstance(obj.get("value"), dict):
            value = _attr_value(obj["value"])
            if isinstance(value, str) and value:
                found[value] = label
        for v in obj.values():
            _otel_account_ids(v, found)
    elif isinstance(obj, list):
        for v in obj:
            _otel_account_ids(v, found)


def collect_account_ids(ctx):
    dirs = [ctx.config_dir] + accounts_dirs(ctx) if ctx.claude else []
    for d in dirs:
        _, out, _, _ = run([ctx.claude, "auth", "status", "--json"], env=ctx.env(config_dir=d), timeout=30)
        try:
            st = json.loads(out)
        except ValueError:
            continue
        if not isinstance(st, dict):
            continue
        for key, label in AUTH_ACCOUNT_IDS.items():
            if st.get(key):
                ctx.account_ids[str(st[key])] = label
    for f in ctx.out.glob("*-otel.jsonl"):
        for post in read_jsonl(f):
            _otel_account_ids(post.get("data"), ctx.account_ids)


def mask_tree(ctx):
    for p in ctx.out.rglob("*"):
        if p.is_file() and p.suffix in (".json", ".jsonl", ".txt", ".log", ".md") and p.relative_to(ctx.out).parts[0] != "work":
            try:
                text = p.read_bytes().decode(errors="replace")
                masked = ctx.mask(text)
                if masked != text:
                    p.write_text(masked)
            except OSError:
                pass


def write_report(ctx, results):
    data = {
        "spike_version": SPIKE_VERSION,
        "when": time.strftime("%Y-%m-%d %H:%M:%S %z"),
        "model": ctx.model,
        "config_dir": ctx.config_dir,
        "results": [x.as_dict() for x in results],
    }
    (ctx.out / "report.json").write_text(ctx.mask(json.dumps(data, indent=2, default=str)))
    lines = [f"# Cloneyard spike {SPIKE_VERSION}", "", f"- When: {data['when']}", f"- Model: {ctx.model}",
             f"- Config dir: {ctx.config_dir or 'default'}", "", "| Test | Status | Summary |", "| --- | --- | --- |"]
    for x in results:
        lines.append(f"| {x.id} | {x.status} | {x.summary.replace('|', '/')} |")
    for x in results:
        lines += ["", f"## {x.id}: {x.status}", "", x.summary, ""]
        for c in x.checks:
            mark = {True: "ok", False: "FAIL" if c["critical"] else "warn", None: "?"}[c["ok"]]
            note = f" ({c['note']})" if c["note"] and c["ok"] is not True else ""
            lines.append(f"- [{mark}] {c['name']}{note}")
        if x.data:
            lines += ["", "```json", json.dumps(x.data, indent=2, default=str)[:6000], "```"]
    (ctx.out / "report.md").write_text(ctx.mask("\n".join(lines) + "\n"))


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--_landlock-exec":
        landlock_exec_main(sys.argv[2:])
        return 0
    ap = argparse.ArgumentParser(description="Checks the Claude Code assumptions behind Cloneyard.")
    ap.add_argument("--list", action="store_true", help="list tests and exit")
    ap.add_argument("--only", help="comma-separated test ids")
    ap.add_argument("--skip", help="comma-separated test ids to skip")
    ap.add_argument("--offline", action="store_true", help="no model calls")
    ap.add_argument("--with-rc", action="store_true", help="include the Remote Control test")
    ap.add_argument("--accounts", help="two CLAUDE_CONFIG_DIRs, comma-separated, to test in parallel")
    ap.add_argument("--config-dir", help="CLAUDE_CONFIG_DIR for every test")
    ap.add_argument("--model", default="haiku", help="model alias or name (default: haiku)")
    ap.add_argument("--claude", help="path to the claude binary")
    ap.add_argument("--out", help="report folder (default: ./cloneyard-spike-<timestamp>)")
    ap.add_argument("--timeout", type=int, default=180, help="per model turn, seconds (default 180)")
    ap.add_argument("--keep-api-key", action="store_true", help="do not strip ANTHROPIC_API_KEY")
    ap.add_argument("--keep-cc-data", action="store_true",
                    help="keep Claude Code transcripts of the scratch repos (default: claude purge them)")
    args = ap.parse_args()

    if args.list:
        for tid, _, needs_model, opt_in, what in TESTS:
            tag = ("model" if needs_model else "local") + (", opt-in" if opt_in else "")
            print(f"{tid:16} [{tag}] {what}")
        return 0
    if not sys.platform.startswith("linux"):
        print("Linux only.")
        return 2

    only = set(args.only.split(",")) if args.only else None
    skip = set(args.skip.split(",")) if args.skip else set()
    ctx = Ctx(args)
    selected = []
    for tid, fn, needs_model, opt_in, _ in TESTS:
        if only is not None and tid not in only:
            continue
        if tid in skip or (args.offline and needs_model):
            continue
        if opt_in and only is None:
            if tid == "remote_control" and not args.with_rc:
                continue
            if tid == "accounts" and not args.accounts:
                continue
        selected.append((tid, fn, needs_model))
    if only is not None and "preflight" not in only:
        selected.insert(0, ("preflight", t_preflight, False))  # sets version and help text
    n_model = sum(1 for _, _, m in selected if m)
    print(f"Cloneyard spike {SPIKE_VERSION} -> {ctx.out}")
    if n_model:
        print(f"{n_model} test(s) make small calls with --model {args.model} on your plan.")
    results = []
    preflight_ok = True
    for tid, fn, needs_model in selected:
        r = Result(tid)
        tty = sys.stdout.isatty()
        if tty:
            print(f"  {tid:16} ...", end="", flush=True)
        try:
            if needs_model and not preflight_ok:
                r.skip("claude not usable (see preflight)")
            elif needs_model and ctx.logged_in is False:
                r.skip("not logged in (see auth)")
            elif needs_model and tid != "model_probe" and ctx.model_ok is False:
                r.skip("model call failing (see model_probe): " + ctx.model_err[:60])
            else:
                fn(ctx, r)
        except Exception as e:  # keep going, record the crash
            r.check("test crashed", False, f"{type(e).__name__}: {e}")
            r.finish()
        if tid == "preflight" and not ctx.claude:
            preflight_ok = False
        results.append(r)
        print(f"{chr(13) if tty else ''}  {tid:16} {r.status:5} {r.seconds:6.1f}s  {ctx.mask(r.summary)[:90]}")
    collect_account_ids(ctx)
    write_report(ctx, results)
    mask_tree(ctx)
    if ctx.claude and not args.keep_cc_data:
        purged = 0
        for repo, cfg in ctx.repos:
            if repo.exists():
                rc, _, _, _ = run([ctx.claude, "purge", "-y", str(repo)], env=ctx.env(config_dir=cfg), timeout=60)
                purged += rc == 0
        if purged:
            print(f"Removed Claude Code data (transcripts, history) of {purged} scratch repo(s); --keep-cc-data keeps it.")
    print(f"\nReport: {ctx.out / 'report.md'}")
    return 1 if any(x.status == "FAIL" for x in results) else 0


if __name__ == "__main__":
    sys.exit(main())
