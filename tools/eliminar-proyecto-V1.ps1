<#
.SYNOPSIS
  Elimina un proyecto creado con nuevo-proyecto-V1.ps1: app de Fly.io,
  repositorio de GitHub y clon local. La carpeta de Drive NUNCA se toca: el
  script solo te dice cual era y, si se lo pides, te la abre en el navegador.

  Se ejecuta desde cualquier sitio (no hace falta tener el clon delante) y
  SIMULA por defecto: sin -Confirmar no borra nada.

.EXAMPLE
  .\eliminar-proyecto-V1.ps1 inversion                    # plan, no borra nada
  .\eliminar-proyecto-V1.ps1 inversion -Confirmar         # borra, pidiendo el nombre
  .\eliminar-proyecto-V1.ps1 inversion -Confirmar -Forzar # sin preguntar
  .\eliminar-proyecto-V1.ps1 inversion -Confirmar -SinLocal -AbrirDrive

.NOTES
  Requisitos:
    gh autenticado con el scope delete_repo:  gh auth refresh -h github.com -s delete_repo
    flyctl autenticado si el proyecto tiene backend (o pasa -SinFly a sabiendas)

  El inventario sale, por este orden, de:
    1. los parametros que le pases,
    2. el registro local creados\<nombre>.json que deja nuevo-proyecto-V1.ps1,
    3. el propio repo: variable FLY_APP y tabla Parametros de CLAUDE.md.
  Por eso el repositorio se borra EL ULTIMO: mientras existe es quien sabe
  cual es la app de Fly y cual la carpeta de Drive.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory, Position = 0)][string]$Nombre,
  [string]$Owner   = "npiobject-labs",
  [string]$FlyApp  = "",
  [string]$Local   = "",
  [switch]$Confirmar,
  [switch]$Forzar,
  [switch]$SinFly,
  [switch]$SinRepo,
  [switch]$SinLocal,
  [switch]$AbrirDrive,
  [string[]]$Protegidos = @("DesdeMovil")
)

# "Continue" a proposito: en Windows PowerShell 5.1, con "Stop", cualquier linea
# que un comando nativo (gh, git, flyctl) escriba en stderr con 2>&1 se convierte
# en error terminal. Los fallos se comprueban con $LASTEXITCODE.
$ErrorActionPreference = "Continue"
$repo = "$Owner/$Nombre"

function Paso([string]$t) { Write-Host "`n== $t" -ForegroundColor Cyan }
function Ok([string]$t)   { Write-Host "   OK  $t" -ForegroundColor Green }
function Aviso([string]$t){ Write-Host "   !!  $t" -ForegroundColor Yellow }
function Falla([string]$t){ Write-Host "   XX  $t" -ForegroundColor Red; exit 1 }

# Mismo derivado que deploy.yml: minusculas, lo que no sea [a-z0-9-] a guion,
# recorte a 30 y sin guiones al final. Ni colapsa guiones repetidos ni quita
# los del principio, igual que el workflow.
function Derivar-FlyApp([string]$proyecto, [string]$owner) {
  $a = ("$proyecto-$owner").ToLowerInvariant() -replace '[^a-z0-9-]', '-'
  if ($a.Length -gt 30) { $a = $a.Substring(0, 30) }
  return ($a -replace '-*$', '')
}

# ---------------------------------------------------------------- 0. Guardas
Paso "Comprobaciones"
if ($Protegidos -contains $Nombre) {
  Falla "'$Nombre' esta en la lista de protegidos (la plantilla y similares). Si de verdad quieres borrarlo: -Protegidos @()"
}
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Falla "Falta gh (winget install GitHub.cli)" }
& gh auth status 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { Falla "gh no esta autenticado: gh auth login" }

$flyctl = Get-Command flyctl -ErrorAction SilentlyContinue
if (-not $flyctl -and (Test-Path "$env:USERPROFILE\.fly\bin\flyctl.exe")) {
  $flyctl = Get-Command "$env:USERPROFILE\.fly\bin\flyctl.exe"
}
$registro = Join-Path $PSScriptRoot "creados\$Nombre.json"

# ---------------------------------------------------------------- 1. Inventario
Paso "1/5 Inventario de '$Nombre'"
& gh repo view $repo 2>&1 | Out-Null
$hayRepo = ($LASTEXITCODE -eq 0)
$driveId = ""

if (Test-Path $registro) {
  try {
    $reg = Get-Content $registro -Raw | ConvertFrom-Json
    if (-not $FlyApp -and $reg.fly_app) { $FlyApp = "$($reg.fly_app)".Trim() }
    if (-not $Local   -and $reg.local)   { $Local   = "$($reg.local)".Trim() }
    if ($reg.drive_id) { $driveId = "$($reg.drive_id)".Trim() }
    Ok "Registro local: $registro"
  } catch { Aviso "El registro '$registro' no se pudo leer: $($_.Exception.Message)" }
}

