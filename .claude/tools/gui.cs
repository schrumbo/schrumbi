using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32.SafeHandles;

static class Gui {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h, uint cmd);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool PeekMessage(out MSG m, IntPtr h, uint a, uint b, uint r);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
    [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint type);
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("user32.dll")] static extern bool GetKeyboardState(byte[] s);
    [DllImport("user32.dll")] static extern bool SetKeyboardState(byte[] s);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll")] static extern bool SetHandleInformation(SafeFileHandle h, uint mask, uint flags);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcess(string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string dir, ref STARTUPINFOEX si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool InitializeProcThreadAttributeList(IntPtr list, int count, int flags, ref IntPtr size);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool UpdateProcThreadAttribute(IntPtr list, uint flags, IntPtr attr, IntPtr value, IntPtr size, IntPtr prev, IntPtr ret);
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] static extern IntPtr OpenThread(uint access, bool inherit, uint tid);
    [DllImport("kernel32.dll")] static extern uint SuspendThread(IntPtr h);
    [DllImport("kernel32.dll")] static extern uint ResumeThread(IntPtr h);
    [DllImport("kernel32.dll")] static extern bool GetThreadContext(IntPtr h, IntPtr ctx);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", CharSet = CharSet.Ansi)] static extern IntPtr GetProcAddress(IntPtr m, string name);
    [DllImport("dbghelp.dll", SetLastError = true)] static extern bool SymInitialize(IntPtr p, string path, bool invade);
    [DllImport("dbghelp.dll")] static extern uint SymSetOptions(uint o);
    [DllImport("dbghelp.dll")] static extern bool SymFromAddr(IntPtr p, ulong addr, out ulong disp, IntPtr info);
    [DllImport("dbghelp.dll")] static extern bool SymRefreshModuleList(IntPtr p);
    [DllImport("dbghelp.dll")] static extern bool StackWalk64(uint machine, IntPtr p, IntPtr t, IntPtr frame, IntPtr ctx, IntPtr read, IntPtr table, IntPtr modBase, IntPtr translate);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr LoadLibrary(string name);
    [DllImport("winmm.dll")] static extern uint timeBeginPeriod(uint ms);

    struct RECT { public int L, T, R, B; }
    struct POINT { public int X, Y; }
    struct MSG { public IntPtr h; public uint m; public IntPtr w, l; public uint t; public POINT p; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO {
        public int cb; public string reserved, desktop, title;
        public int x, y, w, h, cx, cy, fill, flags; public short show, r2; public IntPtr r3, stdin, stdout, stderr;
    }
    struct STARTUPINFOEX { public STARTUPINFO si; public IntPtr attributes; }
    struct PROCESS_INFORMATION { public IntPtr process, thread; public int pid, tid; }

    static readonly Dictionary<string, int> Mods = new Dictionary<string, int> { { "ctrl", 0x11 }, { "cmd", 0x11 }, { "shift", 0x10 }, { "alt", 0x12 } };
    static readonly Dictionary<string, int> Names = new Dictionary<string, int> {
        { "esc", 0x1B }, { "escape", 0x1B }, { "enter", 0x0D }, { "return", 0x0D }, { "tab", 0x09 }, { "space", 0x20 },
        { "backspace", 0x08 }, { "delete", 0x2E }, { "left", 0x25 }, { "up", 0x26 }, { "right", 0x27 }, { "down", 0x28 },
        { "home", 0x24 }, { "end", 0x23 }, { "pageup", 0x21 }, { "pagedown", 0x22 }, { ",", 0xBC }, { ".", 0xBE }, { "-", 0xBD }, { "+", 0xBB }
    };

    static int Main(string[] a) {
        SetProcessDPIAware();
        try { return Run(a); } catch (Exception e) { Console.Error.WriteLine(e.Message); return 1; }
    }

    static int Run(string[] a) {
        string cmd = a.Length > 0 ? a[0] : "";
        if (cmd == "start") return Start(a[1], a[2], a.Skip(3).ToArray());
        if (cmd == "alive") { try { return Process.GetProcessById(int.Parse(a[1])).HasExited ? 1 : 0; } catch { return 1; } }
        if (cmd == "kill") { try { Process.GetProcessById(int.Parse(a[1])).Kill(); } catch { } return 0; }
        if (cmd == "stats") {
            var p = Process.GetProcessById(int.Parse(a[1]));
            Console.WriteLine("{0:F0} {1}", p.TotalProcessorTime.TotalMilliseconds, p.WorkingSet64 / 1024);
            return 0;
        }
        if (cmd == "demangle") { Console.WriteLine(Demangle(a[1])); return 0; }
        if (cmd == "sample") return Sample(uint.Parse(a[1]), a.Length > 2 ? double.Parse(a[2]) : 3);
        uint pid = uint.Parse(a[1]);
        var wins = Windows(pid);
        if (cmd == "list") {
            foreach (var w in wins) {
                RECT r; GetClientRect(w, out r); var o = Origin(w); var t = new StringBuilder(256); GetWindowText(w, t, 256);
                Console.WriteLine("{0} {1} {2} {3} {4} {5}", w.ToInt64(), o.X, o.Y, r.R, r.B, t);
            }
            return 0;
        }
        if (wins.Count == 0) { Console.Error.WriteLine("no window for pid " + pid); return 1; }
        IntPtr h = wins[0];
        switch (cmd) {
            case "shot": Shot(a.Length > 3 ? (IntPtr)long.Parse(a[3]) : h, a[2], a.Length > 4 ? int.Parse(a[4]) : -1); break;
            case "move": Move(h, int.Parse(a[2]), int.Parse(a[3])); break;
            case "click": {
                int n = a.Length > 4 && a[4].All(char.IsDigit) ? int.Parse(a[4]) : 1;
                bool right = a.Skip(4).Contains("right");
                string mods = a.Skip(4).FirstOrDefault(s => s.Split('+').All(Mods.ContainsKey)) ?? "";
                WithMods(h, mods, () => Click(h, int.Parse(a[2]), int.Parse(a[3]), n, right, mods));
                break;
            }
            case "key": WithMods(h, a.Length > 3 ? a[3] : "", () => Key(h, a[2], a.Length > 3 ? a[3] : "")); break;
            case "type": Type(h, a[2]); break;
            case "wheel": WithMods(h, a.Length > 5 ? a[5] : "", () => Wheel(h, int.Parse(a[2]), int.Parse(a[3]), int.Parse(a[4]))); break;
            case "release": break;
            default: Console.Error.WriteLine("unknown command " + cmd); return 1;
        }
        Release(pid, h);
        return 0;
    }

    static List<IntPtr> Windows(uint pid) {
        var found = new List<IntPtr>();
        EnumWindows((h, l) => {
            uint owner; GetWindowThreadProcessId(h, out owner);
            RECT r;
            if (owner == pid && IsWindowVisible(h) && GetWindow(h, 4) == IntPtr.Zero && GetClientRect(h, out r) && (r.R > 0 || IsIconic(h))) found.Add(h);
            return true;
        }, IntPtr.Zero);
        return found.OrderByDescending(h => { RECT r; GetClientRect(h, out r); return (long)r.R * r.B; }).ToList();
    }

    static POINT Origin(IntPtr h) { var p = new POINT(); ClientToScreen(h, ref p); return p; }
    static IntPtr XY(int x, int y) { return (IntPtr)(((y & 0xFFFF) << 16) | (x & 0xFFFF)); }

    static void Release(uint pid, IntPtr h) {
        uint owner; GetWindowThreadProcessId(GetForegroundWindow(), out owner);
        if (owner != pid) return;
        string prev = Environment.GetEnvironmentVariable("GUI_PREV");
        ShowWindow(h, 6);
        Thread.Sleep(150);
        long p;
        if (prev != null && long.TryParse(prev, out p) && p != 0) SetForegroundWindow((IntPtr)p);
        Thread.Sleep(80);
        ShowWindow(h, 4);
    }

    static int Start(string log, string exe, string[] args) {
        IntPtr prev = GetForegroundWindow();
        var fs = new FileStream(log, FileMode.Append, FileAccess.Write, FileShare.ReadWrite | FileShare.Delete);
        SetHandleInformation(fs.SafeFileHandle, 1, 1);
        var ex = new STARTUPINFOEX();
        ex.si.cb = Marshal.SizeOf(ex);
        ex.si.flags = 0x100 | 0x1;
        ex.si.show = 4;
        ex.si.stdout = fs.SafeFileHandle.DangerousGetHandle();
        ex.si.stderr = ex.si.stdout;
        IntPtr size = IntPtr.Zero;
        InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref size);
        ex.attributes = Marshal.AllocHGlobal(size);
        InitializeProcThreadAttributeList(ex.attributes, 1, 0, ref size);
        IntPtr only = Marshal.AllocHGlobal(IntPtr.Size);
        Marshal.WriteIntPtr(only, ex.si.stdout);
        UpdateProcThreadAttribute(ex.attributes, 0, (IntPtr)0x20002, only, (IntPtr)IntPtr.Size, IntPtr.Zero, IntPtr.Zero);
        var line = new StringBuilder("\"" + exe + "\"");
        foreach (var x in args) line.Append(" \"" + x + "\"");
        PROCESS_INFORMATION pi;
        if (!CreateProcess(null, line, IntPtr.Zero, IntPtr.Zero, true, 0x08000000 | 0x00080000, IntPtr.Zero, null, ref ex, out pi)) {
            Console.Error.WriteLine("could not start " + exe + ": error " + Marshal.GetLastWin32Error());
            return 1;
        }
        fs.Close();
        Environment.SetEnvironmentVariable("GUI_PREV", prev.ToInt64().ToString());
        var proc = Process.GetProcessById(pi.pid);
        var until = DateTime.Now.AddSeconds(60);
        IntPtr h = IntPtr.Zero;
        while (DateTime.Now < until && h == IntPtr.Zero) {
            if (proc.HasExited) { Console.Error.WriteLine("app exited"); return 1; }
            var w = Windows((uint)pi.pid);
            if (w.Count > 0) h = w[0];
            Thread.Sleep(20);
        }
        if (h == IntPtr.Zero) { Console.Error.WriteLine("no window after 60 s"); return 1; }
        for (int i = 0; i < 100; i++) { Release((uint)pi.pid, h); Thread.Sleep(30); }
        Console.WriteLine("{0} {1}", pi.pid, prev.ToInt64());
        return 0;
    }

    static void Shot(IntPtr h, string outPath, int width) {
        RECT wr; GetWindowRect(h, out wr);
        RECT cr; GetClientRect(h, out cr);
        var o = Origin(h);
        using (var full = new Bitmap(wr.R - wr.L, wr.B - wr.T)) {
            using (var g = Graphics.FromImage(full)) { var dc = g.GetHdc(); PrintWindow(h, dc, 2); g.ReleaseHdc(dc); }
            var crop = new Rectangle(o.X - wr.L, o.Y - wr.T, cr.R, cr.B);
            int w = cr.R, hh = cr.B;
            int limit = width < 0 ? Math.Min(w, 1500) : width == 0 ? w : width;
            if (w != limit) { hh = (int)Math.Round(hh * (double)limit / w); w = limit; }
            using (var outBmp = new Bitmap(w, hh)) {
                using (var g = Graphics.FromImage(outBmp)) { g.InterpolationMode = InterpolationMode.HighQualityBicubic; g.DrawImage(full, new Rectangle(0, 0, w, hh), crop, GraphicsUnit.Pixel); }
                Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(outPath)));
                outBmp.Save(outPath, ImageFormat.Png);
            }
        }
    }

    static void Move(IntPtr h, int x, int y) {
        for (int i = 0; i < 3; i++) { PostMessage(h, 0x0200, IntPtr.Zero, XY(x, y)); Thread.Sleep(16); }
    }

    static int MouseMods(string mods) {
        int f = 0;
        foreach (var m in mods.Split(new[] { '+' }, StringSplitOptions.RemoveEmptyEntries)) { if (m == "shift") f |= 4; else if (m == "ctrl" || m == "cmd") f |= 8; }
        return f;
    }

    static void Click(IntPtr h, int x, int y, int n, bool right, string mods) {
        var l = XY(x, y);
        int f = MouseMods(mods);
        PostMessage(h, 0x0200, (IntPtr)f, l);
        Thread.Sleep(60);
        uint down = right ? 0x0204u : 0x0201u;
        int held = right ? 2 : 1;
        for (int i = 0; i < n; i++) {
            PostMessage(h, down, (IntPtr)(f | held), l); Thread.Sleep(30);
            PostMessage(h, down + 1, (IntPtr)f, l); Thread.Sleep(60);
        }
    }

    static void WithMods(IntPtr h, string mods, Action act) {
        var keys = mods.Split(new[] { '+' }, StringSplitOptions.RemoveEmptyEntries).Where(Mods.ContainsKey).Select(m => Mods[m]).Distinct().ToList();
        if (keys.Count == 0) { act(); Thread.Sleep(30); return; }
        MSG msg; PeekMessage(out msg, IntPtr.Zero, 0, 0, 0);
        uint ignored;
        uint target = GetWindowThreadProcessId(h, out ignored);
        uint me = GetCurrentThreadId();
        AttachThreadInput(me, target, true);
        var state = new byte[256];
        GetKeyboardState(state);
        foreach (var k in keys) { state[k] = 0x80; state[Left(k)] = 0x80; }
        SetKeyboardState(state);
        Thread.Sleep(30);
        act();
        Thread.Sleep(250);
        foreach (var k in keys) { state[k] = 0; state[Left(k)] = 0; }
        SetKeyboardState(state);
        AttachThreadInput(me, target, false);
    }

    static int Left(int k) { return k == 0x11 ? 0xA2 : k == 0x10 ? 0xA0 : 0xA4; }

    static int VirtualKey(string k) {
        int v;
        if (Names.TryGetValue(k.ToLowerInvariant(), out v)) return v;
        if (k.Length == 1 && char.IsLetterOrDigit(k[0])) return char.ToUpperInvariant(k[0]);
        if (k.Length > 1 && (k[0] == 'f' || k[0] == 'F') && int.TryParse(k.Substring(1), out v)) return 0x6F + v;
        return int.Parse(k);
    }

    static void Key(IntPtr h, string key, string mods) {
        int vk = VirtualKey(key);
        bool alt = mods.Split('+').Contains("alt");
        uint scan = MapVirtualKey((uint)vk, 0);
        bool ext = (vk >= 0x21 && vk <= 0x28) || vk == 0x2D || vk == 0x2E;
        long l = 1 | (scan << 16) | (ext ? 1L << 24 : 0) | (alt ? 1L << 29 : 0);
        PostMessage(h, alt ? 0x0104u : 0x0100u, (IntPtr)vk, (IntPtr)l);
        Thread.Sleep(20);
        PostMessage(h, alt ? 0x0105u : 0x0101u, (IntPtr)vk, (IntPtr)(l | (1L << 30) | (1L << 31)));
    }

    static void Type(IntPtr h, string text) {
        foreach (char c in text) {
            if (c == '\n') { Key(h, "enter", ""); Thread.Sleep(15); continue; }
            PostMessage(h, 0x0102, (IntPtr)c, (IntPtr)1);
            Thread.Sleep(15);
        }
    }

    static void Wheel(IntPtr h, int x, int y, int steps) {
        var o = Origin(h);
        PostMessage(h, 0x0200, IntPtr.Zero, XY(x, y));
        Thread.Sleep(60);
        int delta = steps > 0 ? 120 : -120;
        for (int i = 0; i < Math.Abs(steps); i++) {
            PostMessage(h, 0x020A, (IntPtr)((delta & 0xFFFF) << 16), XY(o.X + x, o.Y + y));
            Thread.Sleep(40);
        }
    }

    static int Sample(uint pid, double seconds) {
        var wins = Windows(pid);
        if (wins.Count == 0) { Console.Error.WriteLine("no window for pid " + pid); return 1; }
        uint ignored;
        uint tid = GetWindowThreadProcessId(wins[0], out ignored);
        IntPtr proc = OpenProcess(0x0400 | 0x0010, false, pid);
        IntPtr thread = OpenThread(0x0002 | 0x0008 | 0x0040, false, tid);
        if (proc == IntPtr.Zero || thread == IntPtr.Zero) { Console.Error.WriteLine("cannot open the process"); return 1; }
        SymSetOptions(0x2 | 0x4 | 0x10);
        string dir = Path.GetDirectoryName(Process.GetProcessById((int)pid).MainModule.FileName);
        timeBeginPeriod(1);
        if (!SymInitialize(proc, dir, true)) { Console.Error.WriteLine("SymInitialize failed " + Marshal.GetLastWin32Error()); return 1; }
        IntPtr dbghelp = LoadLibrary("dbghelp.dll");
        IntPtr table = GetProcAddress(dbghelp, "SymFunctionTableAccess64");
        IntPtr modBase = GetProcAddress(dbghelp, "SymGetModuleBase64");
        IntPtr raw = Marshal.AllocHGlobal(1232 + 16);
        IntPtr ctx = (IntPtr)(((long)raw + 15) & ~15L);
        IntPtr frame = Marshal.AllocHGlobal(1024);
        var stacks = new List<ulong[]>();
        var until = DateTime.Now.AddSeconds(seconds);
        while (DateTime.Now < until) {
            if (SuspendThread(thread) == 0xFFFFFFFF) break;
            var pcs = new List<ulong>();
            for (int i = 0; i < 1232; i += 8) Marshal.WriteInt64(ctx, i, 0);
            Marshal.WriteInt32(ctx, 0x30, 0x10000B);
            if (GetThreadContext(thread, ctx)) {
                for (int i = 0; i < 1024; i += 8) Marshal.WriteInt64(frame, i, 0);
                Marshal.WriteInt64(frame, 0, Marshal.ReadInt64(ctx, 0xF8));
                Marshal.WriteInt32(frame, 12, 3);
                Marshal.WriteInt64(frame, 32, Marshal.ReadInt64(ctx, 0xA0));
                Marshal.WriteInt32(frame, 44, 3);
                Marshal.WriteInt64(frame, 48, Marshal.ReadInt64(ctx, 0x98));
                Marshal.WriteInt32(frame, 60, 3);
                while (pcs.Count < 160 && StackWalk64(0x8664, proc, thread, frame, ctx, IntPtr.Zero, table, modBase, IntPtr.Zero)) {
                    ulong pc = (ulong)Marshal.ReadInt64(frame, 0);
                    if (pc == 0) break;
                    pcs.Add(pc);
                }
            }
            ResumeThread(thread);
            if (pcs.Count > 0) stacks.Add(pcs.ToArray());
            Thread.Sleep(1);
        }
        IntPtr info = Marshal.AllocHGlobal(88 + 1024);
        var names = new Dictionary<ulong, string>();
        Func<ulong, string> name = pc => {
            string s;
            if (names.TryGetValue(pc, out s)) return s;
            for (int i = 0; i < 88; i += 4) Marshal.WriteInt32(info, i, 0);
            Marshal.WriteInt32(info, 0, 88);
            Marshal.WriteInt32(info, 80, 1000);
            ulong disp;
            s = SymFromAddr(proc, pc, out disp, info) ? Demangle(Marshal.PtrToStringAnsi((IntPtr)((long)info + 84), Marshal.ReadInt32(info, 76))) : "0x" + pc.ToString("x");
            names[pc] = s;
            return s;
        };
        var named = stacks.Select(st => st.Select(name).ToArray()).ToList();
        int total = named.Count;
        Func<string, bool> waiting = s => s.Contains("MsgWaitForMultipleObjects") || s.Contains("NtUserGetMessage") || s.Contains("WaitForSingleObject") || s.Contains("NtWaitFor") || s.Contains("ZwWaitFor") || s.Contains("NtUserMsgWait");
        int idle = named.Count(st => waiting(st[0]));
        Console.WriteLine("UI thread {0}: {1} samples in {2} s, {3} waiting for input ({4:F0}% busy)", tid, total, seconds, idle, total == 0 ? 0 : 100.0 * (total - idle) / total);
        var busy = named.Where(st => !waiting(st[0])).ToList();
        Console.WriteLine();
        Console.WriteLine("Inclusive (busy samples that pass through the function):");
        foreach (var g in busy.SelectMany(st => st.Distinct()).GroupBy(s => s).OrderByDescending(g => g.Count()).Take(30))
            Console.WriteLine("{0,6} {1,5:F1}%  {2}", g.Count(), 100.0 * g.Count() / Math.Max(1, busy.Count), Short(g.Key));
        Console.WriteLine();
        Console.WriteLine("App code (busy samples that pass through schrumbi):");
        foreach (var g in busy.SelectMany(st => st.Where(f => f.Contains("schrumbi::") && !f.StartsWith("schrumbi::main") && !f.Contains("Application::run")).Distinct()).GroupBy(f => f).OrderByDescending(g => g.Count()).Take(20))
            Console.WriteLine("{0,6} {1,5:F1}%  {2}", g.Count(), 100.0 * g.Count() / Math.Max(1, busy.Count), Short(g.Key));
        Console.WriteLine();
        Console.WriteLine("Self (busy samples on top of the stack):");
        foreach (var g in busy.GroupBy(st => st[0]).OrderByDescending(g => g.Count()).Take(15))
            Console.WriteLine("{0,6} {1,5:F1}%  {2}", g.Count(), 100.0 * g.Count() / Math.Max(1, busy.Count), Short(g.Key));
        Console.WriteLine();
        Console.WriteLine("Heaviest stacks (app frames only, innermost first):");
        foreach (var g in busy.Select(st => string.Join("\n        <- ", st.Where(f => f.Contains("schrumbi") || f.Contains("gpui")).Select(Short).Take(12))).Where(s => s.Length > 0).GroupBy(s => s).OrderByDescending(g => g.Count()).Take(8)) {
            Console.WriteLine("{0,6}  {1}", g.Count(), g.Key);
        }
        return 0;
    }

    static string Short(string s) { return s.Length > 200 ? s.Substring(0, 197) + "..." : s; }

    static string Demangle(string s) {
        int dot = s.IndexOf(".llvm.");
        if (dot > 0) s = s.Substring(0, dot);
        if (s.StartsWith("_R") || s.StartsWith("R")) {
            try { return new V0(s.Substring(s.StartsWith("_R") ? 2 : 1)).Path(); } catch { return s; }
        }
        string body = s.StartsWith("_ZN") ? s.Substring(3) : s.StartsWith("__ZN") ? s.Substring(4) : null;
        if (body == null) return s;
        var parts = new List<string>();
        int i = 0;
        while (i < body.Length && char.IsDigit(body[i])) {
            int j = i;
            while (j < body.Length && char.IsDigit(body[j])) j++;
            int len = int.Parse(body.Substring(i, j - i));
            if (j + len > body.Length) break;
            parts.Add(body.Substring(j, len));
            i = j + len;
        }
        if (parts.Count > 1 && parts[parts.Count - 1].Length == 17 && parts[parts.Count - 1][0] == 'h') parts.RemoveAt(parts.Count - 1);
        var text = string.Join("::", parts);
        text = text.Replace("$LT$", "<").Replace("$GT$", ">").Replace("$RF$", "&").Replace("$BP$", "*").Replace("$C$", ",").Replace("$u20$", " ").Replace("$u27$", "'").Replace("$u5b$", "[").Replace("$u5d$", "]").Replace("$u7b$", "{").Replace("$u7d$", "}").Replace("$LP$", "(").Replace("$RP$", ")").Replace("..", "::");
        return text;
    }
}

