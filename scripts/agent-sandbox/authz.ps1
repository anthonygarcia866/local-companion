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

if (-not ('GlimAuthz' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
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
        IntPtr h = CreateFileW(path, 0x80 /* FILE_READ_ATTRIBUTES */, 7 /* share all */, IntPtr.Zero, 3 /* OPEN_EXISTING */, 0x02000000 /* BACKUP_SEMANTICS: folders too */, IntPtr.Zero);
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
        uint any = 0;
        foreach (var v in Variants) {
            var sids = new string[Base.Length + v.Length];
            Base.CopyTo(sids, 0); v.CopyTo(sids, Base.Length);
            any |= GrantedWith(sid, sd, sids);
        }
        return any;
    }

    static uint GrantedWith(string sid, byte[] sd, string[] logon) {
        IntPtr rm, ctx, ctx2 = IntPtr.Zero;
        if (!AuthzInitializeResourceManager(1 /* NO_AUDIT */, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, "glim-check", out rm)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        try {
            var s = new SecurityIdentifier(sid); var b = new byte[s.BinaryLength]; s.GetBinaryForm(b, 0);
            if (!AuthzInitializeContextFromSid(0, b, rm, IntPtr.Zero, new LUID(), IntPtr.Zero, out ctx)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            var pins = new GCHandle[logon.Length];
            var add = new SID_AND_ATTRIBUTES[logon.Length];
            try {
                for (int i = 0; i < logon.Length; i++) {
                    var g = new SecurityIdentifier(logon[i]); var gb = new byte[g.BinaryLength]; g.GetBinaryForm(gb, 0);
                    pins[i] = GCHandle.Alloc(gb, GCHandleType.Pinned);
                    add[i] = new SID_AND_ATTRIBUTES { Sid = pins[i].AddrOfPinnedObject(), Attributes = 7 /* MANDATORY|ENABLED_BY_DEFAULT|ENABLED */ };
                }
                if (!AuthzAddSidsToContext(ctx, add, (uint)add.Length, IntPtr.Zero, 0, out ctx2)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            } finally { foreach (var p in pins) if (p.IsAllocated) p.Free(); AuthzFreeContext(ctx); }
            var req = new REQUEST { DesiredAccess = 0x02000000 /* MAXIMUM_ALLOWED */ };
            IntPtr mask = Marshal.AllocHGlobal(4), err = Marshal.AllocHGlobal(4);
            try {
                var rep = new REPLY { ResultListLength = 1, GrantedAccessMask = mask, Error = err };
                if (!AuthzAccessCheck(0, ctx2, ref req, IntPtr.Zero, sd, IntPtr.Zero, 0, ref rep, IntPtr.Zero)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                return (uint)Marshal.ReadInt32(mask);
            } finally { Marshal.FreeHGlobal(mask); Marshal.FreeHGlobal(err); }
        } finally { if (ctx2 != IntPtr.Zero) AuthzFreeContext(ctx2); AuthzFreeResourceManager(rm); }
    }
}
'@
}

# Rights the checks care about.
$GlimRead = 0x1                                   # FILE_READ_DATA / FILE_LIST_DIRECTORY
$GlimWrite = 0x2 -bor 0x4 -bor 0x10000 -bor 0x40 -bor 0x40000 -bor 0x80000  # write/append (add file/subfolder), DELETE, DELETE_CHILD, WRITE_DAC, WRITE_OWNER

function Get-GlimAccess([string]$Sid, [string]$Path) {
    # The object's owner, group and DACL, read by the owner (who can).
    $sd = (Get-Acl -LiteralPath $Path).GetSecurityDescriptorBinaryForm()
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
