"""Starts, drives, screenshots and measures private Schrumbi instances for agents. See `app help`."""

import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

WIN = os.name == "nt"
MAC = sys.platform == "darwin"
TOOLS = Path(__file__).resolve().parent
ROOT = Path(subprocess.run(["git", "-C", str(TOOLS), "rev-parse", "--show-toplevel"], capture_output=True, text=True).stdout.strip())
COMMON = (ROOT / subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--git-common-dir"], capture_output=True, text=True).stdout.strip()).resolve()
EXE = "schrumbi.exe" if WIN else "schrumbi"
GENERIC_NAMES = "abcdefghjkmnpqrstuvwxyz"


def base_dir():
    if os.environ.get("APP_HOME"):
        return Path(os.environ["APP_HOME"])
    if WIN:
        return Path(os.environ.get("LOCALAPPDATA", tempfile.gettempdir())) / "Temp" / "schrumbi-agent"
    return Path(f"/tmp/schrumbi-agent-{os.getuid()}")


BASE = base_dir()
TREE = BASE / hashlib.sha1(str(ROOT).lower().encode() if WIN else str(ROOT).encode()).hexdigest()[:10]
SAFE_ROOTS = [Path(tempfile.gettempdir()), BASE] + ([] if WIN else [Path("/tmp"), Path("/private/tmp")])


def scratch_only(path, what):
    real = Path(os.path.realpath(path))
    if not any(real.is_relative_to(Path(os.path.realpath(r))) for r in SAFE_ROOTS):
        die(f"{what} {real} is outside the temp folder; a driven instance never touches real files. Copy it into your instance's tmp folder first")
    return real


def die(msg, code=1):
    print(msg, file=sys.stderr)
    sys.exit(code)


if WIN:
    import ctypes
    from ctypes import wintypes

    k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    k32.OpenProcess.restype = wintypes.HANDLE
    k32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    k32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    user32 = ctypes.WinDLL("user32")

    class PROCESSENTRY32W(ctypes.Structure):
        _fields_ = [("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD), ("th32ProcessID", wintypes.DWORD), ("th32DefaultHeapID", ctypes.c_size_t),
                    ("th32ModuleID", wintypes.DWORD), ("cntThreads", wintypes.DWORD), ("th32ParentProcessID", wintypes.DWORD), ("pcPriClassBase", ctypes.c_long),
                    ("dwFlags", wintypes.DWORD), ("szExeFile", ctypes.c_wchar * 260)]

    class PMC(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [(n, ctypes.c_size_t) for n in ("Peak", "WorkingSet", "QPeakPaged", "QPaged", "QPeakNon", "QNon", "Pagefile", "PeakPagefile")]

    def processes():
        snap = k32.CreateToolhelp32Snapshot(2, 0)
        entry = PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(entry)
        out = {}
        ok = k32.Process32FirstW(snap, ctypes.byref(entry))
        while ok:
            out[entry.th32ProcessID] = (entry.th32ParentProcessID, entry.szExeFile)
            ok = k32.Process32NextW(snap, ctypes.byref(entry))
        k32.CloseHandle(snap)
        return out

    def image(pid):
        h = k32.OpenProcess(0x1000, False, pid)
        if not h:
            return None
        try:
            code = wintypes.DWORD()
            if not k32.GetExitCodeProcess(h, ctypes.byref(code)) or code.value != 259:
                return None
            buf = ctypes.create_unicode_buffer(1024)
            size = wintypes.DWORD(1024)
            return buf.value if k32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)) else ""
        finally:
            k32.CloseHandle(h)

    def kill(pid):
        h = k32.OpenProcess(0x0001, False, pid)
        if h:
            k32.TerminateProcess(h, 1)
            k32.CloseHandle(h)

    def usage(pid):
        h = k32.OpenProcess(0x1000 | 0x0010, False, pid)
        if not h:
            return None
        try:
            t = [wintypes.FILETIME() for _ in range(4)]
            k32.GetProcessTimes(h, *[ctypes.byref(x) for x in t])
            cpu = sum((x.dwHighDateTime << 32 | x.dwLowDateTime) for x in t[2:]) / 1e4
            pmc = PMC()
            pmc.cb = ctypes.sizeof(pmc)
            k32.K32GetProcessMemoryInfo(h, ctypes.byref(pmc), pmc.cb)
            return cpu, pmc.WorkingSet / 1048576
        finally:
            k32.CloseHandle(h)

    def foreground():
        h = user32.GetForegroundWindow()
        pid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(h, ctypes.byref(pid))
        return h or 0, pid.value
else:
    import signal

    def processes():
        out = {}
        for line in subprocess.run(["ps", "-A", "-o", "pid=,ppid=,comm="], capture_output=True, text=True).stdout.splitlines():
            parts = line.split(None, 2)
            if len(parts) == 3:
                out[int(parts[0])] = (int(parts[1]), os.path.basename(parts[2]))
        return out

    def image(pid):
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return None
        except PermissionError:
            return ""
        if Path(f"/proc/{pid}/exe").exists():
            try:
                return os.readlink(f"/proc/{pid}/exe")
            except OSError:
                return ""
        line = subprocess.run(["ps", "-o", "stat=,comm=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
        if not line or line.startswith("Z"):
            return None
        return line.split(None, 1)[1] if " " in line else ""

    def kill(pid):
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass

    def usage(pid):
        out = subprocess.run(["ps", "-o", "time=,rss=", "-p", str(pid)], capture_output=True, text=True).stdout.split()
        if len(out) != 2:
            return None
        clock, rss = out
        days, _, clock = clock.rpartition("-")
        secs = 0.0
        for part in clock.split(":"):
            secs = secs * 60 + float(part)
        return (secs + (int(days) * 86400 if days else 0)) * 1000, int(rss) / 1024

    def foreground():
        return None


def owner_pid():
    if os.environ.get("APP_OWNER"):
        return int(os.environ["APP_OWNER"])
    table = processes()
    pid, seen = int(os.environ.get("APP_FROM_PID") or os.getppid()), set()
    while pid and pid not in seen and pid in table:
        seen.add(pid)
        parent, name = table[pid]
        if name.lower().removesuffix(".exe") in ("claude", "node", "bun"):
            return pid
        pid = parent
    return 0


class Instance:
    def __init__(self, name):
        self.name = name
        self.dir = TREE / name
        self.meta = json.loads((self.dir / "meta.json").read_text(encoding="utf-8")) if (self.dir / "meta.json").exists() else {}

    @property
    def pid(self):
        return self.meta.get("pid", 0)

    @property
    def exe(self):
        return self.dir / "bin" / EXE

    def alive(self):
        if not self.pid:
            return False
        found = image(self.pid)
        if found is None:
            return False
        return bool(found) and os.path.normcase(os.path.realpath(found)) == os.path.normcase(os.path.realpath(self.exe))

    def log_tail(self, n=30):
        log = self.dir / "log"
        lines = log.read_text(encoding="utf-8", errors="replace").splitlines()[-n:] if log.exists() else []
        crash = self.dir / "home" / "data" / "crash.log"
        if crash.exists():
            lines.append(f"crash log: {crash}")
        return "\n".join(lines)

    def call(self, cmd, timeout=10, **args):
        info_path = self.dir / "drive.json"
        if not self.alive():
            die(f"instance {self.name} is not running (exited or never started).\n{self.log_tail()}")
        try:
            info = json.loads(info_path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            die(f"instance {self.name} has no drive channel yet")
        body = {"token": info["token"], "cmd": cmd, "timeout": timeout, **{k: v for k, v in args.items() if v is not None}}
        try:
            with socket.create_connection(("127.0.0.1", info["port"]), timeout=5) as s:
                s.settimeout(timeout + 15)
                s.sendall((json.dumps(body) + "\n").encode())
                data = b""
                while not data.endswith(b"\n"):
                    chunk = s.recv(1 << 20)
                    if not chunk:
                        break
                    data += chunk
        except OSError as e:
            if not self.alive():
                die(f"instance {self.name} died.\n{self.log_tail()}")
            die(f"no answer from instance {self.name}: {e}")
        if not data:
            if not self.alive():
                die(f"instance {self.name} died.\n{self.log_tail()}")
            die(f"instance {self.name} closed the connection")
        out = json.loads(data)
        if out.get("menu"):
            print(f"warning: {out['menu']}", file=sys.stderr)
        if not out.get("ok"):
            if out.get("hung"):
                die(f"{out.get('error')}. The UI thread hangs; `app sample` shows where.", 3)
            die(out.get("error", "failed"))
        return out


def instances():
    if not TREE.exists():
        return []
    return [Instance(d.name) for d in sorted(TREE.iterdir()) if d.is_dir() and (d / "meta.json").exists()]


def gc(quiet=True):
    removed = []
    if TREE.exists():
        for d in TREE.iterdir():
            inst = Instance(d.name) if d.is_dir() else None
            try:
                if not inst or (inst.meta and inst.alive()) or time.time() - d.stat().st_mtime < 600:
                    continue
            except OSError:
                continue
            shutil.rmtree(d, ignore_errors=True)
            removed.append(d.name)
    for tree in BASE.iterdir() if BASE.exists() else []:
        if tree != TREE and tree.is_dir():
            try:
                removed += [f"{tree.name}/{d.name}" for d in tree.iterdir() if stale_elsewhere(d)]
            except OSError:
                pass
    if not quiet:
        print(f"removed {', '.join(removed)}" if removed else "nothing to clean")


def stale_elsewhere(d):
    try:
        info = json.loads((d / "meta.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        info = {}
    found = image(info["pid"]) if info.get("pid") else None
    if found and os.path.normcase(os.path.realpath(found)) == os.path.normcase(os.path.realpath(d / "bin" / EXE)):
        return False
    try:
        if time.time() - d.stat().st_mtime <= 86400:
            return False
        shutil.rmtree(d, ignore_errors=True) if d.is_dir() else d.unlink(missing_ok=True)
    except OSError:
        return False
    return True


def pick_instance(name):
    name = name or os.environ.get("APP_INSTANCE")
    if name:
        inst = Instance(name)
        if not inst.meta:
            die(f"no instance {name}; `app ps` lists them")
        return inst
    live = [i for i in instances() if i.alive()]
    owner = owner_pid()
    if owner:
        live = [i for i in live if i.meta.get("owner") == owner]
    if not live:
        dead = [i for i in instances() if owner and i.meta.get("owner") == owner and not i.meta.get("stopped")]
        if dead:
            gone = max(dead, key=lambda i: i.meta.get("started", 0))
            die(f"instance {gone.name} exited without app stop:\n{gone.log_tail()}")
        die("none of your instances is running; use: app start (app ps lists all)")
    if len(live) == 1:
        return live[0]
    die("several of your instances run; pick one with -i NAME or APP_INSTANCE: " + ", ".join(i.name for i in live))


def claim(name):
    TREE.mkdir(parents=True, exist_ok=True)
    if not WIN:
        os.chmod(BASE, 0o700)
    try:
        (TREE / name).mkdir()
        return True
    except FileExistsError:
        return False


def stop(inst, quiet=False):
    if inst.alive():
        try:
            inst.call("quit", timeout=5)
        except SystemExit:
            pass
        deadline = time.time() + 5
        while time.time() < deadline and inst.alive():
            time.sleep(0.1)
        if inst.alive():
            kill(inst.pid)
            time.sleep(0.3)
    if not inst.alive():
        shutil.rmtree(inst.dir / "bin", ignore_errors=True)
    if inst.meta:
        inst.meta["stopped"] = True
        (inst.dir / "meta.json").write_text(json.dumps(inst.meta), encoding="utf-8")
    (inst.dir / "drive.json").unlink(missing_ok=True)
    if not quiet:
        print(f"stopped {inst.name}")


def start(opts):
    if opts["data"]:
        opts["data"] = str(scratch_only(opts["data"], "--data"))
    gc()
    owner = owner_pid()
    profile = "dev" if opts["debug"] else "perf"
    target = ROOT / "target" / ("debug" if profile == "dev" else profile)
    if opts["build"]:
        print(f"building {profile} …", file=sys.stderr)
        if subprocess.run(["cargo", "build", "--profile", profile], cwd=ROOT, stdout=sys.stderr).returncode:
            die("build failed")
    if not (target / EXE).exists():
        die(f"no build at {target / EXE}; run without --no-build")
    name = opts["name"]
    if name:
        inst = Instance(name)
        if not claim(name):
            if inst.meta and inst.alive():
                if inst.meta.get("owner") != owner:
                    die(f"instance {name} belongs to another agent (pid {inst.meta.get('owner')}); pick another name")
                stop(inst, quiet=True)
            elif not inst.meta and time.time() - inst.dir.stat().st_mtime < 60:
                die(f"instance {name} is being started by someone else; pick another name")
    else:
        name = next((n for n in GENERIC_NAMES if claim(n)), None) or next(f"i{n}" for n in range(1000) if claim(f"i{n}"))
    inst = Instance(name)
    d = inst.dir
    os.utime(d)
    for sub in ("home", "data", "tmp", "bin", "shots"):
        shutil.rmtree(d / sub, ignore_errors=True)
    for sub in ("home", "tmp", "bin", "shots"):
        (d / sub).mkdir(parents=True)
    shutil.copy2(target / EXE, inst.exe)
    if WIN and (target / "schrumbi.pdb").exists():
        shutil.copy2(target / "schrumbi.pdb", d / "bin" / "schrumbi.pdb")
    data = scratch_only(opts["data"], "--data") if opts["data"] else d / "data"
    if not opts["data"]:
        if subprocess.run([sys.executable, "-I", str(TOOLS / "fixture.py"), str(data), str(opts["notes"])]).returncode:
            die("fixture failed")
    w, h = opts["size"]
    settings = {"mainWindow": [100, 80, w, h], "updateCheck": False, "tray": False, "calendarFeed": False, "remindersEnabled": False}
    if opts["theme"]:
        settings["theme"] = opts["theme"]
    if opts["lang"]:
        settings["language"] = opts["lang"]
    if opts["data"] is None:
        inst_notes = opts["notes"]
    else:
        inst_notes = None
    (d / "home" / "settings.json").write_text(json.dumps(settings), encoding="utf-8")
    env = {k: v for k, v in os.environ.items() if k not in ("SCHRUMBI_PERF", "SCHRUMBI_SMOKE")}
    env.update(SCHRUMBI_HOME=str(d / "home"), SCHRUMBI_DRIVE=str(d), SCHRUMBI_DRIVE_OWNER=str(owner), SCHRUMBI_DRIVE_TTL=str(opts["ttl"]),
               RUST_BACKTRACE="1", TMP=str(d / "tmp"), TEMP=str(d / "tmp"), TMPDIR=str(d / "tmp"))
    env.pop("SCHRUMBI_DRIVE_AWAY", None)
    if WIN and not opts["visible"]:
        env["SCHRUMBI_DRIVE_AWAY"] = "1"
    (d / "drive.json").unlink(missing_ok=True)
    before = foreground()
    with open(d / "log", "wb") as log:
        if WIN:
            k32.SetErrorMode(0x8003)
            si = subprocess.STARTUPINFO()
            si.dwFlags = subprocess.STARTF_USESHOWWINDOW
            si.wShowWindow = 4
            proc = subprocess.Popen([str(inst.exe), str(data)], env=env, cwd=d, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                                    startupinfo=si, creationflags=0x08000000 | 0x00000200, close_fds=True)
        else:
            proc = subprocess.Popen([str(inst.exe), str(data)], env=env, cwd=d, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    inst.meta = {"pid": proc.pid, "owner": owner, "started": time.time(), "profile": profile, "data": str(data), "notes": inst_notes, "root": str(ROOT)}
    (d / "meta.json").write_text(json.dumps(inst.meta), encoding="utf-8")
    deadline = time.time() + opts["timeout"]
    while not channel_of(d, proc.pid):
        if proc.poll() is not None:
            die(f"app exited with {proc.returncode}:\n{inst.log_tail()}")
        if time.time() > deadline:
            stop(inst, quiet=True)
            die(f"no drive channel after {opts['timeout']} s, stopped it:\n{inst.log_tail()}")
        time.sleep(0.05)
    inst.call("wait", timeout=opts["timeout"], **{"for": "idle", "quiet": 300})
    wins = inst.call("windows")["windows"]
    after = foreground()
    print(f"instance {name}  pid {proc.pid}  profile {profile}  data {data}")
    for win in wins:
        print_window(win)
    if before and after and before != after:
        print(f"warning: the foreground window changed from {before} to {after}", file=sys.stderr)


def channel_of(d, pid):
    try:
        return json.loads((d / "drive.json").read_text(encoding="utf-8")).get("pid") == pid
    except (OSError, ValueError):
        return False


def print_window(w):
    print(f"window {w['id']}{' main' if w['main'] else ''}  {w['w']:.0f}x{w['h']:.0f} at {w['x']:.0f},{w['y']:.0f}  scale {w['scale']:g}  {'dark' if w['dark'] else 'light'}  \"{w['title']}\"")


def is_num(s):
    try:
        float(s)
        return True
    except ValueError:
        return False


def point_args(args, prefix=""):
    if len(args) >= 2 and is_num(args[0]) and is_num(args[1]):
        return {f"{prefix}x": float(args[0]), f"{prefix}y": float(args[1])}, args[2:]
    if not args:
        die("needs a target (id:…, text:…, role:…, or plain text) or x y")
    if "," in args[0] and all(is_num(p) for p in args[0].split(",", 1)):
        x, y = args[0].split(",", 1)
        return {f"{prefix}x": float(x), f"{prefix}y": float(y)}, args[1:]
    return {f"{prefix}target": args[0]}, args[1:]


MOD_WORDS = {"cmd", "mod", "secondary", "shift", "alt", "option", "ctrl", "control", "platform", "super", "win"}


def repeat(key):
    name, star, count = key.rpartition("*")
    if star and name and count.isdigit():
        return [name] * int(count)
    return [key]


def is_mods(s):
    return all(p in MOD_WORDS for p in s.replace("-", "+").split("+"))


def describe(spot):
    if not spot:
        return ""
    label = f" \"{spot['label']}\"" if spot.get("label") else ""
    return f" {spot['role']} {spot['id']}{label}"


def show_spot(s):
    x, y, w, h = s["bounds"]
    hidden = "" if s["visible"] else "  (clipped)"
    label = f"  \"{s['label']}\"" if s.get("label") else ""
    print(f"{s['id']}  {s['role']}{label}  {x:g},{y:g} {w:g}x{h:g}{hidden}")


def shot(inst, name, window, full):
    wins = inst.call("windows")["windows"]
    if window is None:
        win = next((w for w in wins if w["main"]), wins[0] if wins else None)
    elif str(window).isdigit():
        win = next((w for w in wins if str(w["id"]) == str(window)), None)
    else:
        win = next((w for w in wins if str(window).lower() in w["title"].lower()), None)
    if not win:
        die(f"no window {window}")
    if win["native"] is None:
        die("this platform gives no window handle to capture (Wayland?)")
    inst.call("wait", timeout=5, **{"for": "idle", "quiet": 150})
    out = inst.dir / "shots" / f"{name}.png"
    out.parent.mkdir(parents=True, exist_ok=True)
    width = 0 if full else round(win["w"])
    if WIN:
        gui = gui_exe()
        r = subprocess.run([str(gui), "shot", str(inst.pid), str(out), str(win["native"]), str(width)], capture_output=True, text=True)
        if r.returncode:
            die(r.stderr.strip() or "screenshot failed")
    elif MAC and inst_capture(inst, win, out, full):
        pass
    elif MAC:
        r = subprocess.run(["screencapture", "-x", "-o", f"-l{win['native']}", str(out)], capture_output=True, text=True)
        if r.returncode or not out.exists():
            die("screencapture failed; the shell needs Screen Recording permission: " + r.stderr.strip())
        if width:
            subprocess.run(["sips", "-z", str(round(win["h"])), str(width), str(out)], capture_output=True)
    else:
        r = subprocess.run(["import", "-silent", "-window", str(win["native"]), str(out)], capture_output=True, text=True)
        if r.returncode:
            die("needs ImageMagick `import` on X11: " + r.stderr.strip())
        if width:
            subprocess.run(["convert", str(out), "-resize", f"{width}x{round(win['h'])}!", str(out)], capture_output=True)
    print(out)


def inst_capture(inst, win, out, full):
    stderr, sys.stderr = sys.stderr, open(os.devnull, "w")
    try:
        inst.call("capture", path=str(out), full=full, window=win["id"])
        return out.exists()
    except SystemExit:
        return False
    finally:
        sys.stderr.close()
        sys.stderr = stderr


def gui_exe():
    gui = COMMON / "agent-tools" / "gui.exe"
    src = TOOLS / "gui.cs"
    if not gui.exists() or src.stat().st_mtime > gui.stat().st_mtime:
        gui.parent.mkdir(parents=True, exist_ok=True)
        tmp = gui.with_name(f"gui-{os.getpid()}.exe")
        csc = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Microsoft.NET" / "Framework64" / "v4.0.30319" / "csc.exe"
        if subprocess.run([str(csc), "-nologo", "-optimize", "-platform:x64", "-r:System.Drawing.dll", f"-out:{tmp}", str(src)], stdout=sys.stderr).returncode:
            die("gui.cs does not compile")
        try:
            os.replace(tmp, gui)
        except OSError:
            tmp.unlink(missing_ok=True)
    return gui


def perf_mark(inst):
    frames = inst.call("frames", since=2**62)
    cpu = usage(inst.pid)
    (inst.dir / "perf-from.json").write_text(json.dumps({"next": frames["next"], "cpu": cpu[0] if cpu else 0, "wall": time.time()}), encoding="utf-8")


def data_key(inst):
    notes = inst.meta.get("notes")
    return f"notes {notes}" if notes is not None else f"data {inst.meta.get('data')}"


def perf_measure(inst, scenario):
    try:
        mark = json.loads((inst.dir / "perf-from.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        die("run `app perf begin` first")
    times = sorted(ms for _, ms in inst.call("frames", since=mark["next"])["frames"])
    cpu_now = usage(inst.pid) or (0, 0)
    cpu = (cpu_now[0] - mark["cpu"]) / max((time.time() - mark["wall"]) * 1000, 1) * 100

    def pick(q):
        return times[min(int(len(times) * q), len(times) - 1)] if times else 0.0

    line = (f"frames {len(times)} p50 {pick(.5):.1f} p95 {pick(.95):.1f} max {(times[-1] if times else 0):.1f} ms, "
            f"over8ms {sum(t > 8.3 for t in times)} over16ms {sum(t > 16.7 for t in times)}, cpu {cpu:.1f}% rss {cpu_now[1]:.0f} MB")
    print(f"{scenario}: {line}")
    path = COMMON / "agent-tools" / "perf.log"
    path.parent.mkdir(parents=True, exist_ok=True)
    key = data_key(inst)
    previous = None
    if path.exists():
        for row in path.read_text(encoding="utf-8", errors="replace").splitlines():
            parts = row.split("\t")
            if len(parts) == 7 and parts[4] == scenario and parts[5] == key and parts[3] == sys.platform:
                previous = parts
    if previous:
        print(f"last ({previous[1]} on {previous[2]}, {key}): {previous[6]}")
    commit = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    branch = subprocess.run(["git", "-C", str(ROOT), "branch", "--show-current"], capture_output=True, text=True).stdout.strip() or "detached"
    with open(path, "a", encoding="utf-8") as f:
        f.write(f"{time.strftime('%Y-%m-%d %H:%M')}\t{commit}\t{branch}\t{sys.platform}\t{scenario}\t{key}\t{line}\n")


HELP = """\
app start [--name N] [--light|--dark] [--lang de|en] [--notes N] [--size WxH] [--data DIR] [--debug] [--no-build] [--visible]
                                 build, then start a new private instance: scratch data, settings and temp folder;
                                 --name N restarts your instance N (e.g. after a rebuild); --data only inside the temp folder;
                                 on Windows its windows open outside every screen unless --visible
app ps | app gc                  instances of this worktree | remove dead ones
app stop [--all]                 quit the selected instance (or all of yours)
app windows                      window id, size, position, scale, theme, title (alias: list)
app tree [filter]                elements: id role "label" x,y wxh
app find <target>                one element; error lists similar ones
app click <target|x y> [n] [right|middle] [mods]   e.g. app click id:row-3 shift, app click text:Datei 2
app hover <target|x y> [name]    move the pointer, wait, screenshot if a name is given
app move <target|x y> [steps]    glides there from the last pointer position; keeps a held button held
app down <target|x y> [right|middle] [mods] | app up <target|x y> [steps]
                                 press, then move/wheel/key while it is held, then release: a drag in parts
app drag <from> <to> [steps]     from and to are targets or x,y; press, glide, release
app wheel <target|x y> <steps> [mods] [--px]   positive steps scroll up; --px scrolls pixels like a trackpad
app key <key>... [mods]          esc enter tab space backspace delete left … f12, a, cmd-k, shift-tab, down*5 (five times);
                                 every key is pressed and released; copy, cut and paste are refused unless --clipboard
app type <text> [--paste]        types every character as a key (umlauts work); --paste inserts it at once
app action <Name> [json]         dispatches a registered action; app actions [filter] lists them
app drop <target|x y> <path>...  drops files from outside, as the file manager would (temp folder only)
app dialog <path>... | --cancel  answers the next file dialog instead of showing it (temp folder only)
app events                       what the app tried to do outside: open, reveal, trash, network, notify, dialogs, menus
app state                        route, workspace, last error
app wait [idle|<target>] [--gone] [--window T] [--timeout S]
app resize <w> <h> | app theme light|dark|system
app shot <name> [--full]         screenshot of the window, prints the path
app perf begin | end <scenario> | idle [seconds]
                                 frame = CPU from render to last paint, without the GPU present (~0.6 ms);
                                 compared with the last run of that scenario on this OS and data size
app ping                         is the UI thread responsive? exit 3 when it hangs
app stats | app sample [seconds] | app log [lines]

Options for every command: -i NAME (or APP_INSTANCE) picks the instance, -w WINDOW a window
(main, part of its title, or its id). Without -i your only instance is used; with several, -i is required.
Targets: id:sidebar-today (exact id), text:Heute (exact label), role:tab, ~heu (part of id or label),
plain text (part of id or label; exact matches first). --nth N picks among several matches.
Coordinates are logical pixels from the window's top-left, the same on every OS and scale;
screenshots have the same size unless --full. Input goes straight to the app's event loop:
the owner's mouse, keyboard and focus are never used, and the window never comes to the front.
Exit codes: 0 done, 1 error (message on stderr), 3 the UI thread hangs. Native menus cannot be
driven; one that opens is closed again and reported.
"""


def main(argv):
    inst_name, window, rest = None, None, []
    nth = None
    it = iter(argv)
    for a in it:
        if a in ("-i", "--instance"):
            inst_name = next(it, None)
        elif a in ("-w", "--window"):
            window = next(it, None)
        elif a == "--nth":
            nth = int(next(it, "0"))
        else:
            rest.append(a)
    cmd, args = (rest[0], rest[1:]) if rest else ("help", [])
    win_arg = {"window": int(window) if window and window.isdigit() else window}
    if nth is not None:
        win_arg["nth"] = nth

    if cmd in ("help", "-h", "--help"):
        print(HELP)
        return
    if cmd == "start":
        opts = {"name": None, "theme": None, "lang": None, "notes": 60, "size": (1360, 860), "data": None, "debug": False, "build": True, "timeout": 90, "ttl": 1800, "visible": False}
        a = iter(args)
        for x in a:
            if x == "--name":
                opts["name"] = next(a)
            elif x in ("--light", "--dark"):
                opts["theme"] = x[2:]
            elif x == "--notes":
                opts["notes"] = int(next(a))
            elif x == "--lang":
                opts["lang"] = next(a)
            elif x == "--size":
                w, h = next(a).lower().split("x")
                opts["size"] = (int(w), int(h))
            elif x == "--data":
                opts["data"] = next(a)
            elif x == "--debug":
                opts["debug"] = True
            elif x == "--no-build":
                opts["build"] = False
            elif x == "--visible":
                opts["visible"] = True
            elif x == "--timeout":
                opts["timeout"] = float(next(a))
            elif x == "--ttl":
                opts["ttl"] = int(next(a))
            else:
                die(f"unknown option {x}")
        if inst_name and opts["name"] and inst_name != opts["name"]:
            die(f"-i {inst_name} and --name {opts['name']} name different instances")
        if inst_name:
            opts["name"] = inst_name
        start(opts)
        return
    if cmd in ("ps", "instances"):
        owner = owner_pid()
        for i in instances():
            state = "running" if i.alive() else "stopped"
            mine = " (yours)" if owner and i.meta.get("owner") == owner else ""
            print(f"{i.name}  {state}  pid {i.pid}  owner {i.meta.get('owner')}{mine}  {i.meta.get('profile')}  {i.dir}")
        return
    if cmd == "gc":
        gc(quiet=False)
        return
    if cmd == "stop":
        if "--all" in args:
            owner = owner_pid()
            for i in instances():
                if i.alive() and i.meta.get("owner") == owner:
                    stop(i)
            return
        name = inst_name or os.environ.get("APP_INSTANCE")
        if not name and not [i for i in instances() if i.alive()]:
            return
        stop(pick_instance(name))
        return

    inst = pick_instance(inst_name)
    if cmd == "ping":
        out = inst.call("ping")
        if not out["ui"]:
            die(f"pid {out['pid']} answers, but its UI thread hangs; `app sample` shows where", 3)
        print(f"pid {out['pid']} answers, UI thread responsive")
    elif cmd in ("windows", "list"):
        for w in inst.call("windows")["windows"]:
            print_window(w)
    elif cmd == "tree":
        spots = inst.call("tree", target=args[0] if args else None, **win_arg)["spots"]
        for s in spots:
            show_spot(s)
        print(f"{len(spots)} elements", file=sys.stderr)
    elif cmd == "find":
        if not args:
            die("app find <target>")
        show_spot(inst.call("find", target=args[0], **win_arg))
    elif cmd in ("click", "move", "hover", "down", "up"):
        p, more = point_args(args)
        body = {**p, **win_arg}
        if cmd in ("click", "down", "up", "move"):
            for x in more:
                if x.isdigit():
                    body["count" if cmd == "click" else "steps"] = int(x)
                elif x in ("right", "middle") and cmd in ("click", "down"):
                    body["button"] = x
                elif is_mods(x):
                    body["mods"] = x
                else:
                    die(f"{cmd} does not understand {x}")
        out = inst.call("move" if cmd == "hover" else cmd, **body)
        at = out["at"]
        held = f"  ({out['held']} button held)" if out.get("held") else ""
        print(f"{cmd} {at[0]:g},{at[1]:g}{describe(out.get('spot'))}{held}")
        if cmd == "hover":
            inst.call("wait", timeout=5, **{"for": "idle", "quiet": 350})
            if more:
                shot(inst, more[0], window, False)
    elif cmd == "drag":
        p, more = point_args(args, "from_")
        q, more = point_args(more, "to_")
        body = {**p, **q, **win_arg}
        if more and more[0].isdigit():
            body["steps"] = int(more[0])
        out = inst.call("drag", **body)
        print(f"dragged from {out['at'][0]:g},{out['at'][1]:g}{describe(out.get('spot'))}")
    elif cmd == "wheel":
        p, more = point_args(args)
        if not more or not is_num(more[0]):
            die("app wheel <target|x y> <steps> [mods]")
        steps = int(float(more[0]))
        body = {**p, **win_arg, "dy": 1 if steps > 0 else -1, "steps": abs(steps)}
        for x in more[1:]:
            if x == "--px":
                body["pixels"] = True
                body["dy"] = steps
                body["steps"] = 1
            elif is_mods(x):
                body["mods"] = x
            else:
                die(f"wheel does not understand {x}")
        inst.call("wheel", **body)
    elif cmd == "key":
        if not args:
            die("app key <key>... [mods]")
        clip = "--clipboard" in args
        keys = [k for k in args if k != "--clipboard"]
        if not keys:
            die("app key <key>... [mods]")
        if len(keys) >= 2 and is_mods(keys[-1]) and not is_mods(keys[0]):
            mods = keys.pop().replace("+", "-")
            keys = [f"{mods}-{k}" for k in keys]
        keys = [part for k in keys for part in repeat(k)]
        inst.call("key", keys=keys, clipboard=clip, **win_arg)
    elif cmd == "type":
        paste = "--paste" in args
        text = " ".join(a for a in args if a != "--paste")
        inst.call("type", text=text, paste=paste, **win_arg)
    elif cmd == "action":
        if not args:
            die("app action <Name> [json]")
        clip = "--clipboard" in args
        rest_args = [a for a in args if a != "--clipboard"]
        inst.call("action", name=rest_args[0], data=json.loads(rest_args[1]) if len(rest_args) > 1 else None, clipboard=clip, **win_arg)
    elif cmd == "actions":
        for a in inst.call("actions")["actions"]:
            if not args or args[0].lower() in a.lower():
                print(a)
    elif cmd == "drop":
        p, paths = point_args(args)
        if not paths:
            die("app drop <target|x y> <path>...")
        inst.call("drop", paths=[str(scratch_only(x, "drop")) for x in paths], **p, **win_arg)
    elif cmd == "dialog":
        inst.call("dialog", paths=[] if "--cancel" in args else [str(scratch_only(x, "dialog")) for x in args])
        print("cancel" if "--cancel" in args or not args else f"next dialog answers {len(args)} path(s)")
    elif cmd == "events":
        for e in inst.call("events")["events"]:
            print(e)
    elif cmd == "state":
        out = inst.call("state")
        print(json.dumps({k: v for k, v in out.items() if k != "ok"}, ensure_ascii=False, indent=1))
    elif cmd == "wait":
        timeout = 10.0
        body = {**win_arg}
        what = "idle"
        a = iter(args)
        for x in a:
            if x == "--timeout":
                timeout = float(next(a))
            elif x == "--gone":
                what = "gone"
            elif x == "idle":
                pass
            elif x == "--window":
                what = "window"
                body["window"] = next(a)
            elif x.startswith("--") or "target" in body:
                die(f"wait does not understand {x}; app wait [idle|<target>] [--gone] [--window T] [--timeout S]")
            else:
                body["target"] = x
                what = "gone" if what == "gone" else "target"
        out = inst.call("wait", timeout=timeout, **{"for": what}, **body)
        print(f"waited {out['waited']} ms")
    elif cmd == "resize":
        inst.call("resize", w=float(args[0]), h=float(args[1]), **win_arg)
    elif cmd == "theme":
        inst.call("theme", name=args[0] if args else "system", **win_arg)
    elif cmd == "shot":
        if not args:
            die("app shot <name> [--full]")
        shot(inst, args[0], window, "--full" in args)
    elif cmd == "perf":
        sub = args[0] if args else ""
        if sub == "begin":
            perf_mark(inst)
        elif sub == "end":
            if len(args) < 2:
                die("app perf end <scenario>")
            perf_measure(inst, args[1])
        elif sub == "idle":
            perf_mark(inst)
            time.sleep(float(args[1]) if len(args) > 1 else 3)
            perf_measure(inst, "idle")
        else:
            die("app perf begin | end <scenario> | idle [seconds]")
    elif cmd == "stats":
        u = usage(inst.pid)
        if not u:
            die("no stats")
        print(f"cpu time {u[0]:.0f} ms  rss {u[1]:.0f} MB")
    elif cmd == "sample":
        secs = args[0] if args else "3"
        if WIN:
            sys.exit(subprocess.run([str(gui_exe()), "sample", str(inst.pid), secs]).returncode)
        elif MAC:
            sys.exit(subprocess.run(["sample", str(inst.pid), secs]).returncode)
        else:
            die("sample needs perf or gdb on Linux; try: perf top -p " + str(inst.pid))
    elif cmd == "log":
        print(inst.log_tail(int(args[0]) if args else 40))
    else:
        die(f"unknown command {cmd}; app help lists them")


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
    main(sys.argv[1:])
