<#
.SYNOPSIS
  Crea un proyecto nuevo desde la plantilla DesdeMovil: repositorio de GitHub,
  Pages, app de Fly.io y clon local, con los parametros anotados en CLAUDE.md.

  Lo que el script genera: el repositorio y la app de Fly.
  Lo que aportas tu: la carpeta de Drive (su id) y la ruta de la carpeta local.
  El script NO crea, mueve ni borra nada en Drive: solo apunta el id.

  Sin parametros pregunta por pantalla el nombre, el id de Drive y la carpeta
  local; con parametros no pregunta nada de lo que ya le hayas dado.

.EXAMPLE
  .\nuevo-proyecto-V1.ps1                              # interactivo: lo pregunta todo
  .\nuevo-proyecto-V1.ps1 inversion -DriveId 1AbC... -Local "D:\Proyectos\inversion" -Simular
  .\nuevo-proyecto-V1.ps1 inversion -DriveId 1AbC... -Local "D:\Proyectos\inversion"
  .\nuevo-proyecto-V1.ps1 inversion -DriveId "https://drive.google.com/drive/folders/1AbC..."
  .\nuevo-proyecto-V1.ps1 pruebax -SinFly              # solo repo + Pages + local
  .\nuevo-proyecto-V1.ps1 pruebax -SinEsperar          # no esperar al build de Rust
  .\nuevo-proyecto-V1.ps1 pruebax -Si -Abrir           # sin preguntar, abre las URLs al acabar
  .\nuevo-proyecto-V1.ps1 -Listar                      # proyectos creados desde este PC
  .\nuevo-proyecto-V1.ps1 pruebax -Eliminar            # delega en eliminar-proyecto-V1.ps1

.NOTES
  Requisitos (una sola vez):
    winget install GitHub.cli ; gh auth login   (scopes: repo, workflow)
    git con credenciales para github.com
    Fly: opcional. Si flyctl esta en el PATH y autenticado, la app se reserva
         ANTES de crear el repo, para no dejar restos si el nombre esta cogido.
         Si no hay flyctl, la crea deploy.yml.
    Drive: la carpeta la creas tu, con el nombre que quieras y en la cuenta que
         quieras; aqui solo pasas su id con -DriveId. Sin -DriveId la fila de
         CLAUDE.md queda vacia y las sesiones omiten el paso de Drive.

  Seguro por defecto:
    - ensena el plan y pide confirmacion antes de tocar nada (-Si lo salta).
    - -Simular ensena el plan y termina sin crear nada.
    - si el repo ya existe y es tuyo, ofrece REANUDAR en vez de fallar.
    - si algo falla a mitad, dice exactamente que quedo a medias y como deshacerlo.
    - verifica por HTTP, al final, que Pages y Fly responden de verdad.
#>
[CmdletBinding()]
param(
  [Parameter(Position = 0)][string]$Nombre = "",
  [string]$Owner        = "npiobject-labs",
  [string]$Plantilla    = "npiobject-labs/DesdeMovil",
  [string]$FlyOrg       = "desdemovil",
  [string]$DriveId      = "",
  [string]$Local        = "",
  [string]$Descripcion  = "",
  [string]$CommitNombre = "",
  [string]$CommitEmail  = "",
  [switch]$SinFly,
  [switch]$SinLocal,
  [switch]$SinEsperar,
  [switch]$Simular,
  [switch]$Si,
  [switch]$Abrir,
  [switch]$Listar,
  [switch]$Eliminar,
  [switch]$Forzar,
  [int]$TimeoutMin      = 10
)

# "Continue" a proposito: en Windows PowerShell 5.1, con "Stop", cualquier linea
# que un comando nativo (gh, git, flyctl) escriba en stderr con 2>&1 se convierte
# en error terminal. Los fallos se comprueban con $LASTEXITCODE.
$ErrorActionPreference = "Continue"
$inicio   = Get-Date
$carpetaR = Join-Path $PSScriptRoot "creados"

