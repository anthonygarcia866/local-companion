# Effective access of an account to a file or folder, computed on the OWNER's
# side with Windows' AuthZ API from that account's SID and the object's own
# security descriptor. Nothing runs as that account, so it can't influence
# the answer (review 2026-11-22: a probe running as glim-agent could be made
# to lie by glim-agent's own profile). Dot-source it.
#
# The context gets the account's local groups (AuthZ computes them) plus the
# SIDs every interactive logon carries: Everyone, Users, INTERACTIVE, CONSOLE
# LOGON, LOCAL, Authenticated Users, This Organization, Local account. Only
# the object's own DACL is evaluated, which is exactly what opening it by its
# full path needs (everyone has "bypass traverse checking").

if (-not ('GlimFs' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Security.Principal;
public static class GlimAuthz {
    [StructLayout(LayoutKind.Sequential)] struct LUID { public uint Low; public int High; }
    [StructLayout(LayoutKind.Sequential)] struct SID_AND_ATTRIBUTES { public IntPtr Sid; public uint Attributes; }
    [StructLayout(LayoutKind.Sequential)] struct REQUEST { public uint DesiredAccess; public IntPtr PrincipalSelfSid; public IntPtr ObjectTypeList; public uint ObjectTypeListLength; public IntPtr OptionalArguments; }
    [StructLayout(LayoutKind.Sequential)] struct REPLY { public uint ResultListLength; public IntPtr GrantedAccessMask; public IntPtr SaclEvaluationResults; public IntPtr Error; }
    [DllImport("authz.dll", SetLastError = true)] static extern bool AuthzInitializeResourceManager(uint flags, IntPtr a, IntPtr b, IntPtr c, [MarshalAs(UnmanagedType.LPWStr)] string name, out IntPtr rm);
    [DllImport("authz.dll", SetLastError = true)] static extern bool AuthzInitializeContextFromSid(uint flags, byte[] sid, IntPtr rm, IntPtr exp, LUID id, IntPtr dyn, out IntPtr ctx);
    [DllImport("authz.dll", SetLastError = true)] static extern bool AuthzAddSidsToContext(IntPtr ctx, SID_AND_ATTRIBUTES[] sids, uint n, IntPtr r, uint rn, out IntPtr newCtx);
    [DllImport("authz.dll", SetLastError = true)] static extern bool AuthzAccessCheck(uint flags, IntPtr ctx, ref REQUEST req, IntPtr audit, byte[] sd, IntPtr opt, uint optN, ref REPLY reply, IntPtr results);
    [DllImport("authz.dll")] static extern bool AuthzFreeContext(IntPtr ctx);
    [StructLayout(LayoutKind.Sequential)] struct LSA_OBJECT_ATTRIBUTES { public int Length; public IntPtr RootDirectory; public IntPtr ObjectName; public uint Attributes; public IntPtr SecurityDescriptor; public IntPtr SecurityQualityOfService; }
    [StructLayout(LayoutKind.Sequential)] struct LSA_UNICODE_STRING { public ushort Length; public ushort MaximumLength; public IntPtr Buffer; }
    [DllImport("advapi32.dll")] static extern uint LsaOpenPolicy(IntPtr system, ref LSA_OBJECT_ATTRIBUTES attrs, uint access, out IntPtr policy);
    [DllImport("advapi32.dll")] static extern uint LsaEnumerateAccountRights(IntPtr policy, byte[] sid, out IntPtr rights, out uint count);
    [DllImport("advapi32.dll")] static extern uint LsaFreeMemory(IntPtr p);
    [DllImport("advapi32.dll")] static extern uint LsaClose(IntPtr p);
    [DllImport("advapi32.dll")] static extern int LsaNtStatusToWinError(uint status);

    // The user rights (privileges and logon rights) assigned directly to `sid`.
    public static string[] Rights(string sid) {
        var oa = new LSA_OBJECT_ATTRIBUTES { Length = Marshal.SizeOf(typeof(LSA_OBJECT_ATTRIBUTES)) };
        IntPtr pol;
        uint st = LsaOpenPolicy(IntPtr.Zero, ref oa, 0x800 /* POLICY_LOOKUP_NAMES */, out pol);
        if (st != 0) throw new System.ComponentModel.Win32Exception(LsaNtStatusToWinError(st));
        try {
            var s = new SecurityIdentifier(sid); var b = new byte[s.BinaryLength]; s.GetBinaryForm(b, 0);
            IntPtr arr; uint n;
            st = LsaEnumerateAccountRights(pol, b, out arr, out n);
            if (st == 0xC0000034) return new string[0];  // none assigned
            if (st != 0) throw new System.ComponentModel.Win32Exception(LsaNtStatusToWinError(st));
            try {
                var outv = new string[n]; int sz = Marshal.SizeOf(typeof(LSA_UNICODE_STRING));
                for (int i = 0; i < n; i++) {
                    var u = (LSA_UNICODE_STRING)Marshal.PtrToStructure(new IntPtr(arr.ToInt64() + i * sz), typeof(LSA_UNICODE_STRING));
                    outv[i] = Marshal.PtrToStringUni(u.Buffer, u.Length / 2);
                }
                return outv;
            } finally { LsaFreeMemory(arr); }
        } finally { LsaClose(pol); }
    }
    public static string[] LogonSids() { return (string[])Logon.Clone(); }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern uint GetFinalPathNameByHandleW(IntPtr h, System.Text.StringBuilder buf, uint len, uint flags);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);

    // The real path `path` leads to, every junction and symlink on the way
    // followed (what opening it really reaches); null if it can't be opened.
    public static string FinalPath(string path) {
        IntPtr h = CreateFileW(GlimFs.Raw(path), 0x80 /* FILE_READ_ATTRIBUTES */, 7 /* share all */, IntPtr.Zero, 3 /* OPEN_EXISTING */, 0x02000000 /* BACKUP_SEMANTICS: folders too */, IntPtr.Zero);
        if (h == new IntPtr(-1)) return null;
        try {
            var sb = new System.Text.StringBuilder(32768);
            uint n = GetFinalPathNameByHandleW(h, sb, (uint)sb.Capacity, 0);
            if (n == 0 || n >= sb.Capacity) return null;
            string s = sb.ToString();
            if (s.StartsWith(@"\\?\UNC\")) return @"\\" + s.Substring(8);
            if (s.StartsWith(@"\\?\")) return s.Substring(4);
            return s;
        } finally { CloseHandle(h); }
    }
    [DllImport("authz.dll")] static extern bool AuthzFreeResourceManager(IntPtr rm);
    // Every SID any of its logons may carry (for the "who could it be" checks:
    // user rights and group membership, where more is the stricter side).
    static readonly string[] Logon = { "S-1-1-0", "S-1-5-32-545", "S-1-5-4", "S-1-2-1", "S-1-2-0", "S-1-5-11", "S-1-5-15", "S-1-5-113", "S-1-5-64-10", "S-1-5-3" };
    // Access checks use the SIDs each real logon carries, one variant at a
    // time (a superset is NOT stricter there: a deny entry for a SID the real
    // token lacks would hide access it has). An object counts as granted
    // whatever ANY variant is granted.
    static readonly string[] Base = { "S-1-1-0", "S-1-5-32-545", "S-1-2-0", "S-1-5-11", "S-1-5-15", "S-1-5-113", "S-1-5-64-10" };
    static readonly string[][] Variants = {
        new[] { "S-1-5-4", "S-1-2-1" },  // interactive, console session
        new[] { "S-1-5-4" },             // interactive, no CONSOLE LOGON
        new[] { "S-1-5-3" },             // batch
    };

    // The access mask `sid` gets on an object with security descriptor `sd` (MAXIMUM_ALLOWED).
    public static uint Granted(string sid, byte[] sd) {
        using (var c = new Checker(sid)) return c.Granted(sd);
    }

    // One resource manager and one context per logon variant, made once and
    // reused for every object (a profile walk checks ~1M of them).
    public sealed class Checker : IDisposable {
        IntPtr rm; IntPtr[] ctxs;
        public Checker(string sid) {
            if (!AuthzInitializeResourceManager(1, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, "glim-check", out rm)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            ctxs = new IntPtr[Variants.Length];
            var s = new SecurityIdentifier(sid); var b = new byte[s.BinaryLength]; s.GetBinaryForm(b, 0);
            for (int v = 0; v < Variants.Length; v++) {
                var logon = new string[Base.Length + Variants[v].Length];
                Base.CopyTo(logon, 0); Variants[v].CopyTo(logon, Base.Length);
                IntPtr ctx;
                if (!AuthzInitializeContextFromSid(0, b, rm, IntPtr.Zero, new LUID(), IntPtr.Zero, out ctx)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                var pins = new GCHandle[logon.Length];
                var add = new SID_AND_ATTRIBUTES[logon.Length];
                try {
                    for (int i = 0; i < logon.Length; i++) {
                        var g = new SecurityIdentifier(logon[i]); var gb = new byte[g.BinaryLength]; g.GetBinaryForm(gb, 0);
                        pins[i] = GCHandle.Alloc(gb, GCHandleType.Pinned);
                        add[i] = new SID_AND_ATTRIBUTES { Sid = pins[i].AddrOfPinnedObject(), Attributes = 7 };
                    }
                    if (!AuthzAddSidsToContext(ctx, add, (uint)add.Length, IntPtr.Zero, 0, out ctxs[v])) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                } finally { foreach (var pn in pins) if (pn.IsAllocated) pn.Free(); AuthzFreeContext(ctx); }
            }
        }
        public uint Granted(byte[] sd) {
            uint any = 0;
            IntPtr mask = Marshal.AllocHGlobal(4), err = Marshal.AllocHGlobal(4);
            try {
                foreach (var c in ctxs) {
                    var req = new REQUEST { DesiredAccess = 0x02000000 };
                    var rep = new REPLY { ResultListLength = 1, GrantedAccessMask = mask, Error = err };
                    if (!AuthzAccessCheck(0, c, ref req, IntPtr.Zero, sd, IntPtr.Zero, 0, ref rep, IntPtr.Zero)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                    any |= (uint)Marshal.ReadInt32(mask);
                }
            } finally { Marshal.FreeHGlobal(mask); Marshal.FreeHGlobal(err); }
            return any;
        }
        public void Dispose() {
            if (ctxs != null) foreach (var c in ctxs) if (c != IntPtr.Zero) AuthzFreeContext(c);
            ctxs = null;
            if (rm != IntPtr.Zero) AuthzFreeResourceManager(rm);
            rm = IntPtr.Zero;
        }
    }

}
public static class GlimFs {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr tmpl);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetFileInformationByHandleEx(IntPtr h, int cls, IntPtr buf, uint size);
    [DllImport("advapi32.dll")]
    static extern uint GetSecurityInfo(IntPtr h, int type, uint info, out IntPtr owner, out IntPtr group, out IntPtr dacl, out IntPtr sacl, out IntPtr sd);
    [DllImport("advapi32.dll")] static extern uint GetSecurityDescriptorLength(IntPtr sd);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr p, uint access, out IntPtr tok);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool LookupPrivilegeValueW(string sys, string name, out long luid);
    [StructLayout(LayoutKind.Sequential, Pack = 4)] struct TP { public uint Count; public long Luid; public uint Attr; }
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool AdjustTokenPrivileges(IntPtr tok, bool disableAll, ref TP tp, uint len, IntPtr prev, IntPtr retLen);

    const uint READ_CONTROL = 0x20000, LIST = 0x1, SYNC = 0x100000;

    // The path in \\?\ form, so Windows takes it literally: no length limit,
    // and a trailing space or dot isn't trimmed (review 8: "C:\x\.claude2 "
    // was silently read as ".claude2").
    public static string Raw(string p) {
        if (p.Length == 2 && p[1] == ':') p += @"\";   // "C:" alone would open the volume
        if (p.StartsWith(@"\\?\")) return p;
        if (p.StartsWith(@"\\")) return @"\\?\UNC\" + p.Substring(2);
        return @"\\?\" + p;
    }
    const uint BACKUP = 0x02000000, REPARSE = 0x00200000;
    static readonly IntPtr BAD = new IntPtr(-1);

    // Turns on SeBackupPrivilege in this process (an elevated run holds it,
    // disabled). With it, the reads below get past a DACL that leaves the
    // owner out (READ_CONTROL and listing only; nothing is opened for data).
    public static bool EnableBackup() { return EnablePrivilege("SeBackupPrivilege"); }

    public static bool EnablePrivilege(string name) {
        IntPtr tok;
        if (!OpenProcessToken(GetCurrentProcess(), 0x28, out tok)) return false;
        try {
            long luid; if (!LookupPrivilegeValueW(null, name, out luid)) return false;
            var tp = new TP { Count = 1, Luid = luid, Attr = 2 };
            if (!AdjustTokenPrivileges(tok, false, ref tp, 0, IntPtr.Zero, IntPtr.Zero)) return false;
            return Marshal.GetLastWin32Error() == 0;   // 1300 = not held
        } finally { CloseHandle(tok); }
    }

    // The object's owner, group and DACL, read from the object itself (a
    // link: the link's own, never its target's). null + error code on failure
    // (32 = in use with no sharing: nobody, glim-agent included, can open it).
    public static byte[] ReadSd(string path, out int error) {
        error = 0;
        IntPtr h = CreateFileW(Raw(path), READ_CONTROL, 7, IntPtr.Zero, 3, BACKUP | REPARSE, IntPtr.Zero);
        if (h == BAD) { error = Marshal.GetLastWin32Error(); return null; }
        try {
            IntPtr o, g, d, sa, sd;
            uint rc = GetSecurityInfo(h, 1, 7, out o, out g, out d, out sa, out sd);
            if (rc != 0) { error = (int)rc; return null; }
            try {
                int len = (int)GetSecurityDescriptorLength(sd);
                var b = new byte[len]; Marshal.Copy(sd, b, 0, len); return b;
            } finally { LocalFree(sd); }
        } finally { CloseHandle(h); }
    }

    public struct Entry { public string Path; public bool Dir; public bool Link; }

    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool SetKernelObjectSecurity(IntPtr h, uint info, byte[] sd);
    // Pack = 4: its FILETIMEs are two DWORDs, 4-aligned (review 2026-11-26
    // HIGH-B: as 8-aligned longs, "Links" read nFileIndexHigh).
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    struct BHFI { public uint Attr; public long C, A, W; public uint Vol, SizeHi, SizeLo, Links, IdxHi, IdxLo; }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetFileInformationByHandle(IntPtr h, out BHFI info);
    [DllImport("advapi32.dll", SetLastError = true, EntryPoint = "AdjustTokenPrivileges")]
    static extern bool AdjustPrev(IntPtr tok, bool disableAll, ref TP tp, uint len, ref TP prev, out uint retLen);

    // Rewrite's refusals (negative, so never a Windows error code).
    public const int HardLink = -2, NullDacl = -3;
    const AceFlags Inh = AceFlags.ObjectInherit | AceFlags.ContainerInherit;
    const AceFlags Odd = AceFlags.InheritOnly | AceFlags.NoPropagateInherit;

    // Does `path` carry an explicit (not inherited) deny for `sid` covering
    // all of `mask` that applies to the object itself (not inherit-only) and,
    // for a tree, is inherited by files and folders all the way down (no
    // no-propagate)? Read from the object itself (setup reads back each lock).
    public static bool HasDeny(string path, string sid, uint mask, bool tree) {
        int e; byte[] sd = ReadSd(path, out e);
        if (sd == null) return false;
        var r = new RawSecurityDescriptor(sd, 0);
        if (r.DiscretionaryAcl == null) return false;
        foreach (GenericAce a in r.DiscretionaryAcl) {
            var k = a as KnownAce;
            if (k == null || a.AceType != AceType.AccessDenied) continue;
            if ((a.AceFlags & AceFlags.Inherited) != 0 || (a.AceFlags & Odd) != 0) continue;
            if (k.SecurityIdentifier.Value != sid) continue;
            if (((uint)k.AccessMask & mask) != mask) continue;
            if (tree && (a.AceFlags & Inh) != Inh) continue;
            return true;
        }
        return false;
    }

    // Puts an explicit deny for `sid` (`mask`; inheritable if `inherit`) first
    // in the object's own DACL. Review 2026-11-26 (H1, M1, M2): written with
    // SetKernelObjectSecurity on a handle to the object itself (backup
    // semantics; a link's own entry, never its target's), so Windows
    // recomputes nothing: no inherited entries re-stamped from a parent,
    // nothing passed down to children, the DACL's protected / auto-inherited
    // flags exactly as they were, owner unchanged. Only items created later
    // inherit it; callers give each existing item that needs one its own.
    // With SeRestorePrivilege (on only for the write) an elevated admin can
    // do this on folders SYSTEM or TrustedInstaller own.
    // Refused, never changed: a file with more than one hard link (its other
    // names may be anywhere: HardLink) and a null DACL (everyone allowed; a
    // deny alone would leave nobody else any access: NullDacl).
    // An existing explicit deny for `sid` is merged into the new one (masks
    // and inheritance OR'd, inherit-only / no-propagate dropped), never
    // narrowed. 0 on success, else a Windows error code or a refusal.
    public static int AddDeny(string path, string sid, uint mask, bool inherit) {
        return Rewrite(path, sid, mask, inherit, true);
    }

    // Removes every explicit deny for `sid` from the object's own DACL (same
    // way: nothing re-inherited or passed down).
    public static int RemoveDeny(string path, string sid) {
        return Rewrite(path, sid, 0, false, false);
    }

    // Turns a privilege on or off; returns whether it was on before.
    static bool SetPriv(string name, bool on) {
        IntPtr tok;
        if (!OpenProcessToken(GetCurrentProcess(), 0x28, out tok)) return false;
        try {
            long luid; if (!LookupPrivilegeValueW(null, name, out luid)) return false;
            var tp = new TP { Count = 1, Luid = luid, Attr = on ? 2u : 0u };
            var prev = new TP(); uint len;
            if (!AdjustPrev(tok, false, ref tp, (uint)Marshal.SizeOf(typeof(TP)), ref prev, out len)) return false;
            return prev.Count == 1 ? (prev.Attr & 2) != 0 : on;   // Count 0: it was already as asked
        } finally { CloseHandle(tok); }
    }

    static int Rewrite(string path, string sid, uint mask, bool inherit, bool add) {
        bool hadRestore = SetPriv("SeRestorePrivilege", true);
        try {
            IntPtr h = CreateFileW(Raw(path), READ_CONTROL | 0x40000 /* WRITE_DAC */, 7, IntPtr.Zero, 3, BACKUP | REPARSE, IntPtr.Zero);
            if (h == BAD) return Marshal.GetLastWin32Error();
            try {
                BHFI fi;
                if (!GetFileInformationByHandle(h, out fi)) return Marshal.GetLastWin32Error();
                if ((fi.Attr & 0x10) == 0 && fi.Links > 1) return HardLink;
                IntPtr o, g, d, sa, psd;
                uint rc = GetSecurityInfo(h, 1, 4 /* DACL */, out o, out g, out d, out sa, out psd);
                if (rc != 0) return (int)rc;
                byte[] sd;
                try { int len = (int)GetSecurityDescriptorLength(psd); sd = new byte[len]; Marshal.Copy(psd, sd, 0, len); } finally { LocalFree(psd); }
                var r = new RawSecurityDescriptor(sd, 0);
                if (r.DiscretionaryAcl == null) return add ? NullDacl : 0;   // null or absent: everyone allowed
                var acl = r.DiscretionaryAcl ?? new RawAcl(GenericAcl.AclRevision, 1);
                var me = new SecurityIdentifier(sid);
                int merged = (int)mask; AceFlags mflags = inherit ? Inh : AceFlags.None;
                bool found = false;
                for (int i = acl.Count - 1; i >= 0; i--) {
                    var k = acl[i] as KnownAce;
                    if (k != null && acl[i].AceType == AceType.AccessDenied && (acl[i].AceFlags & AceFlags.Inherited) == 0 && k.SecurityIdentifier == me) {
                        merged |= k.AccessMask; mflags |= acl[i].AceFlags & Inh;
                        acl.RemoveAce(i); found = true;
                    }
                }
                if (!add && !found) return 0;
                if (add) acl.InsertAce(0, new CommonAce(mflags, AceQualifier.AccessDenied, merged, me, false, null));
                // Only the DACL and its own flags: protected / auto-inherited kept as read.
                var keep = r.ControlFlags & (ControlFlags.DiscretionaryAclProtected | ControlFlags.DiscretionaryAclAutoInherited | ControlFlags.DiscretionaryAclAutoInheritRequired);
                // Without the request bit Windows drops "auto-inherited" (tested 2026-11-26); with it the
                // kernel keeps the object's own inherited entries, never recomputed from a parent.
                if ((keep & ControlFlags.DiscretionaryAclAutoInherited) != 0) keep |= ControlFlags.DiscretionaryAclAutoInheritRequired;
                var nsd = new RawSecurityDescriptor(keep | ControlFlags.DiscretionaryAclPresent | ControlFlags.SelfRelative, null, null, null, acl);
                var bin = new byte[nsd.BinaryLength]; nsd.GetBinaryForm(bin, 0);
                if (!SetKernelObjectSecurity(h, 4 /* DACL */, bin)) return Marshal.GetLastWin32Error();
                return 0;
            } finally { CloseHandle(h); }
        } finally { if (!hadRestore) SetPriv("SeRestorePrivilege", false); }
    }

    // A folder whose existing items may later gain children: does it carry a
    // deny for `sid` covering `mask` that new files and folders inside will
    // inherit (explicit, or an inherited copy icacls passed down; object and
    // container inherit, no inherit-only on the folder itself, no
    // no-propagate)? Review 2026-11-26 M-a: without one, a file a service
    // creates there later could be writable by glim-agent.
    public static bool HasInheritableDeny(string path, string sid, uint mask) {
        int e; byte[] sd = ReadSd(path, out e);
        return sd != null && InhDeny(sd, sid, mask);
    }
    static bool InhDeny(byte[] sd, string sid, uint mask) {
        var r = new RawSecurityDescriptor(sd, 0);
        if (r.DiscretionaryAcl == null) return false;
        foreach (GenericAce a in r.DiscretionaryAcl) {
            var k = a as KnownAce;
            if (k == null || a.AceType != AceType.AccessDenied) continue;
            if ((a.AceFlags & Odd) != 0 || (a.AceFlags & Inh) != Inh) continue;
            if (k.SecurityIdentifier.Value != sid) continue;
            if (((uint)k.AccessMask & mask) != mask) continue;
            return true;
        }
        return false;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct SA { public int Len; public IntPtr Sd; public bool Inherit; }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateDirectoryW(string name, ref SA sa);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(string sddl, uint rev, out IntPtr sd, out uint len);

    // Creates a NEW folder with exactly this security descriptor (SDDL, a
    // DACL only), in one step: no moment with other permissions, nothing
    // re-stamped. Fails (183) if it already exists. 0 on success.
    public static int CreateDirSddl(string path, string sddl) {
        IntPtr sd; uint len;
        if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, 1, out sd, out len)) return Marshal.GetLastWin32Error();
        try {
            var sa = new SA { Len = Marshal.SizeOf(typeof(SA)), Sd = sd, Inherit = false };
            if (!CreateDirectoryW(Raw(path), ref sa)) return Marshal.GetLastWin32Error();
            return 0;
        } finally { LocalFree(sd); }
    }

    // Replaces this one object's DACL with `sddl`'s (protected / auto-
    // inherited flags as the SDDL says), through the same non-propagating
    // kernel write as AddDeny. Only for a folder setup just emptied. 0 = ok.
    public static int SetDaclSddl(string path, string sddl) {
        var want = new RawSecurityDescriptor(sddl);
        IntPtr h = CreateFileW(Raw(path), READ_CONTROL | 0x40000 /* WRITE_DAC */, 7, IntPtr.Zero, 3, BACKUP | REPARSE, IntPtr.Zero);
        if (h == BAD) return Marshal.GetLastWin32Error();
        try {
            var keep = want.ControlFlags & (ControlFlags.DiscretionaryAclProtected | ControlFlags.DiscretionaryAclAutoInherited);
            var nsd = new RawSecurityDescriptor(keep | ControlFlags.DiscretionaryAclPresent | ControlFlags.SelfRelative, null, null, null, want.DiscretionaryAcl);
            var bin = new byte[nsd.BinaryLength]; nsd.GetBinaryForm(bin, 0);
            if (!SetKernelObjectSecurity(h, 4, bin)) return Marshal.GetLastWin32Error();
            return 0;
        } finally { CloseHandle(h); }
    }

    // Is `path` a real folder (not a link) whose owner is one of `owners` and
    // whose DACL is protected and holds exactly `sddl`'s entries (type,
    // flags, mask, SID; order ignored)? "" when it is, else what differs.
    public static string SameDir(string path, string sddl, string[] owners) { return SameItem(path, sddl, owners, true); }
    // The same for a real file (setup's sentinel in the work folder).
    public static string SameFile(string path, string sddl, string[] owners) { return SameItem(path, sddl, owners, false); }

    // A real folder (attributes read literally: \\?\, no MAX_PATH limit, no
    // trailing dot or space trimmed; review 3 L-7), never a link?
    public static bool IsRealDir(string path) {
        uint attr = GetFileAttributesW(Raw(path));
        return attr != 0xFFFFFFFF && (attr & 0x10) != 0 && (attr & 0x400) == 0;
    }

    // Can `path` be opened with no sharing at all (the link itself)? Only
    // when no other handle to it is open anywhere (review 3 H-1: one held
    // from before setup could still turn the folder into a junction).
    public static bool NoOtherHandles(string path) {
        // Data access (read, write, delete) asked for: Windows' sharing check
        // ignores READ_CONTROL alone, so only this conflicts with an open handle
        // (tested 2026-11-26).
        IntPtr h = CreateFileW(Raw(path), READ_CONTROL | 0x1 | 0x2 | 0x10000, 0, IntPtr.Zero, 3, BACKUP | REPARSE, IntPtr.Zero);
        if (h == BAD) return false;
        CloseHandle(h);
        return true;
    }

    static string SameItem(string path, string sddl, string[] owners, bool dir) {
        int e; byte[] sd = ReadSd(path, out e);
        if (sd == null) return "can't read it (error " + e + ")";
        if (IsLink(path)) return "it is a link";
        uint attr = GetFileAttributesW(Raw(path));
        if (attr == 0xFFFFFFFF || ((attr & 0x10) != 0) != dir) return dir ? "not a folder" : "not a file";
        var r = new RawSecurityDescriptor(sd, 0);
        if (r.Owner == null || Array.IndexOf(owners, r.Owner.Value) < 0) return "owner is " + (r.Owner == null ? "nobody" : r.Owner.Value);
        if ((r.ControlFlags & ControlFlags.DiscretionaryAclProtected) == 0) return "it inherits from its parent";
        var want = new RawSecurityDescriptor(sddl).DiscretionaryAcl;
        var have = r.DiscretionaryAcl;
        if (have == null) return "no DACL";
        var a = new List<string>(); var b = new List<string>();
        foreach (GenericAce x in have) { var k = x as KnownAce; a.Add(k == null ? "?" : ((int)x.AceType + "/" + (int)(x.AceFlags & ~AceFlags.Inherited) + "/" + k.AccessMask.ToString("X") + "/" + k.SecurityIdentifier.Value)); }
        foreach (GenericAce x in want) { var k = (KnownAce)x; b.Add((int)x.AceType + "/" + (int)x.AceFlags + "/" + k.AccessMask.ToString("X") + "/" + k.SecurityIdentifier.Value); }
        a.Sort(StringComparer.Ordinal); b.Sort(StringComparer.Ordinal);
        if (string.Join(";", a.ToArray()) != string.Join(";", b.ToArray())) return "its entries differ";
        return "";
    }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern uint GetFileAttributesW(string name);

    // Does `path` exist, taken literally (\\?\, the last link not followed)?
    // 0 only when Windows says file or path not found (errors 2, 3); 1 when
    // it opens; -1 for anything else (access denied ...): unknown, which
    // callers treat as "exists, not verified" (fail closed).
    public static int Exists(string path) {
        IntPtr h = CreateFileW(Raw(path), 0, 7, IntPtr.Zero, 3, BACKUP | REPARSE, IntPtr.Zero);
        if (h != BAD) { CloseHandle(h); return 1; }
        int e = Marshal.GetLastWin32Error();
        return (e == 2 || e == 3) ? 0 : -1;
    }

    [StructLayout(LayoutKind.Sequential)] struct ATTR_TAG { public uint Attr; public uint Tag; }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool DeviceIoControl(IntPtr h, uint code, IntPtr inBuf, uint inLen, IntPtr outBuf, uint outLen, out uint ret, IntPtr ov);

    // Where a junction / mount point / symlink points, read from the link
    // itself (works when the target is gone). null if it isn't one or can't
    // be read. A relative symlink's target comes back as stored (relative).
    public static string LinkTarget(string path) {
        IntPtr h = CreateFileW(Raw(path), 0, 7, IntPtr.Zero, 3, BACKUP | REPARSE, IntPtr.Zero);
        if (h == BAD) return null;
        IntPtr buf = Marshal.AllocHGlobal(16384);
        try {
            uint got;
            if (!DeviceIoControl(h, 0x000900A8 /* FSCTL_GET_REPARSE_POINT */, IntPtr.Zero, 0, buf, 16384, out got, IntPtr.Zero)) return null;
            uint tag = (uint)Marshal.ReadInt32(buf, 0);
            int pathStart;
            if (tag == 0xA0000003) pathStart = 16;        // junction / mount point
            else if (tag == 0xA000000C) pathStart = 20;   // symlink (has a 4-byte Flags field first)
            else return null;
            int subOff = Marshal.ReadInt16(buf, 8), subLen = Marshal.ReadInt16(buf, 10);
            int prOff = Marshal.ReadInt16(buf, 12), prLen = Marshal.ReadInt16(buf, 14);
            string sub = Marshal.PtrToStringUni(new IntPtr(buf.ToInt64() + pathStart + subOff), subLen / 2);
            if (sub.StartsWith(@"\??\UNC\")) return @"\\" + sub.Substring(8);
            if (sub.StartsWith(@"\??\")) return sub.Substring(4);
            // Only the stored target counts, never the display name (review:
            // a relative link could carry a misleading absolute PrintName).
            return sub;
        } finally { Marshal.FreeHGlobal(buf); CloseHandle(h); }
    }

    // True only for a name-surrogate reparse point (junction, mount point,
    // symlink): something that sends whoever opens it elsewhere. A cloud
    // placeholder (OneDrive) or an app alias is not a link.
    public static bool IsLink(string path) {
        IntPtr h = CreateFileW(Raw(path), 0x80, 7, IntPtr.Zero, 3, BACKUP | REPARSE, IntPtr.Zero);
        if (h == BAD) return false;
        IntPtr buf = Marshal.AllocHGlobal(8);
        try {
            if (!GetFileInformationByHandleEx(h, 9 /* FileAttributeTagInfo */, buf, 8)) return false;
            var t = (ATTR_TAG)Marshal.PtrToStructure(buf, typeof(ATTR_TAG));
            return (t.Attr & 0x400) != 0 && (t.Tag & 0x20000000) != 0;
        } finally { Marshal.FreeHGlobal(buf); CloseHandle(h); }
    }

    // The entries of a folder, listed by handle (backup semantics). null +
    // error code if it can't be listed.
    public static List<Entry> List(string dir, out int error) {
        error = 0;
        IntPtr h = CreateFileW(Raw(dir), LIST | SYNC, 7, IntPtr.Zero, 3, BACKUP | REPARSE, IntPtr.Zero);
        if (h == BAD) { error = Marshal.GetLastWin32Error(); return null; }
        var outv = new List<Entry>();
        IntPtr buf = Marshal.AllocHGlobal(65536);
        try {
            int cls = 0x0F;   // FileFullDirectoryRestartInfo, then FileFullDirectoryInfo
            while (true) {
                if (!GetFileInformationByHandleEx(h, cls, buf, 65536)) {
                    int e = Marshal.GetLastWin32Error();
                    if (e == 18) break;   // no more files
                    error = e; return null;
                }
                cls = 0x0E;
                int off = 0;
                while (true) {
                    IntPtr p = new IntPtr(buf.ToInt64() + off);
                    int next = Marshal.ReadInt32(p, 0);
                    uint attr = (uint)Marshal.ReadInt32(p, 56);
                    int nlen = Marshal.ReadInt32(p, 60);
                    // With the reparse attribute set, EaSize (offset 64) holds the reparse tag.
                    uint tag = (attr & 0x400) != 0 ? (uint)Marshal.ReadInt32(p, 64) : 0;
                    bool link = (attr & 0x400) != 0 && (tag & 0x20000000) != 0;   // IsReparseTagNameSurrogate
                    string name = Marshal.PtrToStringUni(new IntPtr(p.ToInt64() + 68), nlen / 2);
                    if (name != "." && name != "..") {
                        string full = dir.EndsWith("\\") ? dir + name : dir + "\\" + name;
                        outv.Add(new Entry { Path = full, Dir = (attr & 0x10) != 0, Link = link });
                    }
                    if (next == 0) break;
                    off += next;
                }
            }
            return outv;
        } finally { Marshal.FreeHGlobal(buf); CloseHandle(h); }
    }

    // A readable description of the explicit (not inherited) allow entries in
    // `sd` that give any of `sids` a bit in `mask` ("" if none). A null DACL
    // grants everyone everything and is reported as such.
    public static string ExplicitGrant(byte[] sd, string[] sids, uint mask) { return Grant(sd, sids, mask, true); }

    // explicitOnly: only entries set on the object itself (any inheritance
    // flags: an inherit-only one reaches what's inside). Otherwise: every
    // allow entry that applies to the object itself (inherited ones too,
    // inherit-only ones not).
    public static string Grant(byte[] sd, string[] sids, uint mask, bool explicitOnly) {
        var r = new RawSecurityDescriptor(sd, 0);
        if (r.DiscretionaryAcl == null) return "no DACL at all (everyone: full control)";
        var parts = new List<string>();
        foreach (GenericAce a in r.DiscretionaryAcl) {
            var c = a as KnownAce;
            if (c == null) continue;
            var t = a.AceType;
            if (t != AceType.AccessAllowed && t != AceType.AccessAllowedCallback && t != AceType.AccessAllowedObject && t != AceType.AccessAllowedCallbackObject) continue;
            if (explicitOnly && (a.AceFlags & AceFlags.Inherited) != 0) continue;
            // Inherit-only counts when it passes to what's inside (review
            // 2026-11-26: Users RX + (OI)(IO)M = every file inside writable).
            if (!explicitOnly && (a.AceFlags & AceFlags.InheritOnly) != 0 && (a.AceFlags & Inh) == 0) continue;
            if (Array.IndexOf(sids, c.SecurityIdentifier.Value) < 0) continue;
            uint m = (uint)c.AccessMask;
            if ((m & mask) == 0) continue;
            string who;
            try { who = c.SecurityIdentifier.Translate(typeof(NTAccount)).Value; } catch { who = c.SecurityIdentifier.Value; }
            parts.Add(who + ": 0x" + m.ToString("X") + ((a.AceFlags & AceFlags.InheritOnly) != 0 ? " (for what's inside)" : ""));
        }
        return string.Join("; ", parts.ToArray());
    }

    public sealed class WalkResult {
        public List<string> Hits = new List<string>();        // path  (detail in Details)
        public List<string> Details = new List<string>();
        public List<bool> HitIsDir = new List<bool>();
        public List<string> Unreadable = new List<string>();
        public List<string> Unlistable = new List<string>();
        public List<string> Links = new List<string>();        // links met (not entered)
        public long Items;
    }

    // Walks everything under `root` (the root itself excluded), never
    // entering a link. Mode "explicit": a hit is an item with an explicit
    // allow giving one of `sids` a bit of `mask`. Mode "access": a hit is an
    // item `checker` gets a bit of `mask` on. Mode "nodeny": a hit is a real
    // folder with no deny for sids[0] covering `mask` that its new items inherit.
    public static WalkResult Walk(string root, string mode, string[] sids, uint mask, GlimAuthz.Checker checker) {
        var res = new WalkResult();
        var q = new Queue<string>(); q.Enqueue(root);
        while (q.Count > 0) {
            string dir = q.Dequeue(); int err;
            var kids = List(dir, out err);
            if (kids == null) { res.Unlistable.Add(dir + " (error " + err + ")"); continue; }
            foreach (var k in kids) {
                res.Items++;
                int e2; byte[] sd = ReadSd(k.Path, out e2);
                if (sd == null) { res.Unreadable.Add(k.Path + " (error " + e2 + ")"); }
                else if (mode == "nodeny") {
                    // a real folder that new items inside wouldn't get a deny from
                    if (k.Dir && !k.Link && !InhDeny(sd, sids[0], mask)) { res.Hits.Add(k.Path); res.Details.Add("no inheritable deny"); res.HitIsDir.Add(true); }
                }
                else if (mode == "explicit") {
                    string d = ExplicitGrant(sd, sids, mask);
                    if (d.Length > 0) { res.Hits.Add(k.Path); res.Details.Add(d); res.HitIsDir.Add(k.Dir && !k.Link); }
                } else {
                    uint g = checker.Granted(sd);
                    if ((g & mask) != 0) { res.Hits.Add(k.Path); res.Details.Add("0x" + (g & mask).ToString("X")); res.HitIsDir.Add(k.Dir && !k.Link); }
                }
                if (k.Link) res.Links.Add(k.Path);
                if (k.Dir && !k.Link) q.Enqueue(k.Path);
            }
        }
        return res;
    }
}
'@
}


# Rights the checks care about.
$GlimRead = 0x1                                   # FILE_READ_DATA / FILE_LIST_DIRECTORY
$GlimWrite = 0x2 -bor 0x4 -bor 0x10000 -bor 0x40 -bor 0x40000 -bor 0x80000  # write/append (add file/subfolder), DELETE, DELETE_CHILD, WRITE_DAC, WRITE_OWNER

# GENERIC_READ / GENERIC_WRITE / GENERIC_ALL as stored in inheritable entries.
$GlimRead = $GlimRead -bor [int64]2147483648 -bor 0x10000000
$GlimWrite = $GlimWrite -bor 0x10 -bor 0x100 -bor 0x40000000 -bor 0x10000000

# Elevated: turn on the backup privilege once, so folders and permissions
# only SYSTEM can open can still be listed and read (review 7).
$GlimBackupOn = $false
try {
    $isAdm = (New-Object Security.Principal.WindowsPrincipal ([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if ($isAdm) { $GlimBackupOn = [GlimFs]::EnableBackup() }
} catch { }

# The four files Windows keeps open with no sharing at a drive root: nobody
# (glim-agent included) can open them, not even to read their permissions.
# They are reported by name as "in use by Windows", never as a failure.
$GlimInUse = 'pagefile.sys', 'hiberfil.sys', 'swapfile.sys', 'DumpStack.log.tmp'
function Test-GlimInUse([string]$Path) {
    (Split-Path -Parent $Path) -match '^[A-Za-z]:\\$' -and $GlimInUse -contains (Split-Path -Leaf $Path)
}

function Get-GlimAccess([string]$Sid, [string]$Path) {
    # The object's own owner, group and DACL (a link: the link's own).
    $e = 0
    $sd = [GlimFs]::ReadSd($Path, [ref]$e)
    if ($null -eq $sd) { throw "can't read the permissions of $Path (error $e)" }
    [GlimAuthz]::Granted($Sid, $sd)
}

# User rights glim-agent may hold (an allowlist: anything else is flagged),
# directly or through any SID its logon carries. Deny rights are fine (they
# only take away). Listing account rights needs elevation (LSA).
$GlimHarmless = 'SeChangeNotifyPrivilege', 'SeIncreaseWorkingSetPrivilege', 'SeTimeZonePrivilege', 'SeShutdownPrivilege', 'SeUndockPrivilege',
    'SeInteractiveLogonRight', 'SeNetworkLogonRight', 'SeBatchLogonRight', 'SeRemoteInteractiveLogonRight'

function Get-GlimPowerfulRights([string]$Sid, [string[]]$GroupSids) {
    $all = @($Sid) + @($GroupSids) + @([GlimAuthz]::LogonSids()) | Sort-Object -Unique
    foreach ($s in $all) {
        foreach ($r in [GlimAuthz]::Rights($s)) {
            if ($r -like 'SeDeny*' -or $GlimHarmless -contains $r) { continue }
            "$r (via $s)"
        }
    }
}
