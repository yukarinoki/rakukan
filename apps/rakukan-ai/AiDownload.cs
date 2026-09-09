using System.IO.Compression;
using System.Security.Cryptography;

namespace Rakukan.AI;

public sealed record AiDownloadProgress(string Stage, long Received, long Total);
public sealed record AiLocalFiles(string ModelPath, string ServerPath);

public static class AiDownload
{
    public sealed record ModelOffer(string Id, string File, long Bytes, string Sha256, string Revision, string Repository);
    public static IReadOnlyList<ModelOffer> Models { get; } = Array.AsReadOnly(new[] {
        new ModelOffer("2b", "Qwen_Qwen3.5-2B-Q4_K_M.gguf", 1396198496,
            "57a1085840f497d764a7fc5d346922dbde961efb54cc792ea81d694fd846a1d8",
            "7d26695454df6de5fbcce2e58681e62dae06ce43", "bartowski/Qwen_Qwen3.5-2B-GGUF"),
        new ModelOffer("0.8b", "Qwen_Qwen3.5-0.8B-Q4_K_M.gguf", 579615840,
            "fb044e93939a70469c905781334f5de1e6c8b608ced6cbc8c9249bd4127d9526",
            "f36b1ea49a332ede8fe5f389bbf5b3575ef71f48", "bartowski/Qwen_Qwen3.5-0.8B-GGUF"),
    });
    private const string ServerVersion = "b10869";
    private const long ServerBytes = 18426417;
    private const string ServerSha = "407746b275594c05412f49cd8625631da0f7ca7117e5c4bb258e91db8f07ae29";
    private const string ServerUrl = "https://github.com/ggml-org/llama.cpp/releases/download/b10869/llama-b10869-bin-win-cpu-x64.zip";
    public static string DirectoryPath => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "rakukan", "ai-local");

    public static async Task<AiLocalFiles> InstallAsync(IProgress<AiDownloadProgress>? progress = null, CancellationToken ct = default, string modelId = "2b")
    {
        var offer = Models.SingleOrDefault(m => m.Id == modelId) ?? throw new ArgumentException("モデルを選択してください。");
        if (!Environment.Is64BitProcess || System.Runtime.InteropServices.RuntimeInformation.ProcessArchitecture != System.Runtime.InteropServices.Architecture.X64)
            throw new PlatformNotSupportedException("このダウンロードはWindows x64 CPU用です。");
        Directory.CreateDirectory(DirectoryPath);
        // Exclusive file lease works across settings and command-line processes, across awaits.
        await using var lease = new FileStream(Path.Combine(DirectoryPath, ".download.lock"), FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None);
        using var http = new HttpClient { Timeout = Timeout.InfiniteTimeSpan };
        http.DefaultRequestHeaders.UserAgent.ParseAdd("rakukan-ai/0.11.4");
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(ct);
        timeout.CancelAfter(TimeSpan.FromMinutes(30));
        ct = timeout.Token;
        var model = Path.Combine(DirectoryPath, offer.File);
        var modelUrl = new Uri($"https://huggingface.co/{offer.Repository}/resolve/{offer.Revision}/{offer.File}");
        await DownloadVerifiedAsync(http, modelUrl, model, offer.Bytes, offer.Sha256, "モデル", progress, ct);
        var archive = Path.Combine(DirectoryPath, $"llama-{ServerVersion}-cpu-x64.zip");
        await DownloadVerifiedAsync(http, new(ServerUrl), archive, ServerBytes, ServerSha, "CPUサーバー", progress, ct);
        var runtime = Path.Combine(DirectoryPath, "llama-" + ServerVersion);
        var executable = Path.Combine(runtime, "llama-server.exe");
        var marker = Path.Combine(runtime, ".complete");
        if (!File.Exists(executable) || !File.Exists(marker) || await File.ReadAllTextAsync(marker, ct) != ServerSha)
        {
            progress?.Report(new("サーバーを展開中", 0, 0));
            var staging = Path.Combine(DirectoryPath, ".extract-" + Guid.NewGuid().ToString("N"));
            try
            {
                await ExtractServerAsync(archive, staging, ct);
                // Some release archives have a containing directory.
                var server = Directory.GetFiles(staging, "llama-server.exe", SearchOption.AllDirectories).Single();
                var source = Path.GetDirectoryName(server)!;
                if (Directory.Exists(runtime))
                    throw new IOException("サーバーの展開先に未完了のファイルがあります。" + runtime + " を確認してください。");
                await File.WriteAllTextAsync(Path.Combine(source, ".complete"), ServerSha, ct);
                Directory.Move(source, runtime);
            }
            finally { if (Directory.Exists(staging)) Directory.Delete(staging, true); }
        }
        progress?.Report(new("ダウンロード完了", 1, 1));
        return new(model, executable);
    }

    public static async Task DownloadVerifiedAsync(HttpClient http, Uri url, string destination, long expectedBytes, string sha256,
        string stage, IProgress<AiDownloadProgress>? progress, CancellationToken ct)
    {
        if (File.Exists(destination))
        {
            progress?.Report(new(stage + "を確認中", 0, 0));
            await using var existing = File.OpenRead(destination);
            if (existing.Length == expectedBytes && Convert.ToHexString(await SHA256.HashDataAsync(existing, ct)).Equals(sha256, StringComparison.OrdinalIgnoreCase))
            { progress?.Report(new(stage, expectedBytes, expectedBytes)); return; }
        }
        Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(destination))!);
        var partial = destination + "." + Guid.NewGuid().ToString("N") + ".part";
        try
        {
            using var response = await http.GetAsync(url, HttpCompletionOption.ResponseHeadersRead, ct);
            response.EnsureSuccessStatusCode();
            if (response.Content.Headers.ContentLength is long length && length != expectedBytes)
                throw new IOException(stage + "のサイズが配布情報と一致しません。");
            await using var input = await response.Content.ReadAsStreamAsync(ct);
            using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
            long received = 0; var buffer = new byte[128 * 1024];
            await using (var output = new FileStream(partial, FileMode.CreateNew, FileAccess.Write, FileShare.None, buffer.Length, true))
            {
                int count; var lastReport = System.Diagnostics.Stopwatch.StartNew();
                while ((count = await input.ReadAsync(buffer, ct)) > 0)
                {
                    received += count;
                    if (received > expectedBytes) throw new IOException(stage + "のサイズが上限を超えました。");
                    hash.AppendData(buffer, 0, count); await output.WriteAsync(buffer.AsMemory(0, count), ct);
                    if (lastReport.ElapsedMilliseconds >= 100) { progress?.Report(new(stage, received, expectedBytes)); lastReport.Restart(); }
                }
                await output.FlushAsync(ct);
            }
            if (received != expectedBytes || !Convert.ToHexString(hash.GetHashAndReset()).Equals(sha256, StringComparison.OrdinalIgnoreCase))
                throw new IOException(stage + "の検証に失敗しました。もう一度ダウンロードしてください。");
            ct.ThrowIfCancellationRequested();
            File.Move(partial, destination, true);
            progress?.Report(new(stage, received, expectedBytes));
        }
        finally { if (File.Exists(partial)) File.Delete(partial); }
    }

    public static async Task ExtractServerAsync(string archive, string directory, CancellationToken ct)
    {
        var root = Path.GetFullPath(directory).TrimEnd(Path.DirectorySeparatorChar) + Path.DirectorySeparatorChar;
        Directory.CreateDirectory(root);
        using var zip = ZipFile.OpenRead(archive);
        long total = 0;
        foreach (var entry in zip.Entries)
        {
            ct.ThrowIfCancellationRequested();
            var target = Path.GetFullPath(Path.Combine(root, entry.FullName));
            if (!target.StartsWith(root, StringComparison.OrdinalIgnoreCase) || entry.FullName.Contains(':') || ((entry.ExternalAttributes >> 16) & 0xF000) == 0xA000)
                throw new IOException("サーバーアーカイブに不正なパスが含まれています。");
            total += entry.Length;
            if (total > 512L * 1024 * 1024) throw new IOException("サーバーアーカイブが大きすぎます。");
            if (entry.Name.Length == 0) { Directory.CreateDirectory(target); continue; }
            Directory.CreateDirectory(Path.GetDirectoryName(target)!);
            await using var input = entry.Open();
            await using var output = new FileStream(target, FileMode.CreateNew, FileAccess.Write, FileShare.None);
            await input.CopyToAsync(output, ct);
        }
    }
}
