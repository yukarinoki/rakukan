using System.Text;
using System.Text.Json;
using Rakukan.AI;
Console.InputEncoding = Encoding.UTF8;
Console.OutputEncoding = new UTF8Encoding(false);
try
{
    var json = await Console.In.ReadToEndAsync();
    if (json.Length > 131072) throw new InvalidOperationException("AI要求が長すぎます。");
    var request = JsonSerializer.Deserialize<AiRequest>(json, AiConfig.Json) ?? throw new ArgumentException("AI要求が不正です。");
    var result = await AiBackend.RunAsync(request);
    Console.Write(JsonSerializer.Serialize(result, AiConfig.Json));
}
catch (Exception ex)
{
    var error = ex is OperationCanceledException ? "AIの応答がタイムアウトしました。" : ex.Message;
    Console.Write(JsonSerializer.Serialize(new AiResult("", false, "", 0, error), AiConfig.Json));
    Environment.ExitCode = 1;
}