function Paso([string]$t) {
  $script:tPaso = Get-Date
  Write-Host ""
  Write-Host "== $t" -ForegroundColor Cyan
}
function Seg() {
  if (-not $script:tPaso) { return "" }
  return " ({0}s)" -f [Math]::Round(((Get-Date) - $script:tPaso).TotalSeconds)
}
function Ok([string]$t)    { Write-Host "   OK  $t$(Seg)" -ForegroundColor Green }
function Aviso([string]$t) { Write-Host "   !!  $t" -ForegroundColor Yellow }
function Nota([string]$t)  { Write-Host "   ..  $t" -ForegroundColor DarkGray }

# Lo que este script ya ha creado, para poder decir que quedo a medias si falla.
$script:hecho = [ordered]@{ fly = $false; repo = $false; local = "" }

function Falla([string]$t) {
  Write-Host "   XX  $t" -ForegroundColor Red
  if ($script:hecho.repo -or $script:hecho.fly -or $script:hecho.local) {
    Write-Host ""
    Write-Host "   Ha quedado a medias:" -ForegroundColor Yellow
    if ($script:hecho.fly)   { Write-Host "     - app de Fly '$flyApp'" }
    if ($script:hecho.repo)  { Write-Host "     - repo https://github.com/$repo" }
    if ($script:hecho.local) { Write-Host "     - carpeta local $($script:hecho.local)" }
    Write-Host ""
    Write-Host "   Para RETOMARLO desde donde se quedo (no borra nada):" -ForegroundColor Cyan
    Write-Host "     $($MyInvocation.ScriptName) $Nombre -DriveId '$DriveId' -Local '$Local'"
    Write-Host "   Para DESHACERLO del todo:" -ForegroundColor Cyan
    Write-Host "     .\eliminar-proyecto-V1.ps1 $Nombre -Confirmar"
  }
  exit 1
}

# gh devuelve JSON; esta funcion lo parsea y aborta si gh fallo.
function GhJson([string[]]$a) {
  $out = & gh @a 2>&1
  if ($LASTEXITCODE -ne 0) { throw "gh $($a -join ' ') -> $out" }
  if (-not $out) { return $null }
  return ($out | Out-String | ConvertFrom-Json)
}

# Espera a que el ultimo run de un workflow (creado despues de $desde) termine.
function Wait-Run([string]$workflow, [datetime]$desde, [int]$minutos = 10) {
  $limite = (Get-Date).AddMinutes($minutos)
  $run = $null
  $puntos = 0
  while ((Get-Date) -lt $limite) {
    # El repo recien creado desde plantilla tarda unos segundos en tener
    # contenido: hasta entonces gh responde 404 "workflow not found".
    try {
      $lista = GhJson @("run","list","-R",$repo,"--workflow",$workflow,"--limit","3",
                        "--json","databaseId,status,conclusion,createdAt,url")
    } catch {
      if ("$_" -match "404|not found") { Nota "esperando a que exista $workflow"; Start-Sleep -Seconds 10; continue }
      throw
    }
    $run = $lista | Where-Object { [datetime]$_.createdAt -ge $desde.AddSeconds(-90) } |
           Sort-Object createdAt -Descending | Select-Object -First 1
    if ($run -and $run.status -eq "completed") { return $run }
    $puntos++
    if ($puntos % 3 -eq 0) {
      $estado = "en cola"; if ($run) { $estado = $run.status }
      Nota "$workflow : $estado ... $([Math]::Round(((Get-Date) - $desde).TotalSeconds))s"
    }
    Start-Sleep -Seconds 10
  }
  if ($run) { throw "Timeout esperando $workflow ($($run.url))" }
  throw "Timeout: no aparecio ningun run de $workflow"
}

# Acepta el id pelado o la URL entera de la carpeta y devuelve solo el id.
function Normaliza-DriveId([string]$v) {
  if (-not $v) { return "" }
  $v = $v.Trim().Trim('"').Trim("'")
  if ($v -match '/folders/([A-Za-z0-9_-]+)')  { $v = $Matches[1] }
  elseif ($v -match '[?&]id=([A-Za-z0-9_-]+)') { $v = $Matches[1] }
  if ($v -cnotmatch '^[A-Za-z0-9_-]{15,}$') {
    Falla "El valor de -DriveId no parece un id de carpeta de Drive: '$v'. Abre la carpeta en Drive y copia el tramo que sigue a /folders/."
  }
  return $v
}

