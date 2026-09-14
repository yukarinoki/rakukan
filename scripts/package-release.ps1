param(
    [string]$BuildDir = 'C:\rb',
    [string]$IsccPath = 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
    [string[]]$IsccExtraArgs = @()
)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$dist = [IO.Path]::GetFullPath((Join-Path $repoRoot 'dist'))
if ($dist -ne [IO.Path]::Combine($repoRoot, 'dist')) { throw 'Unexpected staging path' }
if (Test-Path -LiteralPath $dist) { Remove-Item -LiteralPath $dist -Recurse -Force }
New-Item -ItemType Directory -Force $dist | Out-Null
$version = (Get-Content "$repoRoot\VERSION" -Raw).Trim()
if ($version -ne '0.11.5') { throw 'Update release pins and installer version before packaging another version' }
Copy-Item "$BuildDir\release\rakukan_tsf.dll" $dist
Copy-Item "$BuildDir\release\rakukan_engine.dll" "$dist\rakukan_engine_cpu.dll"
Copy-Item "$BuildDir\release\rakukan-engine-host.exe" $dist
Copy-Item "$repoRoot\crates\rakukan-tsf\rakukan.ico" $dist
Copy-Item "$repoRoot\config\config.toml" $dist
Copy-Item "$PSScriptRoot\register-tip.ps1", "$PSScriptRoot\unregister-tip.ps1" $dist
Copy-Item "$repoRoot\LICENSE", "$repoRoot\NOTICE", "$repoRoot\docs\THIRD_PARTY_LICENSES.md" $dist
Copy-Item "$repoRoot\.build\release-data\rakukan.dict" $dist
Copy-Item "$repoRoot\.build\release-data\models" "$dist\models" -Recurse
Copy-Item "$repoRoot\.build\release-data\sources.json" $dist
Copy-Item "$repoRoot\.build\publish\ai" "$dist\ai" -Recurse
Copy-Item "$repoRoot\.build\publish\settings-ui" "$dist\settings-ui" -Recurse
foreach ($name in @('ai', 'settings-ui')) {
    if (-not (Test-Path "$dist\$name\coreclr.dll")) { throw "$name must be published self-contained" }
}
foreach ($name in @('App.xbf', 'MainWindow.xbf', 'rakukan-settings.pri')) {
    if (-not (Test-Path "$dist\settings-ui\$name")) { throw "Missing WinUI runtime resource: $name" }
}
Push-Location $repoRoot
try {
    cargo metadata --locked --offline --filter-platform x86_64-pc-windows-msvc --format-version 1 > .build\release-metadata.json
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
    python scripts\collect-release-licenses.py .build\release-metadata.json "$dist\licenses"
    if ($LASTEXITCODE -ne 0) { throw 'License collection failed' }
    $files = Get-ChildItem $dist -Recurse -File | ForEach-Object {
        @{ path = [IO.Path]::GetRelativePath($dist, $_.FullName); sha256 = (Get-FileHash $_.FullName).Hash; size = $_.Length }
    }
    @{ version = $version; commit = (git rev-parse HEAD); files = @($files) } | ConvertTo-Json -Depth 4 | Set-Content "$dist\manifest.json" -Encoding utf8
    & $IsccPath @IsccExtraArgs rakukan_installer.iss
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
    $installer = "$repoRoot\output\yurukan-$version-setup.exe"
    $hash = (Get-FileHash $installer).Hash.ToLowerInvariant()
    "$hash  yurukan-$version-setup.exe" | Set-Content "$repoRoot\output\SHA256SUMS.txt" -Encoding ascii
    Copy-Item "$dist\manifest.json" "$repoRoot\output\yurukan-$version-manifest.json" -Force
} finally { Pop-Location }
