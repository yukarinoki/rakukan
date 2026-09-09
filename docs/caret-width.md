# IME 状態によるキャレット幅

設定アプリの「外観」に、候補フォントサイズと「入力位置のキャレット」をまとめました。
「IME のオン／オフで太さを変える」を有効にして保存すると、オン・オフそれぞれの
太さ（1～20、初期値 4 / 1）を使用します。新規設定では無効です。

Windows のアクセシビリティ設定と同じ `SPI_SETCARETWIDTH` を使用します。
独自描画のアプリでは反映されない場合や、入力欄の選び直しが必要な場合があります。
フォントサイズと異なり、値は Windows のキャレット幅設定へそのまま渡します。
文字色や上下のインジケーター、点滅間隔は変更しません。

```toml
[appearance]
candidate_font_height = 17
caret_width_enabled = true
caret_on_width = 4
caret_off_width = 1
```

システム設定の変更は `rakukan-tray.exe` のワーカーで実行し、TSF のキー処理を
ブロックしません。設定は 1 秒ごとに読み直し、モード通知または最大 250ms の間隔で
前面プロセスとモードを確認します。変更がない場合は Windows への設定要求を省略します。

TSF と常駐プロセス間は v2 の共有メモリでプロセス ID とモードを 64-bit atomic に
まとめて通知します。旧版の 4-byte 共有メモリとは名前を分けています。
両方の更新と、TSF を使っているアプリの再起動が必要です。

無効化、別プロセスへのフォーカス移動、TSF の非アクティブ化、常駐プロセスの正常終了で
元の太さに戻します。現在の値が rakukan の設定した値と異なる場合、ユーザーが別途
変更したと判断し、その値を維持します。Windows の永続設定には書き込みません。
常駐プロセスの強制終了時はすぐには戻せませんが、次回起動時に
`%LOCALAPPDATA%\rakukan\caret-width-restore.toml` から復旧します。

検証:

```powershell
cargo test -p rakukan-tray --release --locked --offline
cargo test -p rakukan-tray --release --locked --offline windows_width_switch_and_restore -- --ignored --nocapture
```

後者は Windows の太さを一時変更し、4 → 1 → 元の値への復帰と、外部から変更した値を
復元時に上書きしないことを確認します。テスト終了時に元の値へ戻します。

参考: [SystemParametersInfoW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow)
