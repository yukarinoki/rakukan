using Rakukan.AI;
using System.IO.Pipes;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;

int passed = 0;
void Check(bool ok, string label) { if (!ok) throw new Exception(label); passed++; }
async Task Throws(Func<Task> action, string label) { try { await action(); } catch { passed++; return; } throw new Exception(label); }
var cfg = new AiConfig(); cfg.Validate();
var translated = AiBackend.Resolve(cfg, new() { Text = "元の文", Instruction = "えいご" });
Check(!translated.Append && translated.Language == "english" && translated.Prompt.Contains("Translate"), "English alias resolves language and replacement");
using var copilotRequest = JsonDocument.Parse(JsonSerializer.Serialize(AiBackend.CopilotRequest("id", "文章", "Translate", "english")));
Check(copilotRequest.RootElement.GetProperty("lang").GetString() == "english" && copilotRequest.RootElement.GetProperty("action").GetInt32() == 4, "Copilot generation uses lang, not language");
var continuation = AiBackend.Resolve(cfg, new() { Text = "元の文" });
Check(continuation.Append && continuation.Prompt.Contains("continuation"), "Default continuation appends");
var correction = AiBackend.Resolve(cfg, new() { Text = "元の文", Previous = "Previous translation", Instruction = "もっと短く", Append = false, Language = "english" });
Check(!correction.Append && correction.Language == "english" && correction.Prompt.Contains("Previous translation") && correction.Prompt.Contains("もっと短く"), "Correction retains original, previous result and language");
var different = AiBackend.Resolve(cfg, new() { Text = "元の文", Previous = "old", Instruction = "えいご", Append = false });
Check(!different.Append && different.Prompt.Contains("old"), "Alternative retains transformation");
Check(AiBackend.CleanOutput("<think>private reasoning</think> answer") == "answer", "Strip thinking");
Check(AiBackend.CleanOutput("<think>unfinished") == "", "Never expose unfinished thinking");
Check(AiConfig.ValidKey("Henkan") && AiConfig.ValidKey("Ctrl+Shift+A") && !AiConfig.ValidKey("A") && !AiConfig.ValidKey("F13"), "AI shortcut validation");
var clone = JsonSerializer.Deserialize<AiConfig>(JsonSerializer.Serialize(cfg, AiConfig.Json), AiConfig.Json)!;
Check(clone.Commands[1].Language == "english" && clone.Commands[0].Append, "Config roundtrip");
clone.Commands.Add(new() { Name = "えいご", Prompt = "duplicate" });
await Throws(() => { clone.Validate(); return Task.CompletedTask; }, "Duplicate alias rejected");
var invalid = new AiConfig { Backend = "remote", Url = "file:///C:/secret" };
await Throws(() => { invalid.Validate(); return Task.CompletedTask; }, "Reject non-HTTP endpoint");
await Throws(() => AiBackend.RunAsync(new() { Text = new string('x', 17000) }), "Reject oversized source before contacting model");

// Exercise real framing with a unique pipe, never the user's Copilot server.
async Task PipeCase(bool malformed)
{
    string name = "rakukan-ai-test-" + Guid.NewGuid().ToString("N");
    using var server = new NamedPipeServerStream(name, PipeDirection.InOut, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous);
    using var timeout = new CancellationTokenSource(3000);
    var serve = Task.Run(async () =>
    {
        await server.WaitForConnectionAsync(timeout.Token);
        var header = new byte[4]; await server.ReadExactlyAsync(header, timeout.Token);
        var body = new byte[BitConverter.ToInt32(header)]; await server.ReadExactlyAsync(body, timeout.Token);
        using var request = JsonDocument.Parse(body); Check(request.RootElement.GetProperty("action").GetInt32() == 0, "Pipe request preserved");
        if (malformed) { await server.WriteAsync(BitConverter.GetBytes(int.MaxValue), timeout.Token); return; }
        var bytes = Encoding.UTF8.GetBytes("{\"success\":true,\"text\":\"日本語\"}");
        foreach (byte b in BitConverter.GetBytes(bytes.Length).Concat(bytes)) { await server.WriteAsync(new[] { b }, timeout.Token); }
    });
    if (malformed) await Throws(() => AiBackend.CopilotAsync(new { action = 0 }, timeout.Token, name), "Reject oversized pipe response");
    else { var reply = await AiBackend.CopilotAsync(new { action = 0 }, timeout.Token, name); Check(reply.GetProperty("text").GetString() == "日本語", "Fragmented UTF8 pipe reply"); }
    await serve;
}
await PipeCase(false); await PipeCase(true);

async Task HttpCase(int status, bool cancel, bool continuationCase = false)
{
    var listener = new TcpListener(IPAddress.Loopback, 0); listener.Start();
    try
    {
        var port = ((IPEndPoint)listener.LocalEndpoint).Port;
        using var cts = new CancellationTokenSource(5000);
        var serve = Task.Run(async () =>
        {
            using var client = await listener.AcceptTcpClientAsync(); await using var stream = client.GetStream();
            var header = new List<byte>(); var one = new byte[1];
            while (header.Count < 8192) { await stream.ReadExactlyAsync(one); header.Add(one[0]); if (header.Count >= 4 && Encoding.ASCII.GetString(header.TakeLast(4).ToArray()) == "\r\n\r\n") break; }
            var head = Encoding.ASCII.GetString(header.ToArray());
            Check(head.StartsWith("POST /v1/chat/completions") && head.Contains("Bearer test-token"), "OpenAI URL and authorization");
            int length = int.Parse(head.Split("\r\n").First(l => l.StartsWith("Content-Length:", StringComparison.OrdinalIgnoreCase)).Split(':')[1]);
            var body = new byte[length]; await stream.ReadExactlyAsync(body); using var json = JsonDocument.Parse(body);
            Check(json.RootElement.GetProperty("model").GetString() == "test-model" && json.RootElement.GetProperty("max_tokens").GetInt32() == 512, "Model and output limit forwarded");
            if (cancel) { cts.Cancel(); await Task.Delay(20); return; }
            var response = JsonSerializer.SerializeToUtf8Bytes(new { choices = new[] { new { message = new { content = continuationCase ? "文章の続き" : "Hello" } } } });
            await stream.WriteAsync(Encoding.ASCII.GetBytes($"HTTP/1.1 {status} Test\r\nContent-Length: {response.Length}\r\nConnection: close\r\n\r\n"));
            await stream.WriteAsync(response);
        });
        var request = new AiRequest { Text = "文章", Instruction = continuationCase ? "" : "えいご", ApiKey = "test-token", Config = new() { Backend = "remote", Url = $"http://127.0.0.1:{port}/v1", Model = "test-model" } };
        if (status != 200 || cancel) await Throws(() => AiBackend.RunAsync(request, cts.Token), "HTTP failure/cancel reported");
        else { var reply = await AiBackend.RunAsync(request, cts.Token); Check(continuationCase ? reply.Text == "の続き" && reply.Append : reply.Text == "Hello" && !reply.Append, "Remote result and duplicate-prefix handling"); }
        await serve;
    }
    finally { listener.Stop(); }
}
await HttpCase(200, false); await HttpCase(401, false); await HttpCase(200, true); await HttpCase(200, false, true);
Console.WriteLine($"AI tests passed: {passed}");
