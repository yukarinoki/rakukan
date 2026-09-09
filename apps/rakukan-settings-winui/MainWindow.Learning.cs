using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;

namespace Rakukan.Settings.WinUI;

public sealed partial class MainWindow
{
    private List<LearningEntry> _learningEntries = new();
    private bool _learningBusy;
    private bool _learningLoaded;

    private void OpenLearning_Click(object sender, RoutedEventArgs e)
    {
        RootNavigation.SelectedItem = RootNavigation.MenuItems.OfType<NavigationViewItem>()
            .First(item => item.Tag?.ToString() == "Learning");
    }

    private async Task RunLearningAsync(Func<Task> action)
    {
        if (_learningBusy) return;
        _learningBusy = true;
        LearningStatus.IsOpen = false;
        LearningProgress.IsActive = true;
        foreach (var button in LearningToolbar.Children.OfType<Button>()) button.IsEnabled = false;
        LearningReloadButton.IsEnabled = false;
        LearningList.IsEnabled = false;
        try { await action(); }
        catch (Exception ex)
        {
            LearningStatus.Severity = InfoBarSeverity.Error;
            LearningStatus.Title = "学習履歴を更新できませんでした";
            LearningStatus.Message = ex.Message;
            LearningStatus.IsOpen = true;
        }
        finally
        {
            _learningBusy = false;
            LearningProgress.IsActive = false;
            foreach (var button in LearningToolbar.Children.OfType<Button>()) button.IsEnabled = true;
            LearningReloadButton.IsEnabled = true;
            LearningList.IsEnabled = true;
            UpdateLearningActions();
        }
    }

    private async Task LoadLearningAsync()
    {
        _learningEntries = await LearningHistoryClient.ListAsync();
        _learningLoaded = true;
        FilterLearning();
    }

    private void FilterLearning()
    {
        if (LearningList is null) return;
        var selected = LearningList.SelectedItem as LearningEntry;
        var filtered = LearningHistoryClient.Filter(_learningEntries, LearningSearch.Text);
        LearningList.ItemsSource = filtered;
        LearningList.SelectedItem = filtered.FirstOrDefault(e => e.Reading == selected?.Reading && e.Surface == selected?.Surface);
        LearningCount.Text = $"{filtered.Count:N0} 件 / 全 {_learningEntries.Count:N0} 件";
        LearningEmpty.Text = _learningLoaded && _learningEntries.Count == 0
            ? "学習履歴はまだありません。\n変換を確定すると、ここに表示されます。"
            : "一致する履歴がありません。";
        LearningEmpty.Visibility = _learningLoaded && filtered.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        UpdateLearningActions();
    }

    private void UpdateLearningActions()
    {
        var selected = LearningList.SelectedItem is LearningEntry;
        LearningEditButton.IsEnabled = selected && !_learningBusy;
        LearningPreferButton.IsEnabled = selected && !_learningBusy;
        LearningDeleteButton.IsEnabled = selected && !_learningBusy;
    }

    private void LearningSearch_TextChanged(object sender, TextChangedEventArgs e) => FilterLearning();
    private void LearningList_SelectionChanged(object sender, SelectionChangedEventArgs e) => UpdateLearningActions();
    private async void LearningReload_Click(object sender, RoutedEventArgs e) => await RunLearningAsync(LoadLearningAsync);
    private async void LearningAdd_Click(object sender, RoutedEventArgs e) => await EditLearningAsync(null);
    private async void LearningEdit_Click(object sender, RoutedEventArgs e)
    {
        if (LearningList.SelectedItem is LearningEntry entry) await EditLearningAsync(entry);
    }
    private async void LearningList_DoubleTapped(object sender, DoubleTappedRoutedEventArgs e)
    {
        if (!_learningBusy && LearningList.SelectedItem is LearningEntry entry) await EditLearningAsync(entry);
    }

    private async Task EditLearningAsync(LearningEntry? existing)
    {
        var reading = new TextBox { Header = "読み", PlaceholderText = "あるの", Text = existing?.Reading ?? "", MaxLength = 128 };
        var surface = new TextBox { Header = "使いたい表記", PlaceholderText = "あるの", Text = existing?.Surface ?? "", MaxLength = 256 };
        var error = new TextBlock { TextWrapping = TextWrapping.Wrap };
        var panel = new StackPanel { Spacing = 12 };
        panel.Children.Add(new TextBlock { Text = "この表記を同じ読みの学習候補より優先します。\nユーザー辞書に登録した候補は、引き続き最優先です。", TextWrapping = TextWrapping.Wrap, Opacity = .7 });
        panel.Children.Add(reading);
        panel.Children.Add(surface);
        panel.Children.Add(error);
        var dialog = new ContentDialog
        {
            XamlRoot = RootGrid.XamlRoot, Title = existing is null ? "学習候補を追加" : "学習候補を編集",
            Content = panel, PrimaryButtonText = "優先して保存", CloseButtonText = "キャンセル",
            DefaultButton = ContentDialogButton.Primary,
        };
        dialog.PrimaryButtonClick += (_, args) =>
        {
            if (string.IsNullOrWhiteSpace(reading.Text) || string.IsNullOrWhiteSpace(surface.Text)
                || reading.Text.Any(char.IsControl) || surface.Text.Any(char.IsControl))
            { args.Cancel = true; error.Text = "読みと表記を入力してください。改行などの制御文字は使えません。"; }
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        await RunLearningAsync(async () =>
        {
            await LearningHistoryClient.SendAsync(new { op = "save", original_reading = existing?.Reading,
                original_surface = existing?.Surface, reading = reading.Text.Trim(), surface = surface.Text.Trim() });
            await LoadLearningAsync();
        });
    }

    private async void LearningPrefer_Click(object sender, RoutedEventArgs e)
    {
        if (LearningList.SelectedItem is not LearningEntry entry) return;
        await RunLearningAsync(async () =>
        {
            await LearningHistoryClient.SendAsync(new { op = "save", original_reading = entry.Reading,
                original_surface = entry.Surface, reading = entry.Reading, surface = entry.Surface });
            await LoadLearningAsync();
        });
    }

    private async void LearningDelete_Click(object sender, RoutedEventArgs e)
    {
        if (LearningList.SelectedItem is not LearningEntry entry) return;
        var dialog = new ContentDialog { XamlRoot = RootGrid.XamlRoot, Title = "この学習を削除しますか？",
            Content = $"{entry.Reading} → {entry.Surface}\n\n学習による優先を解除します。辞書の候補は残り、再び確定すると学習されます。",
            PrimaryButtonText = "削除", CloseButtonText = "キャンセル", DefaultButton = ContentDialogButton.Close };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        await RunLearningAsync(async () =>
        {
            await LearningHistoryClient.SendAsync(new { op = "delete", reading = entry.Reading, surface = entry.Surface });
            await LoadLearningAsync();
        });
    }
}
