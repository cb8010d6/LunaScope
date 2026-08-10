param(
  [Parameter(Mandatory = $true)]
  [string[]]$Paths
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_CERTIFICATE) -or
    [string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD)) {
  throw "Signing unavailable - stable release gate not satisfied."
}

$certificatePath = Join-Path $env:RUNNER_TEMP "lunascope-release-signing.pfx"
try {
  [IO.File]::WriteAllBytes(
    $certificatePath,
    [Convert]::FromBase64String($env:WINDOWS_SIGNING_CERTIFICATE)
  )
  $sdkRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
  $signTool = Get-ChildItem -LiteralPath $sdkRoot -Filter signtool.exe -Recurse |
    Where-Object { $_.FullName -match "\\x64\\signtool\.exe$" } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
  if (-not $signTool) {
    throw "Windows SDK signtool.exe was not found."
  }
  foreach ($path in $Paths) {
    $resolved = Resolve-Path -LiteralPath $path -ErrorAction Stop
    & $signTool.FullName sign /fd SHA256 /td SHA256 /tr "http://timestamp.digicert.com" /f $certificatePath /p $env:WINDOWS_SIGNING_CERTIFICATE_PASSWORD $resolved.Path
    if ($LASTEXITCODE -ne 0) {
      throw "signtool failed for $($resolved.Path) with exit code $LASTEXITCODE"
    }
    & $signTool.FullName verify /pa /v $resolved.Path
    if ($LASTEXITCODE -ne 0) {
      throw "signature verification failed for $($resolved.Path)"
    }
  }
} finally {
  if (Test-Path -LiteralPath $certificatePath) {
    Remove-Item -LiteralPath $certificatePath -Force
  }
}
