using System.Text;
using Rakukan.Settings.WinUI;

var directory = Path.Combine(Path.GetTempPath(), "rakukan-dictionary-test-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(directory);
void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
string FileAt(string name) => Path.Combine(directory, name);
try
{
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
