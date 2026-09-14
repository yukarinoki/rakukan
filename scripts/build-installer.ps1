# Compatibility entry point; package fresh build output, never a user's installation.
# See docs/windows-release.md for the preceding build/publish steps.
param(
    [string]$Version,
    [string]$BuildDir = 'C:\rb',
    [string]$IsccPath = 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
    [switch]$Sign,
    [string]$CertSubject = $env:CODESIGN_CERT,
    [string]$SigntoolPath,
    [string]$TimestampUrl = 'http://timestamp.digicert.com',
    [switch]$AutoSelectCert
)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
if ($Version -and $Version -ne (Get-Content "$repoRoot\VERSION" -Raw).Trim()) {
    throw 'Version must match VERSION, Cargo.toml and installer metadata'
}
$extra = @()
if ($Sign) {
    . "$PSScriptRoot\signtool-common.ps1"
    $signtool = Find-SignTool -SigntoolPath $SigntoolPath
    $subject = Resolve-CertSubject -CertSubject $CertSubject -AutoSelectCert:$AutoSelectCert
    $q = '$q'; $f = '$f'
    $certPart = if ($subject) { "/n $q$subject$q" } else { '/a' }
    $extra = @('/DSIGN', "/Srakukan=$q$signtool$q sign /fd SHA256 $certPart /tr $TimestampUrl /td SHA256 $f")
}
& "$PSScriptRoot\package-release.ps1" -BuildDir $BuildDir -IsccPath $IsccPath -IsccExtraArgs $extra
