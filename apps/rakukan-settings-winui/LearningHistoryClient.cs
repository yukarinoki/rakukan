using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Rakukan.Settings.WinUI;

internal sealed class LearningEntry
{
    [JsonPropertyName("reading")] public string Reading { get; set; } = "";
    [JsonPropertyName("surface")] public string Surface { get; set; } = "";
    [JsonPropertyName("last_access_time")] public long LastAccessTime { get; set; }
    [JsonPropertyName("frequency")] public double Frequency { get; set; }
    public string LastUsed => LastAccessTime > 0 && LastAccessTime <= 253402300799
        ? DateTimeOffset.FromUnixTimeSeconds(LastAccessTime).ToLocalTime().ToString("M/d HH:mm") : "—";
}

internal static class LearningHistoryClient
{
    public static async Task<JsonElement> SendAsync(object command)
    {
        var host = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "rakukan", "rakukan-engine-host.exe");
        if (!File.Exists(host)) throw new InvalidOperationException("エンジンホストが見つかりません。インストール状態を確認してください。");
        var start = new ProcessStartInfo(host)
        {
            UseShellExecute = false, CreateNoWindow = true,
            RedirectStandardInput = true, RedirectStandardOutput = true, RedirectStandardError = true,
            StandardInputEncoding = new UTF8Encoding(false), StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
        };
        start.ArgumentList.Add("--manage-learning");
        using var process = Process.Start(start) ?? throw new InvalidOperationException("学習履歴を開けませんでした。");
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(20));
        try
        {
            var output = process.StandardOutput.ReadToEndAsync(timeout.Token);
            var error = process.StandardError.ReadToEndAsync(timeout.Token);
            await process.StandardInput.WriteAsync(JsonSerializer.Serialize(command));
            process.StandardInput.Close();
            await process.WaitForExitAsync(timeout.Token);
            var json = await output;
            var stderr = await error;
            if (process.ExitCode != 0) throw new InvalidOperationException($"学習履歴の処理に失敗しました。{stderr}");
            return ParseResponse(json);
        }
        catch (OperationCanceledException)
        {
            if (!process.HasExited) process.Kill(); // bridge only; never kill the shared host
            throw new InvalidOperationException("エンジンの応答を確認できませんでした。変更が反映済みの場合もあるため、再読み込みしてください。");
        }
    }

    internal static JsonElement ParseResponse(string json)
    {
        if (string.IsNullOrWhiteSpace(json)) throw new InvalidOperationException("エンジンホストの更新が必要です。設定画面を開き直してください。");
        using var document = JsonDocument.Parse(json);
        var root = document.RootElement;
        if (root.TryGetProperty("error", out var error)) throw new InvalidOperationException(error.GetString());
        return root.Clone();
    }

    public static async Task<List<LearningEntry>> ListAsync()
    {
        var response = await SendAsync(new { op = "list" });
        return response.GetProperty("entries").Deserialize<List<LearningEntry>>() ?? new();
    }

    public static List<LearningEntry> Filter(IEnumerable<LearningEntry> entries, string query)
    {
        query = query.Trim();
        return entries.Where(e => e.Reading.Contains(query, StringComparison.OrdinalIgnoreCase)
            || e.Surface.Contains(query, StringComparison.OrdinalIgnoreCase)).ToList();
    }
}
