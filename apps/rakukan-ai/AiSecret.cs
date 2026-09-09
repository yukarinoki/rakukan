using System.Runtime.InteropServices;
using System.Text;
namespace Rakukan.AI;
// Windows DPAPI: per-user encryption; never include the token in ai.json or logs.
public static class AiSecret
{
    [StructLayout(LayoutKind.Sequential)] private struct Blob { public int Size; public IntPtr Data; }
    [DllImport("crypt32.dll", SetLastError = true, CharSet = CharSet.Unicode)] private static extern bool CryptProtectData(ref Blob input, string? description, IntPtr entropy, IntPtr reserved, IntPtr prompt, int flags, out Blob output);
    [DllImport("crypt32.dll", SetLastError = true)] private static extern bool CryptUnprotectData(ref Blob input, IntPtr description, IntPtr entropy, IntPtr reserved, IntPtr prompt, int flags, out Blob output);
    [DllImport("kernel32.dll")] private static extern IntPtr LocalFree(IntPtr p);
    private static string PathName => Path.Combine(AiConfig.DirectoryPath, "ai-key.bin");
    private static byte[] Transform(byte[] bytes, bool protect)
    {
        var input = new Blob { Size = bytes.Length, Data = Marshal.AllocHGlobal(bytes.Length) };
        Blob output = default;
        try
        {
            Marshal.Copy(bytes, 0, input.Data, bytes.Length);
            bool ok = protect ? CryptProtectData(ref input, null, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, 1, out output) : CryptUnprotectData(ref input, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, 1, out output);
            if (!ok) throw new InvalidOperationException("認証キーを保存・復号できません。");
            var result = new byte[output.Size]; Marshal.Copy(output.Data, result, 0, result.Length); return result;
        }
        finally { Marshal.FreeHGlobal(input.Data); if (output.Data != IntPtr.Zero) LocalFree(output.Data); }
    }
    public static string Load() => File.Exists(PathName) ? Encoding.UTF8.GetString(Transform(File.ReadAllBytes(PathName), false)) : "";
    public static bool Save(string value)
    {
        if (Load() == value) return false;
        Directory.CreateDirectory(AiConfig.DirectoryPath);
        if (value.Length == 0) { if (File.Exists(PathName)) File.Delete(PathName); return true; }
        File.WriteAllBytes(PathName, Transform(Encoding.UTF8.GetBytes(value), true));
        return true;
    }
}