class V0 {
    readonly string s;
    int p;
    public V0(string text) { s = text; }

    char Peek() { return p < s.Length ? s[p] : (char)0; }
    char Next() { return s[p++]; }

    long Base62() {
        if (Peek() == '_') { p++; return 0; }
        long v = 0;
        while (Peek() != '_') {
            char c = Next();
            int d = char.IsDigit(c) ? c - '0' : char.IsLower(c) ? c - 'a' + 10 : c - 'A' + 36;
            v = v * 62 + d;
        }
        p++;
        return v + 1;
    }

    void Dis() { if (Peek() == 's') { p++; Base62(); } }

    string Ident() {
        Dis();
        if (Peek() == 'u') p++;
        int start = p;
        if (Peek() == '0') p++;
        else while (char.IsDigit(Peek())) p++;
        int len = int.Parse(s.Substring(start, p - start));
        if (Peek() == '_') p++;
        string id = s.Substring(p, len);
        p += len;
        return id;
    }

    T Back<T>(Func<T> f) {
        int at = (int)Base62();
        int keep = p;
        p = at;
        T r = f();
        p = keep;
        return r;
    }

    public string Path() {
        char c = Next();
        switch (c) {
            case 'C': return Ident();
            case 'M': { Dis(); Path(); return Type(); }
            case 'X': { Dis(); Path(); string t = Type(); return "<" + t + " as " + Path() + ">"; }
            case 'Y': { string t = Type(); return "<" + t + " as " + Path() + ">"; }
            case 'N': {
                char ns = Next();
                string prefix = Path();
                string id = Ident();
                if (char.IsUpper(ns)) return prefix + "::{" + (ns == 'C' ? "closure" : ns == 'S' ? "shim" : ns.ToString()) + (id.Length > 0 ? ":" + id : "") + "}";
                return prefix + "::" + id;
            }
            case 'I': {
                string path = Path();
                var args = new List<string>();
                while (Peek() != 'E') { string g = Generic(); if (g != null) args.Add(g); }
                p++;
                return args.Count > 0 ? path + "<" + string.Join(", ", args) + ">" : path;
            }
            case 'B': return Back(Path);
        }
        throw new FormatException();
    }