if ($hayRepo) {
  $v = & gh variable get FLY_APP -R $repo 2>$null
  if ($LASTEXITCODE -eq 0 -and $v -and -not $FlyApp) { $FlyApp = "$v".Trim() }
  $cm = & gh api "repos/$repo/contents/CLAUDE.md" -H "Accept: application/vnd.github.raw" 2>$null
  if ($LASTEXITCODE -eq 0 -and "$cm" -match '(?m)^\| Carpeta de Drive \(id\) \|\s*`?([A-Za-z0-9_-]{10,})`?\s*\|') { $driveId = $Matches[1] }
  Ok "Repo encontrado: https://github.com/$repo"
} else {
  Aviso "El repositorio $repo no existe (o gh no lo ve). Se sigue con el resto."
}

if (-not $FlyApp) { $FlyApp = Derivar-FlyApp $Nombre $Owner }

# Candidatas de carpeta local, en orden, si no se dio ninguna.
if (-not $Local) {
  foreach ($c in @((Join-Path $env:USERPROFILE "C - Desarrollo\$Nombre\repo"),
                   (Join-Path $env:USERPROFILE "C - Desarrollo\$Nombre"))) {
    if (Test-Path $c) { $Local = $c; break }
  }
}
if ($Local) { $Local = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Local) }

# Que dice Fly de esa app: existe, no existe, o no se puede saber.
$estadoFly = "desconocido (sin flyctl)"
if ($flyctl) {
  $lista = & $flyctl.Source apps list 2>&1
  if ($LASTEXITCODE -ne 0) { $estadoFly = "desconocido (flyctl fallo: revisa flyctl auth login)" }
  elseif ("$lista" -match "\b$([regex]::Escape($FlyApp))\b") { $estadoFly = "existe" }
  else { $estadoFly = "no aparece en tus apps" }
}

# El clon local solo se borra si de verdad apunta a ESTE repo.
$localBorrable = $false
$motivoLocal   = "no hay carpeta"
if ($Local -and (Test-Path $Local)) {
  $origen = & git -C $Local remote get-url origin 2>$null
  if ($LASTEXITCODE -eq 0 -and "$origen" -match [regex]::Escape("$Owner/$Nombre")) {
    $localBorrable = $true; $motivoLocal = "clon de $repo"
  } elseif ($LASTEXITCODE -eq 0) {
    $motivoLocal = "OJO: es un clon de otro repo ($origen). No se toca."
  } elseif ($Forzar) {
    $localBorrable = $true; $motivoLocal = "no es un repo git; se borra por -Forzar"
  } else {
    $motivoLocal = "no es un repo git. Usa -Forzar si aun asi quieres borrarla."
  }
}

# ---------------------------------------------------------------- 2. Plan
$driveUrl = ""; if ($driveId) { $driveUrl = "https://drive.google.com/drive/folders/$driveId" }
Paso "2/5 Plan"
$txtRepo = 'no existe'; if ($hayRepo) { $txtRepo = "https://github.com/$repo" }
if ($SinRepo) { $txtRepo += '  (omitido por -SinRepo)' }
$txtFly = "$FlyApp  [$estadoFly]"; if ($SinFly) { $txtFly += '  (omitida por -SinFly)' }
$txtLocal = 'ninguna'; if ($Local) { $txtLocal = "$Local  [$motivoLocal]" }
if ($SinLocal) { $txtLocal += '  (omitida por -SinLocal)' }
$txtDrive = 'ninguna conocida'; if ($driveUrl) { $txtDrive = $driveUrl }
Write-Host "   1. Fly.io    : $txtFly"
Write-Host "   2. Local     : $txtLocal"
Write-Host "   3. GitHub    : $txtRepo"
Write-Host "   4. Registro  : $registro"
Write-Host "   5. A mano    : Drive $txtDrive" -ForegroundColor Yellow
Write-Host "                  sesiones de https://claude.ai/code" -ForegroundColor Yellow

if (-not $hayRepo -and $estadoFly -ne "existe" -and -not $localBorrable) {
  Ok "No queda nada que borrar de '$Nombre'."
  if ($driveUrl) { Aviso "Salvo la carpeta de Drive, que va a mano: $driveUrl" }
  exit 0
}

