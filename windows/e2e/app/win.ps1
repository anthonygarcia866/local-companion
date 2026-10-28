# Win32 helper for the real-app check (automated testing stage 3, e2e/app/run.mjs).
# CI's runner only: it presses real keys and captures the real screen, so it is
# never run on the owner's desktop.
#
# One process, one JSON command per stdin line, one JSON answer per stdout line:
#   {"cmd":"windows","pid":123}         every top-level window of that process
#   {"cmd":"keys","chord":"ctrl+alt+shift+y"}   SendInput, down then up
#   {"cmd":"shot","x":0,"y":0,"w":10,"h":10,"out":"a.png"}  a desktop capture (BitBlt)
#   {"cmd":"quit"}
$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;

public static class GlimWin {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool GetWindowDisplayAffinity(IntPtr h, out uint a);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();

  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
  // INPUT on x64: type, padding, then the union (MOUSEINPUT is the largest, 32 bytes).
  [StructLayout(LayoutKind.Explicit, Size = 40)] public struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT ki; }

  static string Esc(string s) { return s.Replace("\\", "\\\\").Replace("\"", "\\\""); }

  public static string Windows(uint pid) {
    var parts = new List<string>();
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid) return true;
      var t = new StringBuilder(256); GetWindowText(h, t, 256);
      var c = new StringBuilder(256); GetClassName(h, c, 256);
      uint a; if (!GetWindowDisplayAffinity(h, out a)) a = 0xFFFFFFFF;
      RECT r; GetWindowRect(h, out r);
      parts.Add(string.Format("{{\"hwnd\":{0},\"title\":\"{1}\",\"cls\":\"{2}\",\"visible\":{3},\"affinity\":{4},\"x\":{5},\"y\":{6},\"w\":{7},\"h\":{8}}}",
        h.ToInt64(), Esc(t.ToString()), Esc(c.ToString()), IsWindowVisible(h) ? "true" : "false", a, r.L, r.T, r.R - r.L, r.B - r.T));
      return true;
    }, IntPtr.Zero);
    return "[" + string.Join(",", parts) + "]";
  }

  static ushort Vk(string k) {
    switch (k) {
      case "ctrl": return 0x11; case "alt": return 0x12; case "shift": return 0x10;
      default: return (ushort)char.ToUpperInvariant(k[0]);
    }
  }

  /// The chord's keys down in order, then up in reverse: how many events Windows took (0 = no input desktop).
  public static uint Keys(string chord) {
    var keys = chord.Split('+');
    var list = new List<INPUT>();
    foreach (var k in keys) list.Add(new INPUT { type = 1, ki = new KEYBDINPUT { wVk = Vk(k) } });
    for (int i = keys.Length - 1; i >= 0; i--) list.Add(new INPUT { type = 1, ki = new KEYBDINPUT { wVk = Vk(keys[i]), dwFlags = 2 } });
    return SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
  }

  /// A desktop capture of the rectangle (what a screenshot tool or a screen
  /// share sees): saved, plus how many pixels are Glim's amber/orange.
  public static string Shot(int x, int y, int w, int h, string path) {
    using (var bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
      using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(x, y, 0, 0, new Size(w, h));
      bmp.Save(path, ImageFormat.Png);
      long warm = 0, n = 0; var colours = new HashSet<int>();
      for (int yy = 0; yy < h; yy += 2) for (int xx = 0; xx < w; xx += 2) {
        var c = bmp.GetPixel(xx, yy); n++; colours.Add(c.ToArgb() & 0xF0F0F0);
        if (c.R > 170 && c.G > 90 && c.B < 120 && c.R - c.B > 90) warm++;
      }
      return string.Format("{{\"warm\":{0},\"sampled\":{1},\"colours\":{2}}}", warm, n, colours.Count);
    }
  }

  public static long Foreground() { return GetForegroundWindow().ToInt64(); }
}
"@
[void][GlimWin]::SetProcessDPIAware()
[Console]::Out.WriteLine('{"ready":true}')
[Console]::Out.Flush()
while ($null -ne ($line = [Console]::In.ReadLine())) {
  try {
    $c = $line | ConvertFrom-Json
    switch ($c.cmd) {
      'windows' { $out = [GlimWin]::Windows([uint32]$c.pid) }
      'keys' { $out = '{"sent":' + [GlimWin]::Keys([string]$c.chord) + ',"foreground":' + [GlimWin]::Foreground() + '}' }
      'shot' { $out = [GlimWin]::Shot([int]$c.x, [int]$c.y, [int]$c.w, [int]$c.h, [string]$c.out) }
      'quit' { exit 0 }
      default { $out = '{"error":"unknown command"}' }
    }
  } catch {
    $out = '{"error":' + ($_.Exception.Message | ConvertTo-Json -Compress) + '}'
  }
  [Console]::Out.WriteLine($out)
  [Console]::Out.Flush()
}