# Convierte lo que hayas escrito en un nombre valido para GitHub y para Fly.
function Sanea-Nombre([string]$v) {
  $x = $v.Trim().ToLowerInvariant()
  $x = $x -replace '[\s_.]+', '-'
  $x = $x -replace '[^a-z0-9-]', ''
  $x = $x -replace '-{2,}', '-'
  return $x.Trim('-')
}

# El PC si tiene salida a internet (la sesion en la nube no), asi que aqui si
# se puede comprobar de verdad que lo desplegado responde.
function Comprueba-Url([string]$url, [string]$contiene = "", [int]$intentos = 3) {
  for ($i = 1; $i -le $intentos; $i++) {
    try {
      $r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 20
      if ($r.StatusCode -eq 200 -and (-not $contiene -or "$($r.Content)" -match [regex]::Escape($contiene))) {
        return $true
      }
    } catch { }
    if ($i -lt $intentos) { Start-Sleep -Seconds 10 }
  }
  return $false
}

# Cabecera: lo primero que se ve al lanzarlo.
function Titulo() {
  Write-Host ""
  Write-Host "  +-------------------------------------------------------+" -ForegroundColor Cyan
  Write-Host "  |  DesdeMovil  ::  crear un proyecto nuevo         (V1)  |" -ForegroundColor Cyan
  Write-Host "  +-------------------------------------------------------+" -ForegroundColor Cyan
  Write-Host "  |  Lo hace el script : repo de GitHub + Pages + Fly.io   |" -ForegroundColor DarkGray
  Write-Host "  |  Lo pones tu       : carpeta de Drive + carpeta local  |" -ForegroundColor DarkGray
  Write-Host "  +-------------------------------------------------------+" -ForegroundColor Cyan
}

# Pregunta con valor por defecto entre corchetes (Enter lo acepta).
# Con -Si no pregunta nada: devuelve el valor por defecto y sigue.
function Pregunta([string]$texto, [string]$porDefecto = "") {
  if ($Si) { return $porDefecto }
  $pista = ""
  if ($porDefecto) { $pista = " [$porDefecto]" }
  $r = Read-Host "   $texto$pista"
  if (-not $r) { return $porDefecto }
  return $r.Trim()
}

Titulo

# ================================================================ MODO LISTAR
if ($Listar) {
  Write-Host ""
  Write-Host "== Proyectos creados desde este PC ($carpetaR)" -ForegroundColor Cyan
  $fichas = @(Get-ChildItem -Path $carpetaR -Filter *.json -ErrorAction SilentlyContinue | Sort-Object Name)
  if ($fichas.Count -eq 0) { Aviso "Ninguno todavia."; exit 0 }
  $fichas | ForEach-Object {
    try { $r = Get-Content $_.FullName -Raw | ConvertFrom-Json } catch { return }
    $fly = $r.fly_url; if (-not $fly) { $fly = "(sin Fly)" }
    $drv = "si"; if (-not $r.drive_id) { $drv = "no" }
    "{0,-16} {1,-46} Drive:{2}  {3}" -f $r.proyecto, $r.pages, $drv, $fly
  } | Write-Host
  exit 0
}

if (-not $Nombre) {
  if ($Si) { Falla "Con -Si hay que dar el nombre: .\nuevo-proyecto-V1.ps1 inversion -Si" }
  Write-Host ""
  Write-Host "== Datos del proyecto" -ForegroundColor Cyan
  Nota "minusculas, digitos y guiones; 15 caracteres maximo (se sanea solo)"
  while (-not $Nombre) { $Nombre = (Read-Host "   Nombre del proyecto").Trim() }
}

# ================================================================ MODO ELIMINAR
# No se duplica la logica de borrado: vive entera en eliminar-proyecto-V1.ps1.
if ($Eliminar) {
  $borrar = Join-Path $PSScriptRoot "eliminar-proyecto-V1.ps1"
  if (-not (Test-Path $borrar)) { Falla "No encuentro '$borrar'. El borrado vive en ese script." }
  $a = @($Nombre, "-Owner", $Owner, "-Confirmar")
  if ($Forzar)   { $a += "-Forzar" }
  if ($SinFly)   { $a += "-SinFly" }
  if ($SinLocal) { $a += "-SinLocal" }
  if ($Local)    { $a += @("-Local", $Local) }
  Nota "delegando en eliminar-proyecto-V1.ps1"
  & $borrar @a
  exit $LASTEXITCODE
}

