# Download only public, pinned release data. Never package a user's settings or history.
param([string]$BuildDir = 'C:\rb')
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$stage = Join-Path $repoRoot '.build\release-data'
New-Item -ItemType Directory -Force "$stage\mozc", "$stage\models" | Out-Null
$mozcRevision = 'cbbb6e1bd181cb9f3b409622d916a53a35400ec7'
$modelRevision = 'ec3d27722d73c12362df6de3939e27aedda50590'
function Fetch([string]$Url, [string]$Path) {
    Invoke-WebRequest -Uri $Url -OutFile "$Path.part" -TimeoutSec 120
    Move-Item -LiteralPath "$Path.part" -Destination $Path -Force
}
$inputs = @()
foreach ($n in 0..9) {
    $name = 'dictionary{0:d2}.txt' -f $n
    $path = "$stage\mozc\$name"
    Fetch "https://raw.githubusercontent.com/google/mozc/$mozcRevision/src/data/dictionary_oss/$name" $path
    $inputs += @('--input', $path)
}
Fetch "https://raw.githubusercontent.com/google/mozc/$mozcRevision/src/data/symbol/symbol.tsv" "$stage\mozc\symbol.tsv"
Fetch "https://raw.githubusercontent.com/google/mozc/$mozcRevision/src/data/emoji/emoji_data.tsv" "$stage\mozc\emoji_data.tsv"
& "$BuildDir\release\rakukan-dict-builder.exe" @inputs --symbol "$stage\mozc\symbol.tsv" --emoji "$stage\mozc\emoji_data.tsv" --output "$stage\rakukan.dict"
if ($LASTEXITCODE -ne 0) { throw 'Dictionary build failed' }
foreach ($name in @('jinen-v1-xsmall-Q5_K_M.gguf', 'tokenizer.json')) {
    Fetch "https://huggingface.co/togatogah/jinen-v1-xsmall.gguf/resolve/$modelRevision/$name" "$stage\models\$name"
}
if ((Get-FileHash "$stage\models\jinen-v1-xsmall-Q5_K_M.gguf").Hash -ne 'bb3110f06e539bf8596756df85a48b3946f1378e6cb912322b9c368be06d79aa') {
    throw 'Model hash mismatch'
}
if ((Get-FileHash "$stage\models\tokenizer.json").Hash -ne 'dde9713961ba536b14f20ed0c6e166abeeb5444886b966c590da1ad44dc9a3af') {
    throw 'Tokenizer hash mismatch'
}
@{ mozc = $mozcRevision; jinen = $modelRevision } | ConvertTo-Json | Set-Content "$stage\sources.json" -Encoding utf8
Write-Output "Release data prepared: $stage"