    string Generic() {
        if (Peek() == 'L') { p++; Base62(); return null; }
        if (Peek() == 'K') { p++; Const(); return "_"; }
        return Type();
    }

    void Const() {
        if (Peek() == 'p') { p++; return; }
        if (Peek() == 'B') { p++; Back(() => { Const(); return 0; }); return; }
        Type();
        if (Peek() == 'n') p++;
        while (Peek() != '_') p++;
        p++;
    }

    static readonly Dictionary<char, string> Basic = new Dictionary<char, string> {
        { 'a', "i8" }, { 'b', "bool" }, { 'c', "char" }, { 'd', "f64" }, { 'e', "str" }, { 'f', "f32" }, { 'h', "u8" }, { 'i', "isize" },
        { 'j', "usize" }, { 'l', "i32" }, { 'm', "u32" }, { 'n', "i128" }, { 'o', "u128" }, { 's', "i16" }, { 't', "u16" }, { 'u', "()" },
        { 'v', "..." }, { 'x', "i64" }, { 'y', "u64" }, { 'z', "!" }, { 'p', "_" }
    };

    string Type() {
        char c = Peek();
        string b;
        if (Basic.TryGetValue(c, out b)) { p++; return b; }
        switch (c) {
            case 'A': { p++; string t = Type(); Const(); return "[" + t + "; _]"; }
            case 'S': { p++; return "[" + Type() + "]"; }
            case 'T': { p++; var ts = new List<string>(); while (Peek() != 'E') ts.Add(Type()); p++; return "(" + string.Join(", ", ts) + ")"; }
            case 'R': case 'Q': { p++; if (Peek() == 'L') { p++; Base62(); } return (c == 'R' ? "&" : "&mut ") + Type(); }
            case 'P': { p++; return "*const " + Type(); }
            case 'O': { p++; return "*mut " + Type(); }
            case 'F': {
                p++;
                if (Peek() == 'G') { p++; Base62(); }
                if (Peek() == 'U') p++;
                if (Peek() == 'K') { p++; if (Peek() == 'C') p++; else Ident(); }
                var ts = new List<string>();
                while (Peek() != 'E') ts.Add(Type());
                p++;
                return "fn(" + string.Join(", ", ts) + ") -> " + Type();
            }
            case 'D': {
                p++;
                if (Peek() == 'G') { p++; Base62(); }
                var traits = new List<string>();
                while (Peek() != 'E') {
                    traits.Add(Path());
                    while (Peek() == 'p') { p++; Ident(); Type(); }
                }
                p++;
                if (Peek() == 'L') { p++; Base62(); }
                return "dyn " + string.Join(" + ", traits);
            }
            case 'B': { p++; return Back(Type); }
        }
        return Path();
    }
}
