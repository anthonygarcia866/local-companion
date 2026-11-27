# Agent sandbox network limit, part 1 of 2 (docs/agent-sandbox-plan.md §2):
# the allowlisting proxy glim-agent's processes use. Runs as YOU, not
# elevated, listening on 127.0.0.1 only:
#   powershell -ExecutionPolicy Bypass -File scripts\agent-sandbox\proxy.ps1
#
# HTTPS CONNECT only, to port 443 of exactly these hosts (no TLS
# interception: it never sees what goes through):
#   github.com, api.github.com, codeload.github.com,
#   objects.githubusercontent.com, api.anthropic.com
# Anything else is refused: other methods, other ports, other hosts, IP
# literals, and a host whose DNS answer is on this PC or the local network
# (DNS rebinding to your router or a LAN device).
#
# Part 2 is the firewall (setup.ps1 step 8): glim-agent's SID may connect to
# nothing but 127.0.0.1 on this port, so a program that ignores HTTPS_PROXY
# just can't connect. The proxy is a convenience for the allowlist, the
# firewall is the enforcement.
#
# Logs one line per connection to %LOCALAPPDATA%\Glim\agent-proxy.log: time,
# host:port asked for, allowed/refused and why, bytes each way. Never
# content (it can't see any).
param(
    [int]$Port = 47643,
    [switch]$Echo   # also print each log line (tests)
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Threading;

public static class GlimProxy {
    public static readonly string[] Hosts = {
        "github.com", "api.github.com", "codeload.github.com",
        "objects.githubusercontent.com", "api.anthropic.com"
    };
    const int MaxConns = 64, MaxHead = 8192, HeadMs = 10000, IdleMs = 300000;
    static int conns;
    static string logPath;
    static readonly object logLock = new object();
    public static bool Echo;

    // The decision, alone so tests can call it: null = allowed, else why not.
    public static string Judge(string requestLine, out string host) {
        host = null;
        if (requestLine == null) return "no-request";
        string[] p = requestLine.Split(' ');
        if (p.Length != 3 || !p[2].StartsWith("HTTP/1.")) return "bad-request";
        if (p[0] != "CONNECT") return "not-connect";
        string target = p[1];
        int colon = target.LastIndexOf(':');
        if (colon <= 0 || target.IndexOf(':') != colon) return "bad-target";   // IPv6 literals have more colons
        string h = target.Substring(0, colon).ToLowerInvariant();
        if (h.EndsWith(".")) h = h.Substring(0, h.Length - 1);
        if (target.Substring(colon + 1) != "443") return "not-443";
        IPAddress ip;
        if (IPAddress.TryParse(h, out ip)) return "ip-literal";
        if (Array.IndexOf(Hosts, h) < 0) return "not-allowed";
        host = h;
        return null;
    }

    // Public internet addresses only: no loopback, private, link-local,
    // CGNAT, multicast, unspecified, IPv6 ULA / site-local / mapped.
    public static bool Public(IPAddress a) {
        if (IPAddress.IsLoopback(a)) return false;
        if (a.AddressFamily == AddressFamily.InterNetwork) {
            byte[] b = a.GetAddressBytes();
            if (b[0] == 0 || b[0] == 10 || b[0] == 127 || b[0] >= 224) return false;
            if (b[0] == 169 && b[1] == 254) return false;
            if (b[0] == 172 && b[1] >= 16 && b[1] <= 31) return false;
            if (b[0] == 192 && b[1] == 168) return false;
            if (b[0] == 100 && b[1] >= 64 && b[1] <= 127) return false;
            return true;
        }
        if (a.AddressFamily == AddressFamily.InterNetworkV6) {
            if (a.IsIPv6LinkLocal || a.IsIPv6SiteLocal || a.IsIPv6Multicast || a.IsIPv4MappedToIPv6) return false;
            byte[] b = a.GetAddressBytes();
            if ((b[0] & 0xFE) == 0xFC) return false;          // fc00::/7
            bool zero = true; foreach (byte x in b) if (x != 0) { zero = false; break; }
            if (zero) return false;
            return true;
        }
        return false;
    }

    public static void Run(int port, string log) {
        logPath = log;
        var l = new TcpListener(IPAddress.Loopback, port);
        l.ExclusiveAddressUse = true;
        l.Start();
        Log("listening on 127.0.0.1:" + port);
        while (true) {
            TcpClient c = l.AcceptTcpClient();
            if (Interlocked.Increment(ref conns) > MaxConns) {
                Interlocked.Decrement(ref conns);
                Log("refused (too-many-connections)");
                try { c.Close(); } catch { }
                continue;
            }
            var t = new Thread(Handle); t.IsBackground = true; t.Start(c);
        }
    }

    static void Handle(object o) {
        var c = (TcpClient)o;
        string asked = "?", host;
        try {
            c.NoDelay = true;
            NetworkStream cs = c.GetStream();
            cs.ReadTimeout = HeadMs;
            string head = ReadHead(cs);
            string line = head == null ? null : head.Split(new[] { "\r\n" }, StringSplitOptions.None)[0];
            if (line != null) { string[] p = line.Split(' '); if (p.Length > 1) asked = Clean(p[1]); }
            string why = Judge(line, out host);
            IPAddress to = null;
            if (why == null) {
                foreach (IPAddress a in Dns.GetHostAddresses(host)) { if (!Public(a)) { why = "not-public-address"; break; } if (to == null) to = a; }
                if (why == null && to == null) why = "no-address";
            }
            if (why != null) {
                Send(cs, why == "not-connect" ? "405 Method Not Allowed" : "403 Forbidden");
                Log("refused " + asked + " (" + why + ")");
                return;
            }
            var s = new TcpClient(to.AddressFamily);
            s.Connect(to, 443);
            s.NoDelay = true;
            NetworkStream ss = s.GetStream();
            Send(cs, "200 Connection Established");
            cs.ReadTimeout = IdleMs; ss.ReadTimeout = IdleMs;
            long up = 0, down = 0;
            var t = new Thread(() => { up = Pump(cs, ss); try { s.Client.Shutdown(SocketShutdown.Send); } catch { } });
            t.IsBackground = true; t.Start();
            down = Pump(ss, cs);
            try { c.Client.Shutdown(SocketShutdown.Send); } catch { }
            t.Join(IdleMs);
            s.Close();
            Log("allowed " + asked + " up=" + up + " down=" + down);
        } catch (Exception e) {
            Log("error " + asked + " (" + e.GetType().Name + ")");
        } finally {
            try { c.Close(); } catch { }
            Interlocked.Decrement(ref conns);
        }
    }

    // The request head up to the blank line, at most MaxHead bytes; null if
    // it never ends. Read a byte at a time so nothing after it is consumed:
    // a client that sends its TLS hello early still gets it forwarded.
    static string ReadHead(NetworkStream s) {
        var buf = new byte[MaxHead]; int n = 0;
        while (n < MaxHead) {
            int r = s.Read(buf, n, 1);
            if (r <= 0) return null;
            n++;
            if (n >= 4 && buf[n - 4] == 13 && buf[n - 3] == 10 && buf[n - 2] == 13 && buf[n - 1] == 10)
                return Encoding.ASCII.GetString(buf, 0, n);
        }
        return null;
    }

    static long Pump(Stream from, Stream to) {
        var buf = new byte[65536]; long total = 0;
        try { int r; while ((r = from.Read(buf, 0, buf.Length)) > 0) { to.Write(buf, 0, r); total += r; } } catch { }
        return total;
    }

    static void Send(Stream s, string status) {
        byte[] b = Encoding.ASCII.GetBytes("HTTP/1.1 " + status + "\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        s.Write(b, 0, b.Length);
    }

    // What the client asked for, safe to log: printable ASCII, <= 120 chars.
    static string Clean(string x) {
        var sb = new StringBuilder();
        foreach (char ch in x) { if (sb.Length >= 120) break; sb.Append(ch >= 33 && ch < 127 ? ch : '?'); }
        return sb.ToString();
    }

    static void Log(string what) {
        string line = DateTime.UtcNow.ToString("yyyy-MM-ddTHH:mm:ssZ") + " " + what;
        lock (logLock) {
            if (Echo) Console.WriteLine(line);
            try { File.AppendAllText(logPath, line + "\r\n"); } catch { }
        }
    }
}
'@

$logDir = Join-Path $env:LOCALAPPDATA 'Glim'
if (-not (Test-Path -LiteralPath $logDir)) { New-Item -ItemType Directory -Path $logDir | Out-Null }
[GlimProxy]::Echo = [bool]$Echo
[GlimProxy]::Run($Port, (Join-Path $logDir 'agent-proxy.log'))