# ---------------------------------------------------------------- 0. Comprobaciones
Paso "Comprobaciones"

$limpio = Sanea-Nombre $Nombre
if ($limpio -ne $Nombre) {
  if ($Si) { Aviso "Nombre '$Nombre' saneado a '$limpio'" }
  else {
    Write-Host "   El nombre '$Nombre' no vale tal cual (minusculas, digitos y guiones)." -ForegroundColor Yellow
    $r = Read-Host "   Uso '$limpio'? [S/n]"
    if ($r -and $r -notmatch '^[sSyY]') { Falla "Cancelado. Elige otro nombre." }
  }
  $Nombre = $limpio
}
$repo   = "$Owner/$Nombre"
$flyApp = "$Nombre-$Owner"
if ($Nombre -cnotmatch '^[a-z0-9][a-z0-9-]{0,14}$') {
  Falla "Nombre invalido: minusculas, digitos y guiones, 15 caracteres maximo (la app de Fly sera '$flyApp' y Fly corta a 30)."
}

if (-not (Get-Command gh -ErrorAction SilentlyContinue))  { Falla "Falta gh (winget install GitHub.cli)" }
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Falla "Falta git" }
& gh auth status 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { Falla "gh no esta autenticado: gh auth login" }

$flyctl = Get-Command flyctl -ErrorAction SilentlyContinue
if (-not $flyctl -and (Test-Path "$env:USERPROFILE\.fly\bin\flyctl.exe")) {
  $flyctl = Get-Command "$env:USERPROFILE\.fly\bin\flyctl.exe"
}
$registro = Join-Path $carpetaR "$Nombre.json"

# Reanudacion: si el repo ya existe, no es un error fatal, es un reintento.
& gh repo view $repo 2>&1 | Out-Null
$reanudar = ($LASTEXITCODE -eq 0)

if (-not $DriveId) {
  Nota "Drive es opcional y lo creas tu: pega el id o la URL de la carpeta (Enter = sin Drive)"
  $DriveId = Pregunta "Id o URL de la carpeta de Drive"
}
$DriveId = Normaliza-DriveId $DriveId
if (-not $DriveId) { Aviso "Sin Drive: la fila de CLAUDE.md queda vacia y las sesiones omitiran ese paso" }

# La carpeta local se valida AHORA, antes de crear nada en GitHub ni en Fly.
$localTemporal = $false
$localEsClon   = $false
if ($SinLocal) {
  if ($Local) { Aviso "-SinLocal manda: se ignora -Local y el clon se hace en una carpeta temporal" }
  $Local = Join-Path $env:TEMP "np-$Nombre-$(Get-Random)"
  $localTemporal = $true
} else {
  $sugerido = Join-Path $env:USERPROFILE "C - Desarrollo\$Nombre\repo"
  if (-not $Local) { $Local = Pregunta "Carpeta local del proyecto" $sugerido }
  if (-not $Local) { $Local = $sugerido }
  # GetFullPath resolveria contra el directorio del proceso .NET, que no
  # siempre coincide con el de PowerShell; esto usa el de PowerShell.
  $Local = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Local)
  $localEsClon = $false
  if (Test-Path $Local) {
    $origen = & git -C $Local remote get-url origin 2>$null
    if ($LASTEXITCODE -eq 0 -and "$origen" -match [regex]::Escape("$Owner/$Nombre")) {
      $localEsClon = $true
    } elseif (@(Get-ChildItem -Force $Local -ErrorAction SilentlyContinue).Count -gt 0) {
      Falla "La carpeta local '$Local' ya existe y no esta vacia. Elige otra con -Local o vaciala."
    }
  }
}

# ---------------------------------------------------------------- Plan y confirmacion
$txtFly = "$flyApp (en la org '$FlyOrg')"
if ($SinFly)       { $txtFly = "omitida (-SinFly)" }
elseif (-not $flyctl) { $txtFly = "$flyApp (sin flyctl aqui: la creara deploy.yml)" }
$txtDrive = "sin Drive: la fila de CLAUDE.md queda vacia"
if ($DriveId) { $txtDrive = "https://drive.google.com/drive/folders/$DriveId  (solo se apunta el id)" }
$txtLocal = $Local
if ($localTemporal) { $txtLocal = "temporal, se borra al acabar (-SinLocal)" }
elseif ($localEsClon) { $txtLocal = "$Local  (ya es clon de ${repo}: se actualiza)" }

