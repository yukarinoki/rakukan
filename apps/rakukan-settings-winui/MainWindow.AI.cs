using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Rakukan.AI;

namespace Rakukan.Settings.WinUI;
public sealed partial class MainWindow
{
    private AiConfig _aiConfig = new();
    private bool _aiReady;
    private bool _aiPersistedEnabled;
    private bool _aiSavingEnabled;
    private AiCommand? _aiCommand;
    private CancellationTokenSource? _aiCancel;
    private CancellationTokenSource? _aiDownloadCancel;
    private async void AiDownload_Click(object sender, RoutedEventArgs e)
    {
        if (_aiDownloadCancel != null) return;
        _aiDownloadCancel = new();
        AiDownloadButton.IsEnabled = false; AiDownloadCancelButton.IsEnabled = true;
        AiDownloadProgress.Visibility = Visibility.Visible; AiDownloadProgress.IsIndeterminate = true;
        AiDownloadStatus.Text = "ダウンロードを準備中…";
        try
        {
            var progress = new Progress<AiDownloadProgress>(p =>
            {
                if (_aiDownloadCancel == null) return;
                AiDownloadProgress.IsIndeterminate = p.Total == 0;
                if (p.Total > 0) AiDownloadProgress.Value = 100.0 * p.Received / p.Total;
                AiDownloadStatus.Text = p.Total > 1 ? $"{p.Stage}：{p.Received / 1_000_000.0:N1} / {p.Total / 1_000_000.0:N1} MB" : p.Stage;
            });
            var modelId = (AiDownloadModel.SelectedItem as ComboBoxItem)?.Tag?.ToString() ?? "2b";
            var files = await AiDownload.InstallAsync(progress, _aiDownloadCancel.Token, modelId);
            AiServerPath.Text = files.ServerPath; AiModelPath.Text = files.ModelPath; AiModelName.Text = "";
            AiBackendCombo.SelectedItem = AiBackendCombo.Items.OfType<ComboBoxItem>().First(i => i.Tag?.ToString() == "local");
            var config = CaptureAiSettings(); config.Validate();
            AiBackend.StopLocalIfDifferent(config); config.Save();
            AiDownloadStatus.Text = "保存しました。軽量モデルを使えます。「起動・確認」で接続を確認できます。";
        }
        catch (Exception ex)
        {
            AiDownloadStatus.Text = ex is OperationCanceledException ? "ダウンロードを中止しました。もう一度押すと取得済みのファイルを確認して再試行します。" : "ダウンロードまたは設定保存に失敗しました：" + ex.Message;
        }
        finally
        {
            _aiDownloadCancel.Dispose(); _aiDownloadCancel = null;
            AiDownloadButton.IsEnabled = true; AiDownloadCancelButton.IsEnabled = false;
            AiDownloadProgress.Visibility = Visibility.Collapsed;
        }
    }
    private void AiDownloadCancel_Click(object sender, RoutedEventArgs e) => _aiDownloadCancel?.Cancel();
    private void AiSection_Click(object sender, RoutedEventArgs e)
    {
        SelectAiSection((sender as FrameworkElement)?.Tag?.ToString());
    }
    private void SelectAiSection(string? tag)
    {
        AiConnectionSection.Visibility = tag == "connection" ? Visibility.Visible : Visibility.Collapsed;
        AiCommandsSection.Visibility = tag == "commands" ? Visibility.Visible : Visibility.Collapsed;
        AiTestSection.Visibility = tag == "test" ? Visibility.Visible : Visibility.Collapsed;
        AiConnectionTab.IsChecked = tag == "connection"; AiCommandsTab.IsChecked = tag == "commands"; AiTestTab.IsChecked = tag == "test";
    }
    private void LoadAiSettings()
    {
        try { _aiConfig = AiConfig.Load(); AiApiKey.Password = AiSecret.Load(); }
        catch (Exception ex) { AiTestStatus.Text = "AI設定を読み込めませんでした: " + ex.Message; }
        AiEnabled.IsOn = _aiConfig.Enabled; AiKey.Text = _aiConfig.Key;
        AiBackendCombo.SelectedItem = AiBackendCombo.Items.OfType<ComboBoxItem>().FirstOrDefault(i => i.Tag?.ToString() == _aiConfig.Backend);
        AiUrl.Text = _aiConfig.Url; AiServerPath.Text = _aiConfig.ServerPath; AiModelPath.Text = _aiConfig.ModelPath;
        AiModelName.Text = _aiConfig.Model; AiPort.Value = _aiConfig.Port; AiTimeout.Value = _aiConfig.TimeoutSeconds; AiMaxTokens.Value = _aiConfig.MaxTokens;
        RefreshAiCommands(); AiBackend_Changed(null!, null!);
        _aiPersistedEnabled = AiEnabled.IsOn;
        _aiReady = true;
    }
    private void AiEnabled_Toggled(object sender, RoutedEventArgs e)
    {
        if (!_aiReady || _aiSavingEnabled) return;
        _aiSavingEnabled = true;
        try
        {
            // Enabling applies the visible AI settings. Disabling always works even
            // when other unsaved fields are incomplete. Never reload the conversion engine.
            var config = AiEnabled.IsOn ? CaptureAiSettings() : AiConfig.Load();
            config.Enabled = AiEnabled.IsOn;
            config.Validate();
            if (config.Enabled) AiSecret.Save(AiApiKey.Password);
            config.Save();
            _aiPersistedEnabled = config.Enabled;
        }
        catch (Exception ex)
        {
            AiEnabled.IsOn = _aiPersistedEnabled;
            AiTestStatus.Text = "AIモードを反映できませんでした: " + ex.Message;
            SelectAiSection("test");
        }
        finally { _aiSavingEnabled = false; }
    }
    private void AiBackend_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (AiLocalFields == null) return;
        var backend = (AiBackendCombo.SelectedItem as ComboBoxItem)?.Tag?.ToString();
        AiLocalFields.Visibility = backend == "local" ? Visibility.Visible : Visibility.Collapsed;
        AiRemoteFields.Visibility = backend == "remote" ? Visibility.Visible : Visibility.Collapsed;
        AiCopilotNote.Visibility = backend == "copilot" ? Visibility.Visible : Visibility.Collapsed;
        AiModelName.Visibility = backend == "copilot" ? Visibility.Collapsed : Visibility.Visible;
        AiMaxTokens.IsEnabled = backend != "copilot";
    }
    private void StoreAiCommand()
    {
        if (_aiCommand == null) return;
        _aiCommand.Name = AiCommandName.Text.Trim(); _aiCommand.Aliases = AiAliases.Text.Trim();
        _aiCommand.Prompt = AiPrompt.Text; _aiCommand.Append = AiAppend.IsChecked == true;
        _aiCommand.Language = (AiCommandLanguage.SelectedItem as ComboBoxItem)?.Tag?.ToString() ?? "japanese";
    }
    private void AiCommand_Changed(object sender, SelectionChangedEventArgs e)
    {
        StoreAiCommand(); _aiCommand = AiCommands.SelectedItem as AiCommand;
        AiCommandName.Text = _aiCommand?.Name ?? ""; AiAliases.Text = _aiCommand?.Aliases ?? "";
        AiPrompt.Text = _aiCommand?.Prompt ?? ""; AiAppend.IsChecked = _aiCommand?.Append ?? false;
        AiCommandLanguage.SelectedItem = AiCommandLanguage.Items.OfType<ComboBoxItem>().FirstOrDefault(i => i.Tag?.ToString() == (_aiCommand?.Language ?? "japanese"));
    }
    private void RefreshAiCommands()
    {
        _aiCommand = null; AiCommands.ItemsSource = null; AiCommands.ItemsSource = _aiConfig.Commands.ToList();
        AiCommands.SelectedIndex = _aiConfig.Commands.Count - 1;
    }
    private void AiAddCommand_Click(object sender, RoutedEventArgs e)
    {
        StoreAiCommand(); _aiConfig.Commands.Add(new() { Name = "新しいコマンド", Prompt = "文章を書き換えてください。" }); RefreshAiCommands();
    }
    private void AiDeleteCommand_Click(object sender, RoutedEventArgs e)
    {
        if (_aiCommand != null) _aiConfig.Commands.Remove(_aiCommand); RefreshAiCommands();
    }
    private AiConfig CaptureAiSettings()
    {
        StoreAiCommand();
        return new AiConfig
        {
            Enabled = AiEnabled.IsOn,
            Key = AiKey.Text.Trim(),
            Backend = (AiBackendCombo.SelectedItem as ComboBoxItem)?.Tag?.ToString() ?? "copilot",
            Url = AiUrl.Text.Trim(),
            ServerPath = AiServerPath.Text.Trim().Trim('"'),
            ModelPath = AiModelPath.Text.Trim().Trim('"'),
            Model = AiModelName.Text.Trim(),
            Port = (int)AiPort.Value,
            TimeoutSeconds = (int)AiTimeout.Value,
            MaxTokens = (int)AiMaxTokens.Value,
            Commands = _aiConfig.Commands
        };
    }
    private async Task TestAiAsync(string op)
    {
        if (_aiCancel != null) return;
        _aiCancel = new(); AiProgress.IsActive = true; AiConnectButton.IsEnabled = false; AiGenerateButton.IsEnabled = false;
        AiTestStatus.Text = op == "status" ? "接続を確認中…" : "生成中… 初回はモデルの読み込み時間を含む場合があります。"; AiTestOutput.Text = "";
        try
        {
            var result = await AiBackend.RunAsync(new() { Op = op, Text = AiTestText.Text, Instruction = AiTestInstruction.Text, Config = CaptureAiSettings(), ApiKey = AiApiKey.Password }, _aiCancel.Token);
            AiTestStatus.Text = $"成功 · {result.Backend} · 全体 {result.ElapsedMs:N0} ms\n起動・準備: {(result.StartupMs is long startup ? $"{startup:N0} ms" : "共有／外部のため内訳取得不可")} · 要求応答: {result.RequestMs:N0} ms";
            if (op == "status" && result.Backend == "copilot")
            {
                using var status = System.Text.Json.JsonDocument.Parse(result.Text);
                var data = status.RootElement;
                string field(string name) => data.TryGetProperty(name, out var v) ? v.ToString() : "不明";
                AiTestOutput.Text = $"モデル: {field("text")}\n実行先: {field("hardwareType")}\n保存状態: {field("modelStatus")}\nモデル容量: {field("fileSizeMb")} MB\n接続確認は生成成功を保証しません。「生成を試す」で確認してください。";
            }
            else AiTestOutput.Text = result.Text;
        }
        catch (Exception ex) { AiTestStatus.Text = ex is OperationCanceledException ? "中止またはタイムアウトしました。" : ex.Message; }
        finally { _aiCancel.Dispose(); _aiCancel = null; AiProgress.IsActive = false; AiConnectButton.IsEnabled = true; AiGenerateButton.IsEnabled = true; }
    }
    private async void AiConnect_Click(object sender, RoutedEventArgs e) { SelectAiSection("test"); await TestAiAsync("status"); }
    private async void AiGenerate_Click(object sender, RoutedEventArgs e) => await TestAiAsync("generate");
    private void AiCancel_Click(object sender, RoutedEventArgs e) => _aiCancel?.Cancel();
    private async void AiBrowse_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            bool server = (sender as Button)?.Tag?.ToString() == "server";
            var picker = new Windows.Storage.Pickers.FileOpenPicker();
            picker.FileTypeFilter.Add(server ? ".exe" : ".gguf");
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));
            var file = await picker.PickSingleFileAsync();
            if (file != null) { if (server) AiServerPath.Text = file.Path; else AiModelPath.Text = file.Path; }
        }
        catch (Exception ex) { AiTestStatus.Text = ex.Message; }
    }
    private void AiStop_Click(object sender, RoutedEventArgs e)
    {
        SelectAiSection("test");
        try { AiBackend.StopLocal(CaptureAiSettings()); AiTestStatus.Text = "ローカルサーバーを停止しました。"; }
        catch (Exception ex) { AiTestStatus.Text = ex.Message; }
    }
}
