param(
    [Parameter(Mandatory=$true)][string]$Idf,
    [Parameter(Mandatory=$true)][string]$Output,
    [Parameter(Mandatory=$true)][ValidateSet('pure-ram','flash-build-only','memory-task','display-touch','sd-readonly','sd-file-readonly','sd-rust-bridge','sd-rust-exfat','sd-rust-load','sd-rust-load-display','usb-host','usb-midi','deck-ui','deck-audio','deck-play','deck-mix','deck-engine')][string]$Variant,
    [string]$Python='python',
    [string]$Wsl,
    [string]$CargoHome,
    [string]$RustupHome,
    [switch]$UacProfile
)
$ErrorActionPreference='Stop'
$buildArgs=@("$PSScriptRoot/build_legacy.py",'--idf',$Idf,'--output',$Output,'--variant',$Variant)
if($Wsl){$buildArgs+=@('--wsl',$Wsl)}
if($CargoHome){$buildArgs+=@('--cargo-home',$CargoHome)}
if($RustupHome){$buildArgs+=@('--rustup-home',$RustupHome)}
if($UacProfile){$buildArgs+='--uac-profile'}
& $Python @buildArgs
exit $LASTEXITCODE