Write-Host ""
Write-Host "== Plan" -ForegroundColor Cyan
Write-Host "   Proyecto     : $Nombre"
Write-Host "   Repo         : https://github.com/$repo  (publico, desde $Plantilla)"
Write-Host "   Pages        : https://$Owner.github.io/$Nombre/"
Write-Host "   App de Fly   : $txtFly"
Write-Host "   Carpeta local: $txtLocal"
Write-Host "   Drive        : $txtDrive"
if ($Owner -ne "npiobject-labs" -and -not $SinFly) {
  Aviso "Owner '$Owner': FLY_API_TOKEN es secreto de la org npiobject-labs. Fuera de ella deploy.yml no desplegara."
}
if ($reanudar) { Aviso "El repo YA EXISTE: se reanuda desde donde se quedo, sin volver a crearlo." }
if ($SinEsperar) { Nota "-SinEsperar: no se espera al build de Rust en Fly." }

if ($Simular) {
  Write-Host ""
  Write-Host "   -Simular: no se ha tocado nada. Quita -Simular para hacerlo." -ForegroundColor Yellow
  exit 0
}
if (-not $Si) {
  $r = Read-Host "`n   Adelante? [S/n]"
  if ($r -and $r -notmatch '^[sSyY]') { Falla "Cancelado. No se ha tocado nada." }
}

# Identidad para el commit de parametros: la del PC si la hay, si no la de gh.
if (-not $CommitNombre) { $CommitNombre = (& git config --global user.name  2>$null) }
if (-not $CommitEmail)  { $CommitEmail  = (& git config --global user.email 2>$null) }
if (-not $CommitNombre) { $CommitNombre = "nuevo-proyecto-V1" }
if (-not $CommitEmail) {
  $login = & gh api user -q .login 2>$null
  if ($LASTEXITCODE -eq 0 -and $login) { $CommitEmail = "$("$login".Trim())@users.noreply.github.com" }
  else { $CommitEmail = "nuevo-proyecto-v1@users.noreply.github.com" }
}

# ---------------------------------------------------------------- 1. Fly (reserva)
# Se reserva el nombre ANTES de crear el repo: si esta cogido por otra cuenta,
# no dejamos un repo a medias que luego hay que borrar.
if (-not $SinFly) {
  Paso "1/7 Fly: reservar la app '$flyApp' en la org '$FlyOrg'"
  if ($flyctl) {
    $f = & $flyctl.Source apps create $flyApp --org $FlyOrg 2>&1
    if ($LASTEXITCODE -eq 0) { $script:hecho.fly = $true; Ok "App creada en Fly" }
    elseif ("$f" -match "already|taken|exists") {
      # Puede ser nuestra (reintento) o ajena. Solo es nuestra si sale en la org.
      $mias = & $flyctl.Source apps list --org $FlyOrg 2>&1
      if ("$mias" -match "\b$([regex]::Escape($flyApp))\b") { Ok "La app ya existia en la org" }
      else { Falla "El nombre '$flyApp' esta cogido por otra cuenta de Fly. Usa otro nombre de proyecto. No se ha creado nada." }
    } else { Aviso "flyctl no pudo crear la app ($f); deploy.yml lo intentara" }
  } else { Aviso "flyctl no disponible: deploy.yml creara la app" }
} else { Paso "1/7 Fly: omitido"; $flyApp = "" }

# ---------------------------------------------------------------- 2. Repo desde plantilla
$tCrear = Get-Date
if ($reanudar) {
  Paso "2/7 Repo: ya existe, no se crea"
  Ok "https://github.com/$repo"
} else {
  Paso "2/7 Crear $repo desde $Plantilla (publico)"
  $crearArgs = @("repo","create",$repo,"--template",$Plantilla,"--public")
  if ($Descripcion) { $crearArgs += @("--description",$Descripcion) }
  & gh @crearArgs | Out-Null
  if ($LASTEXITCODE -ne 0) { Falla "gh repo create fallo" }
  $script:hecho.repo = $true
  Ok "https://github.com/$repo"
}

