$ErrorActionPreference = "Stop"

$Version = if ($env:AX_VERSION) { $env:AX_VERSION } else { "latest" }
if ($env:AX_RELEASE_BASE) {
  $Base = $env:AX_RELEASE_BASE
} elseif ($Version -eq "latest") {
  $Base = "https://github.com/axlanguage/axlang/releases/latest/download"
} else {
  $Base = "https://github.com/axlanguage/axlang/releases/download/$Version"
}
$BinDir = if ($env:AX_BIN_DIR) { $env:AX_BIN_DIR } else { "$HOME\.ax\bin" }

$Arch = if ([Environment]::Is64BitOperatingSystem) { "x64" } else { throw "unsupported Windows architecture" }
$Target = "windows-$Arch.exe"

New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
$Out = Join-Path $BinDir "ax.exe"
$Tmp = New-TemporaryFile
$Sums = New-TemporaryFile

function Copy-AxAsset {
  param(
    [string]$Base,
    [string]$Name,
    [string]$OutFile
  )

  if ($Base.StartsWith("file://")) {
    $Path = [Uri]::new("$Base/$Name").LocalPath
    Copy-Item -Force -Path $Path -Destination $OutFile
    return
  }

  $LocalPath = Join-Path $Base $Name
  if (Test-Path $LocalPath) {
    Copy-Item -Force -Path $LocalPath -Destination $OutFile
    return
  }

  Invoke-WebRequest -Uri "$Base/$Name" -OutFile $OutFile
}

try {
  Copy-AxAsset -Base $Base -Name "ax-$Target" -OutFile $Tmp.FullName

  try {
    Copy-AxAsset -Base $Base -Name "SHA256SUMS" -OutFile $Sums.FullName
    $Expected = $null
    foreach ($Line in Get-Content $Sums) {
      $Parts = $Line -split "\s+"
      if ($Parts.Length -ge 2 -and $Parts[1] -eq "ax-$Target") {
        $Expected = $Parts[0].ToLowerInvariant()
      }
    }

    if ($Expected) {
      $Actual = (Get-FileHash -Algorithm SHA256 -Path $Tmp.FullName).Hash.ToLowerInvariant()
      if ($Actual -ne $Expected) {
        throw "checksum mismatch for ax-$Target; expected $Expected, actual $Actual"
      }
    } else {
      Write-Warning "no SHA256SUMS entry for ax-$Target"
    }
  } catch {
    if ($_.Exception.Message -like "checksum mismatch*") {
      throw
    }
    Write-Warning "SHA256SUMS unavailable; installing without checksum verification"
  }

  Move-Item -Force -Path $Tmp.FullName -Destination $Out
} finally {
  Remove-Item -Force -ErrorAction SilentlyContinue $Tmp.FullName
  Remove-Item -Force -ErrorAction SilentlyContinue $Sums.FullName
}

Write-Host "installed ax to $Out"
Write-Host "add $BinDir to PATH if needed"
