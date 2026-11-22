# Credential Manager helpers for the agent sandbox scripts: the same format as
# Glim's own vault (windows/src-tauri/src/ai/key.rs): a GENERIC credential,
# user name "Glim", the secret as UTF-8 bytes, persisted on this machine.
# Dot-source it; it defines Write-GlimCredential, Read-GlimCredential and
# Test-GlimCredential for the CURRENT user's vault only.

if (-not ('GlimCred' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class GlimCred {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct CREDENTIAL {
        public uint Flags; public uint Type; public string TargetName; public string Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint CredentialBlobSize; public IntPtr CredentialBlob; public uint Persist;
        public uint AttributeCount; public IntPtr Attributes; public string TargetAlias; public string UserName;
    }
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CredWriteW(ref CREDENTIAL c, uint flags);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CredReadW(string target, uint type, uint flags, out IntPtr c);
    [DllImport("advapi32.dll")]
    static extern void CredFree(IntPtr p);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CredDeleteW(string target, uint type, uint flags);
    public static bool Delete(string target) { return CredDeleteW(target, GENERIC, 0); }
    const uint GENERIC = 1, LOCAL_MACHINE = 2;
    public static void Write(string target, string secret) {
        byte[] b = Encoding.UTF8.GetBytes(secret);
        IntPtr p = Marshal.AllocHGlobal(b.Length);
        try {
            Marshal.Copy(b, 0, p, b.Length);
            var c = new CREDENTIAL { Type = GENERIC, TargetName = target, UserName = "Glim", CredentialBlobSize = (uint)b.Length, CredentialBlob = p, Persist = LOCAL_MACHINE };
            if (!CredWriteW(ref c, 0)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        } finally {
            for (int i = 0; i < b.Length; i++) { b[i] = 0; Marshal.WriteByte(p, i, 0); }
            Marshal.FreeHGlobal(p);
        }
    }
    public static string Read(string target) {
        IntPtr p;
        if (!CredReadW(target, GENERIC, 0, out p)) return null;
        try {
            var c = (CREDENTIAL)Marshal.PtrToStructure(p, typeof(CREDENTIAL));
            byte[] b = new byte[c.CredentialBlobSize];
            Marshal.Copy(c.CredentialBlob, b, 0, b.Length);
            string s = Encoding.UTF8.GetString(b);
            Array.Clear(b, 0, b.Length);
            return s;
        } finally { CredFree(p); }
    }
}
'@
}

function Write-GlimCredential([string]$Target, [string]$Secret) { [GlimCred]::Write($Target, $Secret) }
function Read-GlimCredential([string]$Target) { [GlimCred]::Read($Target) }
function Remove-GlimCredential([string]$Target) { [void][GlimCred]::Delete($Target) }
function Test-GlimCredential([string]$Target) { $null -ne [GlimCred]::Read($Target) }
