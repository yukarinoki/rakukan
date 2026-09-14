# Windows release — yurukan 0.11.5

The downloadable installer is published on
[GitHub Releases](https://github.com/yukarinoki/yurukan/releases).

This release supports x64 Windows 10 (1809 or later) and Windows 11. The
CPU conversion engine, public Mozc dictionary, jinen-v1-xsmall Q5_K_M model,
tokenizer, .NET runtime and WinUI runtime are included. The
[Visual C++ x64 Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe)
is required; setup checks for it before installation.

The installer is currently unsigned. SHA256SUMS.txt and a file manifest are
published alongside it. Existing applications must be fully restarted to load
an updated TSF DLL; sign out and back in if Windows still shows the old name.

## Defaults and compatibility

- Live conversion enabled; AI mode disabled.
- AI backend: local llama.cpp with Qwen3.5-2B Q4_K_M. Download is optional and
  initiated from the AI settings page; the large AI model is not in the installer.
- Half-width katakana and full-width alphanumeric input modes disabled.
  Enable each independently under Basic settings to show it in the taskbar menu.
  Disabled modes cannot be selected. Disabling an active mode returns to normal
  input on the next input operation. F8/F9 character conversion remains separate
  from these persistent input modes.
- Existing explicit configuration values, dictionaries and learning are preserved.
- Internal filenames, `%LOCALAPPDATA%\rakukan`, `%APPDATA%\rakukan` and TSF IDs
  remain stable for upgrades. This replaces the upstream installation; the two
  editions cannot be installed side by side.

## Build and package

Use a Visual Studio developer PowerShell with Rust, LLVM/libclang, CMake, Ninja,
.NET 8+, Python 3 and Inno Setup 6 available. Build from a clean, committed tree.
Use a short Cargo output path to avoid MSVC path-length limits.

```powershell
$env:CARGO_TARGET_DIR = 'C:\rb'
cargo build -p rakukan-tsf -p rakukan-engine -p rakukan-engine-host -p rakukan-dict-builder --release --locked
dotnet publish apps/rakukan-ai/rakukan-ai.csproj -c Release -r win-x64 --self-contained true -o .build/publish/ai
# Use Visual Studio MSBuild (amd64) for WinUI XAML compilation:
MSBuild apps/rakukan-settings-winui/Rakukan.Settings.WinUI.csproj /restore /t:Publish /p:Configuration=Release /p:Platform=x64 /p:SelfContained=true /p:PublishDir="$PWD/.build/publish/settings-ui/"
./scripts/prepare-release-data.ps1 -BuildDir C:\rb
./scripts/package-release.ps1 -BuildDir C:\rb
```

`prepare-release-data.ps1` uses fixed public source revisions and verifies the
model hash. It builds a fresh dictionary and never copies personal settings,
user dictionaries, logs, API keys or learning history. `package-release.ps1`
collects original dependency notices and fails if required license files or
self-contained runtime files are missing. The default AI paths are calculated
on each user's PC, not copied from the release builder's account.

Test before release:

```powershell
cargo test -p rakukan-tsf --release --lib --locked
cargo clippy -p rakukan-tsf --release --locked -- -D warnings
dotnet run --project apps/rakukan-ai-tests/rakukan-ai-tests.csproj -c Release
dotnet run --project apps/rakukan-settings-tests/Rakukan.Settings.Tests.csproj -c Release
```

Publish `output/yurukan-0.11.5-setup.exe`, `output/SHA256SUMS.txt` and
`output/yurukan-0.11.5-manifest.json` from the same tagged commit.

## Licensing

The fork remains MIT, retaining rakukan's original copyright notice and attribution.
The binary distribution contains the original Mozc license (including dictionary
notices for Google, NAIST and ICOT), jinen's CC BY-SA 4.0 license and model card,
the Mozc date-conversion BSD notice, and dependency licenses. Model weights are
not relicensed as MIT. See [third-party licenses](THIRD_PARTY_LICENSES.md).