if (-not $Confirmar) {
  Write-Host "`nSimulacion: no se ha borrado nada. Repite con -Confirmar para hacerlo de verdad." -ForegroundColor Cyan
  exit 0
}
if (-not $Forzar) {
  Write-Host "`nEsto NO se puede deshacer." -ForegroundColor Red
  $conf = Read-Host "   Escribe el nombre del proyecto para confirmar"
  if ($conf -cne $Nombre) { Falla "El nombre no coincide. No se ha borrado nada." }
}

$pendientes = @()

# ---------------------------------------------------------------- 3. Fly
# Primero: mientras el repo viva, si algo sale mal sigue sabiendose el nombre
# de la app. Al reves, una app huerfana no la recuerda nadie.
if (-not $SinFly) {
  Paso "3/5 Fly: destruir '$FlyApp'"
  if ($flyctl) {
    $f = & $flyctl.Source apps destroy $FlyApp --yes 2>&1
    if ($LASTEXITCODE -eq 0) { Ok "App destruida" }
    elseif ("$f" -match "not find|not found|Could not") { Ok "La app no existia" }
    else { Falla "flyctl no pudo destruir '$FlyApp': $f  (nada mas se ha borrado)" }
  } else {
    Falla "Sin flyctl no se puede destruir la app de Fly. Instala/autentica flyctl, o repite con -SinFly a sabiendas de que la app queda viva. Nada se ha borrado."
  }
} else {
  Paso "3/5 Fly: omitido"
  if ($estadoFly -eq "existe") { $pendientes += "Fly: la app $FlyApp sigue viva -> https://fly.io/dashboard (Settings -> Delete app)" }
}

# ---------------------------------------------------------------- 4. Local
if (-not $SinLocal) {
  Paso "4/5 Carpeta local"
  if ($localBorrable) {
    # Si el script se lanzo desde dentro de esa carpeta, hay que salir antes.
    if ((Get-Location).Path -like "$Local*") { Set-Location $env:USERPROFILE }
    Remove-Item -LiteralPath $Local -Recurse -Force
    Ok "Borrada: $Local"
  } elseif ($Local) {
    Aviso "No se borra '$Local': $motivoLocal"
    $pendientes += "Local: revisa y borra tu '$Local' ($motivoLocal)"
  } else { Ok "No habia carpeta local" }
} else { Paso "4/5 Carpeta local: omitida" }

# ---------------------------------------------------------------- 5. GitHub
if (-not $SinRepo -and $hayRepo) {
  Paso "5/5 GitHub: borrar $repo"
  $g = & gh repo delete $repo --yes 2>&1
  if ($LASTEXITCODE -ne 0) {
    if ("$g" -match "delete_repo") { Falla "gh necesita el scope delete_repo:  gh auth refresh -h github.com -s delete_repo   (Fly y la carpeta local ya se han borrado)" }
    Falla "gh repo delete fallo: $g"
  }
  Ok "Repositorio borrado, con su sitio de Pages, variables y runs"
} else { Paso "5/5 GitHub: omitido" }

if (Test-Path $registro) { Remove-Item $registro -Force; Ok "Registro local borrado" }

# ---------------------------------------------------------------- Verificacion
Paso "Verificacion"
& gh repo view $repo 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) {
  if ($SinRepo) { Aviso "El repositorio sigue ahi (lo pediste con -SinRepo)" }
  else { Aviso "El repositorio SIGUE existiendo: revisalo en https://github.com/$repo/settings" }
} else { Ok "GitHub: $repo ya no existe" }

if ($flyctl -and -not $SinFly) {
  $lista = & $flyctl.Source apps list 2>&1
  if ($LASTEXITCODE -eq 0 -and "$lista" -match "\b$([regex]::Escape($FlyApp))\b") { Aviso "Fly: '$FlyApp' SIGUE apareciendo en tus apps" }
  elseif ($LASTEXITCODE -eq 0) { Ok "Fly: '$FlyApp' ya no aparece" }
}
if ($Local -and (Test-Path $Local)) { Aviso "Local: '$Local' sigue ahi" }
elseif ($Local) { Ok "Local: carpeta borrada" }

# ---------------------------------------------------------------- Pendientes
if ($driveUrl) {
  $pendientes += "Drive: manda a la papelera la carpeta $driveUrl (el script no toca Drive)"
  if ($AbrirDrive) { Start-Process $driveUrl }
} else {
  $pendientes += "Drive: si el proyecto tenia carpeta, borrala tu (no se pudo averiguar su id)"
}
$pendientes += "Claude: archiva o borra las sesiones del proyecto en https://claude.ai/code"

Write-Host "`nQueda por hacer a mano:" -ForegroundColor Yellow
foreach ($p in $pendientes) { Write-Host "  - $p" }
Write-Host "`n================ '$Nombre' eliminado ================" -ForegroundColor Green
