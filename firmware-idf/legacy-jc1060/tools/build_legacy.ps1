param(
    [Parameter(Mandatory=$true)][string]$Idf,
    [Parameter(Mandatory=$true)][string]$Output,
    [Parameter(Mandatory=$true)][ValidateSet('pure-ram','flash-build-only','memory-task','display-touch','sd-readonly','sd-file-readonly','sd-rust-bridge','sd-rust-exfat','sd-rust-load','sd-rust-load-display')][string]$Variant,
    [string]$Python='python',
    [string]$Wsl,
    [string]$CargoHome,
    [string]$RustupHome
)
$ErrorActionPreference='Stop'
$buildArgs=@("$PSScriptRoot/build_legacy.py",'--idf',$Idf,'--output',$Output,'--variant',$Variant)
if($Wsl){$buildArgs+=@('--wsl',$Wsl)}
if($CargoHome){$buildArgs+=@('--cargo-home',$CargoHome)}
if($RustupHome){$buildArgs+=@('--rustup-home',$RustupHome)}
& $Python @buildArgs
exit $LASTEXITCODE
