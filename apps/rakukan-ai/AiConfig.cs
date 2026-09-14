using System.Text.Json;

namespace Rakukan.AI;

public sealed class AiCommand : System.ComponentModel.INotifyPropertyChanged
{
    private string _name = "";
    public string Name { get => _name; set { _name = value; PropertyChanged?.Invoke(this, new(nameof(Name))); } }
    public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;
    public string Aliases { get; set; } = "";
    public string Prompt { get; set; } = "";
    public bool Append { get; set; }
    public string Language { get; set; } = "japanese";
    public override string ToString() => Name;
}

public sealed class AiConfig
{
    public bool Enabled { get; set; }
    public string Key { get; set; } = "Henkan";
    public string Backend { get; set; } = "local";
    public string Url { get; set; } = "http://127.0.0.1:8081/v1";
    public string Model { get; set; } = "";
    public string ModelPath { get; set; } = AiDownload.DefaultFiles.ModelPath;
    public string ServerPath { get; set; } = AiDownload.DefaultFiles.ServerPath;
    public int Port { get; set; } = 8081;
    public int TimeoutSeconds { get; set; } = 180;
    public int MaxTokens { get; set; } = 512;
    public List<AiCommand> Commands { get; set; } = new()
    {
        new() { Name="続き", Aliases="つづき,予測", Prompt="Write only the natural continuation of the original text in the same language. Do not repeat the original text.", Append=true },
        new() { Name="英語", Aliases="えいご,english", Language="english", Prompt="Translate the original text into natural English. Output only the translation." },
        new() { Name="丁寧", Aliases="ていねい", Prompt="元の文章を自然で丁寧な日本語に書き換えてください。結果だけを出力してください。" },
        new() { Name="メール", Aliases="めーる,mail", Prompt="元の文章を自然な日本語のメール文に整えてください。事実を付け加えず、結果だけを出力してください。" },
    };
    public static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower, WriteIndented = true, PropertyNameCaseInsensitive = true };
    public static string DirectoryPath => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "rakukan");
    public static string FilePath => Path.Combine(DirectoryPath, "ai.json");
    public static AiConfig Load() => File.Exists(FilePath) ? JsonSerializer.Deserialize<AiConfig>(File.ReadAllText(FilePath), Json) ?? new() : new();
    public void Validate(bool requireConnection = false)
    {
        if (Backend is not ("copilot" or "local" or "remote")) throw new ArgumentException("接続方式が不正です。");
        if (!ValidKey(Key)) throw new ArgumentException("AIキーは Henkan、F1〜F12、または Ctrl+Shift+A の形式で指定してください。");
        if (Port is < 1 or > 65535 || TimeoutSeconds is < 5 or > 600 || MaxTokens is < 16 or > 4096) throw new ArgumentException("ポート、待機時間、生成上限を確認してください。");
        if (Backend == "remote" && (!Uri.TryCreate(Url, UriKind.Absolute, out var uri) || uri.Scheme is not ("http" or "https") || !string.IsNullOrEmpty(uri.UserInfo))) throw new ArgumentException("接続先を http(s)://ホスト:ポート/v1 の形式で指定してください。");
        if (Backend == "local" && (Enabled || requireConnection) && (!File.Exists(ModelPath) || !File.Exists(ServerPath))) throw new ArgumentException("モデルと llama-server のファイルを指定してください。");
        if (Commands.Count == 0 || Commands.Any(c => string.IsNullOrWhiteSpace(c.Name) || string.IsNullOrWhiteSpace(c.Prompt))) throw new ArgumentException("コマンド名と指示文を入力してください。");
        var aliases = Commands.SelectMany(c => new[] { c.Name }.Concat(c.Aliases.Split(',', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries))).ToList();
        if (aliases.Distinct(StringComparer.OrdinalIgnoreCase).Count() != aliases.Count) throw new ArgumentException("コマンド名・別名が重複しています。");
    }
    public static bool ValidKey(string key)
    {
        var parts = key.Split('+', StringSplitOptions.TrimEntries);
        if (parts.Take(parts.Length - 1).Any(x => x is not ("Ctrl" or "Shift" or "Alt"))) return false;
        var last = parts[^1];
        return last == "Henkan" || (last.StartsWith('F') && int.TryParse(last[1..], out var f) && f >= 1 && f <= 12) || (parts.Length > 1 && last.Length == 1 && char.IsAsciiLetterUpper(last[0]));
    }
    public bool Save()
    {
        Validate(); Directory.CreateDirectory(DirectoryPath);
        var json = JsonSerializer.Serialize(this, Json);
        if (File.Exists(FilePath) && File.ReadAllText(FilePath) == json) return false;
        var temp = FilePath + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try { File.WriteAllText(temp, json); File.Move(temp, FilePath, true); return true; }
        finally { if (File.Exists(temp)) File.Delete(temp); }
    }
}

public sealed class AiRequest
{
    public string Op { get; set; } = "generate";
    public string Text { get; set; } = "";
    public string Instruction { get; set; } = "";
    public string Previous { get; set; } = "";
    public bool Append { get; set; } = true;
    public string Language { get; set; } = "japanese";
    public AiConfig? Config { get; set; }
    public string? ApiKey { get; set; }
}

public sealed record AiResult(string Text, bool Append, string Backend, long ElapsedMs, string? Error = null, string Language = "japanese", long? StartupMs = null, long? RequestMs = null);