# ---------------------------------------------------------------- 3. init-plantilla
Paso "3/7 Esperar al workflow 'Inicializar plantilla'"
& gh api "repos/$repo/contents/.plantilla-pendiente" 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0 -and $reanudar) {
  Ok "Ya estaba inicializado"
} else {
  try { $run = Wait-Run "init-plantilla.yml" $tCrear $TimeoutMin } catch { Falla "$_" }
  if ($run.conclusion -ne "success") { Falla "init-plantilla termino en '$($run.conclusion)': $($run.url)" }
  # Doble comprobacion: el marcador tiene que haber desaparecido de main.
  & gh api "repos/$repo/contents/.plantilla-pendiente" 2>&1 | Out-Null
  if ($LASTEXITCODE -eq 0) { Falla "Sigue existiendo .plantilla-pendiente en main" }
  Ok "Inicializado ($($run.url))"
}

# ---------------------------------------------------------------- 4. Pages
Paso "4/7 Pages con origen 'GitHub Actions'"
$r = & gh api -X POST "repos/$repo/pages" -f build_type=workflow 2>&1
if ($LASTEXITCODE -ne 0) {
  # 409: el sitio ya existe (modo rama). Se cambia el origen.
  $r = & gh api -X PUT "repos/$repo/pages" -f build_type=workflow 2>&1
  if ($LASTEXITCODE -ne 0) { Falla "No se pudo configurar Pages: $r" }
}
$tPages = Get-Date
& gh workflow run pages.yml -R $repo --ref main | Out-Null
try { $run = Wait-Run "pages.yml" $tPages $TimeoutMin } catch { Falla "$_" }
if ($run.conclusion -ne "success") { Falla "pages.yml termino en '$($run.conclusion)': $($run.url)" }
$pagesUrl = "https://$Owner.github.io/$Nombre/"
Ok "$pagesUrl"

# ---------------------------------------------------------------- 5. Variable FLY_APP
if (-not $SinFly) {
  Paso "5/7 Variable de repositorio FLY_APP"
  & gh variable set FLY_APP -R $repo -b $flyApp | Out-Null
  if ($LASTEXITCODE -ne 0) { Falla "No se pudo definir la variable FLY_APP" }
  Ok "FLY_APP = $flyApp"
} else { Paso "5/7 Fly: omitido" }

# ---------------------------------------------------------------- 6. Clon local + CLAUDE.md
Paso "6/7 Clonar en '$Local', anotar CLAUDE.md y lanzar el despliegue"
if ($localEsClon) {
  & git -C $Local fetch -q origin main
  & git -C $Local checkout -q main
  & git -C $Local pull -q --rebase origin main
  if ($LASTEXITCODE -ne 0) { Falla "No se pudo actualizar el clon existente en '$Local'" }
  Nota "clon existente actualizado"
} else {
  $padre = Split-Path $Local -Parent
  if ($padre -and -not (Test-Path $padre)) { New-Item -ItemType Directory -Force $padre | Out-Null }
  & git clone -q "https://github.com/$repo.git" $Local
  if ($LASTEXITCODE -ne 0) { Falla "git clone fallo sobre '$Local'" }
  if (-not $localTemporal) { $script:hecho.local = $Local }
}

$claude = Join-Path $Local "CLAUDE.md"
if (-not (Test-Path $claude)) { Falla "El clon no tiene CLAUDE.md: algo fue mal en la plantilla" }
$txt = Get-Content $claude -Raw -Encoding UTF8
$valorDrive = ""; if ($DriveId) { $valorDrive = '`' + $DriveId + '`' }
$txt = [regex]::Replace($txt, '(?m)^(\| Carpeta de Drive \(id\) \|).*$', ('$1 ' + $valorDrive + ' |'))
if ($flyApp) { $txt = [regex]::Replace($txt, '(?m)^(\| App de Fly\.io \|).*$', ('$1 `' + $flyApp + '` |')) }
[IO.File]::WriteAllText($claude, $txt, (New-Object Text.UTF8Encoding $false))

