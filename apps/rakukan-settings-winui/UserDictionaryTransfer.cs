using System.Text;

namespace Rakukan.Settings.WinUI;

internal sealed record UserDictWordInfo(string PartOfSpeech = "名詞", string Comment = "");

internal static class UserDictionaryTransfer
{
    // Validate the entire input before merging, so a bad row never causes a partial import.
    public static List<UserDictEntry> Read(string path)
    {
        var bytes = File.ReadAllBytes(path);
        string text;
        if (bytes.AsSpan().StartsWith(new byte[] { 0xff, 0xfe }))
            text = new UnicodeEncoding(false, true, true).GetString(bytes, 2, bytes.Length - 2);
        else if (bytes.AsSpan().StartsWith(new byte[] { 0xfe, 0xff }))
            text = new UnicodeEncoding(true, true, true).GetString(bytes, 2, bytes.Length - 2);
        else
        {
            try { text = new UTF8Encoding(false, true).GetString(bytes).TrimStart('\ufeff'); }
            catch (DecoderFallbackException)
            {
                Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);
                text = Encoding.GetEncoding(932, EncoderFallback.ExceptionFallback,
                    DecoderFallback.ExceptionFallback).GetString(bytes);
            }
        }

        var result = new List<UserDictEntry>();
        using var reader = new StringReader(text);
        int lineNumber = 0;
        while (reader.ReadLine() is { } line)
        {
            lineNumber++;
            if (string.IsNullOrWhiteSpace(line) || line.TrimStart().StartsWith('!') || line.TrimStart().StartsWith('#'))
                continue;
            var fields = line.Split('\t');
            if (fields.Length < 2 || fields.Length > 4 || fields.Any(f => f.Contains('\0'))
                || string.IsNullOrWhiteSpace(fields[0]) || string.IsNullOrWhiteSpace(fields[1]))
                throw new FormatException($"{lineNumber} 行目: 読み・単語・品詞（省略可）・コメント（省略可）をタブで区切ってください。");
            var surface = fields[1].Trim();
            var info = new UserDictWordInfo(fields.Length > 2 && !string.IsNullOrWhiteSpace(fields[2]) ? fields[2].Trim() : "名詞",
                fields.Length > 3 ? fields[3] : "");
            result.Add(new UserDictEntry { Reading = fields[0].Trim(), Surfaces = new() { surface },
                WordInfo = new() { [surface] = info } });
        }
        return result;
    }

    public static (int Added, int Duplicates) Merge(IList<UserDictEntry> destination, IEnumerable<UserDictEntry> source)
    {
        var byReading = destination.GroupBy(e => e.Reading, StringComparer.Ordinal)
            .ToDictionary(g => g.Key, g => g.Last(), StringComparer.Ordinal);
        int added = 0, duplicates = 0;
        foreach (var incoming in source)
        {
            if (!byReading.TryGetValue(incoming.Reading, out var entry))
            {
                entry = new UserDictEntry { Reading = incoming.Reading };
                destination.Add(entry);
                byReading.Add(entry.Reading, entry);
            }
            var surfaces = new List<string>(entry.Surfaces);
            foreach (var surface in incoming.Surfaces)
            {
                if (surfaces.Contains(surface, StringComparer.Ordinal)) { duplicates++; continue; }
                surfaces.Add(surface);
                entry.WordInfo[surface] = incoming.WordInfo.GetValueOrDefault(surface) ?? new();
                added++;
            }
            entry.Surfaces = surfaces;
        }
        return (added, duplicates);
    }

    public static void Write(string path, IEnumerable<UserDictEntry> entries)
    {
        var text = new StringBuilder();
        foreach (var entry in entries)
        foreach (var surface in entry.Surfaces)
        {
            var info = entry.WordInfo.GetValueOrDefault(surface) ?? new();
            var fields = new[] { entry.Reading, surface, info.PartOfSpeech, info.Comment };
            if (fields.Any(f => f.IndexOfAny(new[] { '\t', '\r', '\n', '\0' }) >= 0)
                || string.IsNullOrWhiteSpace(entry.Reading) || string.IsNullOrWhiteSpace(surface))
                throw new FormatException("読み・単語・品詞・コメントにタブや改行を含む項目は書き出せません。");
            text.Append(string.Join("\t", info.Comment.Length == 0 ? fields.Take(3) : fields)).Append("\r\n");
        }
        // Write a sibling temporary file first; keep an existing export intact on failure.
        var temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try
        {
            File.WriteAllText(temporary, text.ToString(), new UnicodeEncoding(false, true, true));
            File.Move(temporary, path, overwrite: true);
        }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
}
