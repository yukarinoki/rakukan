using System.Text;
using Rakukan.Settings.WinUI;

var directory = Path.Combine(Path.GetTempPath(), "rakukan-dictionary-test-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(directory);
void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
string FileAt(string name) => Path.Combine(directory, name);
try
{
    var learningJson = LearningHistoryClient.ParseResponse("""
        {"entries":[{"reading":"あるの","surface":"アルノ","last_access_time":1788959706,"frequency":4.99},
        {"reading":"あるの","surface":"あるの","last_access_time":1788959707,"frequency":6}]}
        """);
    var learning = System.Text.Json.JsonSerializer.Deserialize<List<LearningEntry>>(learningJson.GetProperty("entries"))!;
    Check(learning.Count == 2 && learning[0].Frequency == 4.99 && learning[0].LastUsed != "—", "learning JSON/UTF-8/date");
    Check(LearningHistoryClient.Filter(learning, " あるの ").Count == 2, "learning reading search");
    Check(LearningHistoryClient.Filter(learning, "アルノ").Count == 1, "learning surface search");
    Check(LearningHistoryClient.Filter(learning, "見つからない").Count == 0, "learning empty search");
    try { LearningHistoryClient.ParseResponse("{\"error\":\"保存できません\"}"); throw new Exception("ignored server error"); }
    catch (InvalidOperationException ex) { Check(ex.Message == "保存できません", "learning server error"); }
    try { LearningHistoryClient.ParseResponse(""); throw new Exception("accepted empty old-host response"); }
    catch (InvalidOperationException) { }
    Console.WriteLine("Learning history parsing, search, error and old-host response tests PASS");
    var appearanceRoot = Tomlyn.Toml.ToModel("[appearance]\ncandidate_font_height=33\ncaret_width_enabled=true\ncaret_on_width=6\ncaret_off_width=2\n");
    var settings = SettingsStore.LoadConfig(appearanceRoot);
    Check(settings.CaretWidthEnabled && settings.CaretOnWidth == 6 && settings.CaretOffWidth == 2 && settings.CandidateFontHeight == 33, "appearance load");
    SettingsStore.SaveConfig(appearanceRoot, settings);
    var reloaded = SettingsStore.LoadConfig(Tomlyn.Toml.ToModel(Tomlyn.Toml.FromModel(appearanceRoot)));
    Check(reloaded.CaretWidthEnabled && reloaded.CaretOnWidth == 6 && reloaded.CaretOffWidth == 2 && reloaded.CandidateFontHeight == 33, "appearance round trip");
    var legacy = SettingsStore.LoadConfig(Tomlyn.Toml.ToModel("[appearance]\ncandidate_font_height=17"));
    Check(!legacy.CaretWidthEnabled && legacy.CaretOnWidth == 4 && legacy.CaretOffWidth == 1, "legacy defaults");
    Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
    const string sample = "しながし\t品貸\t名詞\r\nたのむら\t田之村\t姓\r\n";
    foreach (var encoding in new Encoding[] { new UnicodeEncoding(false, true, true), new UnicodeEncoding(true, true, true), new UTF8Encoding(true, true), new UTF8Encoding(false, true), Encoding.GetEncoding(932) })
    {
        File.WriteAllText(FileAt("input.txt"), sample, encoding);
        var imported = UserDictionaryTransfer.Read(FileAt("input.txt"));
        Check(imported.Count == 2 && imported[1].WordInfo["田之村"].PartOfSpeech == "姓", "encoding/part of speech");
        SettingsStore.SaveUserDict(FileAt("user.toml"), imported);
        UserDictionaryTransfer.Write(FileAt("output.txt"), SettingsStore.LoadUserDict(FileAt("user.toml")));
        Check(File.ReadAllText(FileAt("output.txt")) == sample, "TOML/text round trip");
        Check(File.ReadAllBytes(FileAt("output.txt")).Take(2).SequenceEqual(new byte[] { 0xff, 0xfe }), "export BOM");
    }
    File.WriteAllText(FileAt("input.txt"), "!Microsoft IME dictionary\n# comment\n\nよみ\t語\t名詞\tコメント\nよみ\t別語\nよみ\t語\n");
    var target = new List<UserDictEntry>();
    var result = UserDictionaryTransfer.Merge(target, UserDictionaryTransfer.Read(FileAt("input.txt")));
    Check(result == (2, 1) && target.Count == 1, "grouping/deduplication");
    Check(UserDictionaryTransfer.Merge(target, UserDictionaryTransfer.Read(FileAt("input.txt"))) == (0, 3), "repeat import");
    SettingsStore.SaveUserDict(FileAt("user.toml"), target);
    Check(SettingsStore.LoadUserDict(FileAt("user.toml"))[0].WordInfo["語"].Comment == "コメント", "comment preservation");
    File.WriteAllText(FileAt("input.txt"), "よみ\t単語\ninvalid row\n");
    try { UserDictionaryTransfer.Merge(target, UserDictionaryTransfer.Read(FileAt("input.txt"))); throw new Exception("accepted malformed row"); }
    catch (FormatException ex) { Check(ex.Message.StartsWith("2 行目"), "error line"); }
    Check(target[0].Surfaces.Count == 2, "atomic import");
    File.WriteAllText(FileAt("output.txt"), "original");
    target[0].WordInfo["語"] = new("名詞", "bad\ncomment");
    try { UserDictionaryTransfer.Write(FileAt("output.txt"), target); throw new Exception("accepted newline"); }
    catch (FormatException) { }
    Check(File.ReadAllText(FileAt("output.txt")) == "original", "failed export preserves file");
    if (args.Length > 0)
    {
        var actual = UserDictionaryTransfer.Read(args[0]);
        SettingsStore.SaveUserDict(FileAt("actual.toml"), actual);
        UserDictionaryTransfer.Write(FileAt("actual.txt"), SettingsStore.LoadUserDict(FileAt("actual.toml")));
        Check(File.ReadAllBytes(args[0]).SequenceEqual(File.ReadAllBytes(FileAt("actual.txt"))), "supplied file byte round trip");
        Console.WriteLine($"Supplied dictionary: {actual.Sum(e => e.Surfaces.Count)} words, byte-identical round trip PASS");
    }
    Console.WriteLine("Encoding, metadata, merge, duplicate, malformed input and safe export tests PASS");
}
finally { Directory.Delete(directory, recursive: true); }