Push-Location $Local
& git add CLAUDE.md
& git -c user.name="$CommitNombre" -c user.email="$CommitEmail" commit -q -m "Anotar parametros: Drive y app de Fly" 2>&1 | Out-Null
$hayCommit = ($LASTEXITCODE -eq 0)
if ($hayCommit) {
  # Por si algun workflow commiteo a main mientras tanto.
  & git pull -q --rebase origin main 2>&1 | Out-Null
  & git push -q -u origin HEAD:main
  if ($LASTEXITCODE -ne 0) { Pop-Location; Falla "git push fallo desde '$Local'" }
}
$sha = (& git rev-parse HEAD).Trim()
Pop-Location
if ($localTemporal) { Remove-Item -Recurse -Force $Local; $Local = "" }
Ok "CLAUDE.md en main ($sha)"

$deployUrl = ""
if (-not $SinFly) {
  $tDeploy = Get-Date
  & gh workflow run deploy.yml -R $repo --ref main | Out-Null
  if ($SinEsperar) {
    Nota "deploy.yml lanzado; no se espera (-SinEsperar). Sigue en https://github.com/$repo/actions"
    $deployUrl = "https://$flyApp.fly.dev/"
  } else {
    Nota "compilando Rust en Fly (varios minutos)"
    try { $run = Wait-Run "deploy.yml" $tDeploy ([Math]::Max($TimeoutMin, 20)) } catch { $run = $null; Aviso "$_" }
    if ($run -and $run.conclusion -ne "success") { Aviso "deploy.yml termino en '$($run.conclusion)': $($run.url)  (mira el resumen del run)" }
    elseif ($run) { $deployUrl = "https://$flyApp.fly.dev/"; Ok "desplegado ($($run.url))" }
  }
}

# ---------------------------------------------------------------- 7. Verificacion real
# Aqui si hay internet (en la sesion de Code no), asi que se comprueba de verdad.
Paso "7/7 Comprobar que responden"
$okPages = Comprueba-Url $pagesUrl
if ($okPages) { Ok "Pages responde: $pagesUrl" }
else { Aviso "Pages aun no responde (suele tardar 1-2 min la primera vez): $pagesUrl" }
$okFly = $null
if ($deployUrl -and -not $SinEsperar) {
  $okFly = Comprueba-Url "${deployUrl}salud" $sha
  if ($okFly) { Ok "Fly responde con el SHA: ${deployUrl}salud" }
  else { Aviso "Fly no devuelve el SHA todavia: ${deployUrl}salud" }
}

# ---------------------------------------------------------------- Resumen
$driveUrl = ""; if ($DriveId) { $driveUrl = "https://drive.google.com/drive/folders/$DriveId" }
$resumen = [ordered]@{
  proyecto = $Nombre; repo = "https://github.com/$repo"; sha = $sha
  pages = $pagesUrl; bitacora = "${pagesUrl}bitacora.html"
  comprobacion = "${pagesUrl}holamundo.html"
  local = $Local
  drive_id = $DriveId; drive_url = $driveUrl
  fly_app = $flyApp; fly_url = $deployUrl
  verificado_pages = $okPages; verificado_fly = $okFly
  code = "https://claude.ai/code  ->  $repo (main)"
  creado = (Get-Date).ToString("s")
  duracion_min = [Math]::Round(((Get-Date) - $inicio).TotalMinutes, 1)
}
New-Item -ItemType Directory -Force $carpetaR | Out-Null
$resumen | ConvertTo-Json | Set-Content $registro -Encoding UTF8

Write-Host ""
Write-Host "================ $Nombre listo en $($resumen.duracion_min) min ================" -ForegroundColor Green
$resumen.GetEnumerator() | ForEach-Object { "{0,-17} {1}" -f $_.Key, $_.Value } | Write-Host

if ($Abrir) {
  Start-Process $resumen.comprobacion
  Start-Process $resumen.repo
  if ($driveUrl) { Start-Process $driveUrl }
}
try { Set-Clipboard -Value $repo -ErrorAction SilentlyContinue } catch { }

Write-Host ""
Write-Host "Siguiente: abre claude.ai/code con $repo seleccionado ('$repo' copiado al portapapeles)."
if (-not $okPages) { Write-Host "Si Pages tarda, reintenta:  .\nuevo-proyecto-V1.ps1 $Nombre -Listar" }
