# prueba — instrucciones del proyecto

Flujo "PC arranca, móvil continúa": el desarrollo, la revisión y las pruebas se hacen desde sesiones en la nube (claude.ai/code con este repo seleccionado, desde web o móvil), con el PC apagado. Trabaja en español. Perfil del usuario: desarrollador senior en solitario; no expliques conceptos básicos; marca toda suposición no verificada como [SUPUESTO] e indica su plan B.

## Parámetros

| Parámetro | Valor |
|---|---|
| Proyecto | `prueba` |
| Owner de GitHub | `npiobject-labs` |
| App de Fly.io | `prueba-npiobject-labs` |
| Carpeta de Drive (id) | `1fK1HO4Ht_3qC990flod_K0hfONUv73xD` |

Esta tabla la rellena sola `.github/workflows/init-plantilla.yml` en el primer push de un repo creado desde la plantilla; no hay nada que tocar a mano salvo el id de Drive.

- **App de Fly.io**: `derivada` significa que `deploy.yml` la calcula como `<repo>-<owner>` en minúsculas, saneado a `[a-z0-9-]` y recortado a 30 caracteres. Si existe la variable de repositorio `FLY_APP`, esa manda; anota aquí el valor cuando la definas.
- **Carpeta de Drive (id)**: vacío significa que este proyecto no usa Drive. Ver ARRANQUE.md para activarlo a mitad de proyecto.

## Fuente de verdad

El repositorio `npiobject-labs/prueba`, rama `main`, es la **única** fuente de verdad, tanto para el código como para la documentación de `docs/planificacion/`. Todo lo que importe vive aquí y se edita aquí.

Google Drive es **opcional** y, cuando está configurado, **solo un destino de copias**, nunca un origen:

- Si el id de la sección **Parámetros** está vacío, este proyecto no usa Drive: omite el paso sin comentarlo.
- Si hay id, al cerrar sesión se suben copias de `docs/planificacion/` a esa carpeta. Solo crear o sobrescribir por nombre: nunca borrar ni renombrar nada en Drive.
- Nunca se toma nada de Drive como origen ni se importa contenido desde allí. Si el repo y Drive difieren, gana el repo.
- La carpeta tiene que ser una carpeta normal de `Mi unidad`. Nunca uses el "Proyecto" de Drive del mismo nombre: el conector no puede escribir en él.

La carpeta local del PC es un espejo de solo lectura. Nunca la trates como origen ni construyas un camino local → nube.

## URLs vivas

| Qué | URL | Despliegue |
|---|---|---|
| Mock estático (Pages) | https://npiobject-labs.github.io/prueba/ | `.github/workflows/pages.yml` en push a `main` |
| Bitácora (Pages) | https://npiobject-labs.github.io/prueba/bitacora.html | idem; el índice lo genera `pages.yml` |
| Backend (Fly.io, opcional) | `https://<app de Fly>.fly.dev/` · `/salud` · `/holamundo` | `.github/workflows/deploy.yml` en push a `main` que toque `app/**` |
| Comprobación del backend (Pages) | https://npiobject-labs.github.io/prueba/holamundo.html | página estática que llama a `/holamundo` y `/salud` desde el navegador |

Pages está siempre activo. Fly también: `FLY_API_TOKEN` es un secreto de la organización `npiobject-labs` y lo heredan sus repos **públicos**, así que `deploy.yml` despliega sin configurar nada. Si el repo fuera privado (plan Free) o viviera fuera de la organización, el secreto no llega y `deploy.yml` termina en verde con el aviso "Fly no configurado" sin desplegar nada.

## Código

- Todo cambio termina en commit + push a `main`. Mensajes de commit en español, imperativo.
- Backend en `app/` (Rust, axum + tokio). `GET /` devuelve texto plano; `GET /salud` devuelve `{"ok":true,"build":"<BUILD_ID>","token":<bool>,"ia":<bool>,"ffmpeg":<bool>}`, donde `BUILD_ID` es el SHA que inyecta el workflow y los tres últimos dicen si hay token de acceso, clave de IA y `ffmpeg` (para transcribir entrevistas).
- `GET /holamundo` devuelve `holamundo` en texto plano; `/holamundo` y `/salud` llevan `Access-Control-Allow-Origin: *` porque los consume `docs/holamundo.html` desde Pages (otro origen). Si añades más rutas para el frontend, ponles la misma cabecera. `deploy.yml` verifica las dos rutas y falla si cambian.
- `docs/holamundo.html` toma el nombre de la app de Fly del `<meta name="fly-app">` (`<repo>-<owner>`, como lo deriva `deploy.yml`). Si el proyecto define `FLY_APP` con otro nombre, actualiza ese `content` en el mismo commit.
- Notas (F1): `GET/POST /notas`, `GET/DELETE /notas/{id}`, `GET /etiquetas`, en SQLite (`rusqlite` bundled). Ruta de la base: `RUTA_DB`, si no `/data/notas.db` cuando existe `/data` (volumen `datos` de Fly, `[mounts]` en `fly.toml`, creado por `deploy.yml`) y si no `./notas.db` en el PC. CORS abierto para todo con `CorsLayer::permissive()`.
- Documentos (F5, D11/D12): `GET/POST /documentos`, `GET/PUT/DELETE /documentos/{id}`, `POST /documentos/{id}/regenerar`. Un documento junta de 1 a 20 notas y la IA redacta con ellas un encargo de trabajo con estructura fija (Contexto, Objetivo, Requisitos, Decisiones y restricciones, Criterios de aceptación, Dudas por resolver); la sección «Notas de origen» la escribe el backend, no el modelo. Se genera **en segundo plano**: `POST` contesta al momento con `estado:"pendiente"` y la app pregunta cada 3 s. Borrar una nota no borra sus documentos: `documento_notas` guarda copia del título y marca `existe:false`. Sin `LLM_API_KEY` la ruta devuelve 503, porque aquí no hay respaldo que valga. En la lista, el modo selección (botón ☑ o pulsación larga) trae **Todas / Ninguna** en la barra inferior: marca o vacía **solo las notas visibles**, así que respeta el filtro de etiqueta y la búsqueda. El tope de 20 sigue mandando: si lo visible lo pasa, se avisa y Generar queda bloqueado.
- Proyectos (F13 a F16, D14 a D26 en `docs/planificacion/plan-proyectos.md`): `GET/POST /proyectos`, `GET/PATCH/DELETE /proyectos/{id}`, `PATCH /notas/{id}` y `POST /notas/mover`; filtro `proyecto=<id>|ninguno` en `/notas`, `/etiquetas` y `/documentos`, y `proyecto` al crear nota. Una nota está en un proyecto como máximo (`notas.proyecto_id`, nullable); `null` es «Sin proyecto». Borrar un proyecto **nunca** borra notas ni documentos. El esquema se versiona con `PRAGMA user_version` (función `migrar` en `main.rs`): un cambio que no quepa en `CREATE ... IF NOT EXISTS` va en una versión nueva. `GET /notas` sin filtro sigue devolviendo todas, para que la app cacheada siga funcionando. En la app, el proyecto activo se recuerda en `localStorage` (`notas-proyecto`) y el orden de la lista de proyectos en `notas-orden-proyectos`. Bajo la cabecera, dos pestañas **📁 Proyectos · 📝 Notas** cambian de pantalla (sustituyen al 📁 de la cabecera): Notas vuelve al proyecto activo o, sin ninguno, enseña todas las notas; dentro de un proyecto, «Editar proyecto» lo gestiona. Crear es igual en las tres pestañas: el botón flotante ＋ de abajo a la derecha (`#nueva`) hace nueva nota, nuevo proyecto o nueva entrevista según la pestaña, y cambia `title`/`aria-label` en `ver()`; en el editor de una nota, el desplegable **Proyecto** parte del proyecto activo, y si se guarda en otro la app entra en ese proyecto.
- Subcarpetas (F23/F24, D43 a D53 en `docs/planificacion/plan-subproyectos.md`): un proyecto puede tener subcarpetas (`proyectos.padre_id`, migración 3), **un solo nivel** (400 si el padre ya es subcarpeta). Una nota, un documento o una entrevista van al proyecto o a una de sus subcarpetas y, si es subcarpeta, traen `proyecto.padre`. `proyecto=<id>` en `/notas`, `/etiquetas`, `/documentos` y `/entrevistas` **incluye las subcarpetas**; `solo=1` las quita («Sin subcarpeta»). `POST/PATCH /proyectos` aceptan `padre` (id o `null`; en `PATCH`, ausente = no se toca); `GET /proyectos` devuelve las subcarpetas como filas con `padre`, y en un principal `notas` suma las de sus subcarpetas (`notas_propias`, `subcarpetas`). En las subcarpetas `nombre_clave` lleva delante el id del padre: el nombre es único dentro de cada padre sin reconstruir la tabla. Archivar es de principales y arrastra a sus subcarpetas. Borrar una subcarpeta sube lo suyo al padre; borrar un principal manda lo suyo a «Sin proyecto» y deja sus subcarpetas como principales (renombradas «X (Padre)» si chocan). En la app: tira **Todo · Sin subcarpeta · 📂 … · ＋ Subcarpeta** bajo la cabecera del proyecto (`#carpetas`), migas «Padre ›» en la cabecera de una subcarpeta (`#migas`), chips 📂 en las tarjetas de Proyectos, «Dentro de» y lista de subcarpetas en la hoja del proyecto, y árbol en «Mover a…» y en el editor (rutas «Padre › Subcarpeta»).
- Entrevistas (F18 a F21, D27 a D42 en `docs/planificacion/plan-entrevista.md`): `GET/POST /entrevistas` (`POST` multipart: `audio`, `proyecto`, `con_quien`, `duracion_s`, `titulo`; hasta 200 MB, volcado a disco según llega), `GET/PUT/DELETE /entrevistas/{id}`, `GET/DELETE /entrevistas/{id}/audio` (con `Range`; borrar solo el audio exige la transcripción hecha), `POST /entrevistas/{id}/transcribir`, `/resumir`, `/nota` (`{cual: ejecutivo|amplio|ambos}`) y `/mover`; filtro `proyecto=` y `textos=1` (lo usa la exportación). El código vive en `app/src/entrevistas.rs`; el audio, en `audio/` junto a la base (en Fly, `/data/audio`). Transcribir: `ffmpeg` (va en la imagen de Docker) trocea en mp3 de 5 min y cada trozo va al modelo como `input_audio` por el mismo proxy (1,2 MB por petición, bajo los 2 MB del proxy); los trozos se guardan en `entrevista_trozos`, así que «Reintentar» sigue desde el que falló. Resumir: una llamada con salida estructurada que da el ejecutivo y el amplio; el markdown del amplio lo compone el backend y el título del modelo no pisa uno editado a mano. Todo en segundo plano con sondeo cada 3 s, como los documentos; al arrancar, lo que estaba a medias pasa a fallido. En la app: tercera pestaña **🎙 Entrevistas** y pantalla de captura a pantalla completa (Grabar/Parar, Pausa, Cerrar) con `MediaRecorder`, vibración, Wake Lock, Media Session con un audio silencioso en bucle (botón de los auriculares) y notificación con Pausa/Parar que `sw.js` reenvía a la página. Lo grabado se guarda en IndexedDB (`notas-grabaciones`) a trozos de 10 s: si la pestaña muere, la pestaña avisa con ⚠️ y la lista ofrece Subir o Descartar. El aviso de consentimiento sale una vez (`notas-consentimiento`). Acceso directo del manifest → `index.html?grabar=1`: con el micro ya permitido arranca solo tras 3 s, y un toque lo cancela. En el detalle, Audio, Transcripción y los dos resúmenes son `details.plegable` con la misma flecha ▾ a la derecha (helper `plegable()`); lo que el usuario pliega a mano se recuerda en `plegEnt` mientras siga en esa entrevista, para que el sondeo no se lo deshaga. **Nunca reutilices ids de la parte de notas**: el detalle usa `ent*` porque `eTitulo` y `eProyecto` son del editor.
- Editar nota: `PUT /notas/{id}` con `{contenido, titulo?}` corrige el texto (título vacío o ausente = se queda el que tenía; proyecto y etiquetas no se tocan). En la app, «✏️ Editar» en el detalle abre el mismo editor con título y dictado (lo dictado se añade detrás). `contenido` es `TEXT` sin límite: una nota que se vea cortada se perdió al dictar, no al guardar. `deploy.yml` verifica el PUT con 6000 caracteres.
- Copiar nota: en el detalle, «📋 Copiar» pone en el portapapeles la nota en markdown (título, proyecto, fecha, etiquetas y contenido), con `execCommand('copy')` de respaldo si no hay `navigator.clipboard`.
- Etiquetas en el buscador (F10): la tira de chips se pliega tras un botón «Etiquetas · N ▾», cerrada por defecto y recordada en `localStorage` (`notas-etiquetas`). Con filtro puesto el botón enseña la etiqueta activa en lugar del recuento: plegado, es lo único que explica por qué la lista viene recortada.
- Búsqueda (F3): `q` va contra la tabla FTS5 `notas_fts` (sin acentos, por prefijo, AND implícito), mantenida por triggers y reconstruida al arrancar si el recuento no cuadra.
- Token (F4, D8): si existe el secreto `TOKEN_API` en Fly, `/notas*` y `/etiquetas` exigen `Authorization: Bearer <token>` (401 si falta); `/salud` y `/holamundo` no. `deploy.yml` lo fija desde el secreto de repositorio `TOKEN_API` (y lo quita si no existe). La app lo guarda en `localStorage` (⚙ Ajustes).
- IA (F2, D5/D6/D9): `app/src/llm.rs` es el **único** sitio que conoce el proveedor; hacia fuera solo ofrece `titular(contenido, etiquetas_existentes)`. Llama al servicio `openrouter` (`npiobject-labs/openrouter`), que habla el formato de la API de OpenAI y factura contra la clave **de esta aplicación**. Al guardar, la nota entra primero con su título de respaldo y `pendiente_ia=1`; si el modelo contesta en menos de 8 s, se reescriben título y etiquetas en una transacción. Si falla, tarda o no hay clave, la nota **se queda como está y no se pierde nunca**, y `POST /notas/{id}/reintentar-ia` la completa después. Al modelo se le pasan las etiquetas ya existentes, más usadas primero, para que reutilice antes de inventar, y se le exige salida estructurada; el título y las etiquetas que devuelve se normalizan antes de tocar la base.
- Secretos y variables de la IA: `LLM_API_KEY` (secreto de repositorio, la clave de aplicación en el servicio; `deploy.yml` la vuelca a Fly y la quita si no está), `LLM_BASE`, `LLM_MODELO`, `LLM_MODELO_DOCUMENTO` y `LLM_MODELO_AUDIO` (variables de repositorio opcionales; el de audio transcribe entrevistas y por defecto es `google/gemini-2.5-flash`; sin ellas valen el servidor y el modelo por defecto). `LLM_MODELO_DOCUMENTO` separa el modelo que redacta documentos del que titula notas: titular es tarea de modelo barato y redactar no; si no se fija, manda `LLM_MODELO`. Sin `LLM_API_KEY` el servicio arranca igual, lo avisa por el log y `GET /salud` lo delata con `"ia":false`. `deploy.yml` lo verifica en cada despliegue: con clave, una nota real tiene que volver titulada y etiquetada; sin ella, `/salud` tiene que declararlo.
- Dictado (D4, arreglado en F8): **nunca `continuous = true`**. Chrome en Android no actualiza el resultado interino en su sitio, va creando uno nuevo con la frase un poco más larga, así que sumar `ev.results` desde 0 escribe la frase repetida palabra a palabra («sí», «sí para», «sí para jugar»…). Se recorre desde `ev.resultIndex`, el interino no se acumula (solo el último), cada trozo final se guarda por separado y uno que solo alarga al anterior lo sustituye, pero **solo si llega dentro de 2 s**: pasado ese tiempo es una frase nueva, y si no, repetir la misma frase a propósito perdería la segunda. El dictado largo sale de rearrancar en `onend`, no de `continuous`.
- Exportar (F11): en ⚙ Ajustes, «Exportar las notas (.md)» descarga **todas** las notas (no lo que haya filtrado) en un markdown con título, fecha, etiquetas y contenido, y ofrece Compartir donde el navegador lo tenga. El sandbox de la sesión no alcanza Fly, así que esta es la vía para sacar los datos: la exportación la hace el navegador del usuario, con su token.
- Tema (F7): el botón 🌗/☀️/🌙 de la cabecera cicla entre automático, claro y oscuro, y guarda la elección en `localStorage` (`notas-tema`). Automático es la ausencia de `data-tema` en `<html>`; los otros dos lo fuerzan. El script del `<head>` que lo aplica antes del primer pintado **tiene que seguir siendo el primero**: sin él se ve el tema del sistema y luego salta. El `<meta name="theme-color">` se recalcula del `--panel` ya resuelto.
- Versión nueva (F12): con la app instalada, una publicación puede tardar en verse porque el service worker ya tiene lo que necesita. La app busca actualización al abrir y al volver a ella, y cuando la hay enseña una banda **«Hay una versión nueva · Actualizar»**; recargar lo decide el usuario, que puede estar escribiendo. En ⚙ Ajustes están el build a la vista y «Buscar actualización» para forzar la comprobación. Si algo publicado no se ve en el móvil, esto es lo primero que hay que mirar.
- PWA (F4): `docs/manifest.webmanifest`, `docs/icono-192.png`, `docs/icono-512.png` y `docs/sw.js`. El service worker cachea la carcasa con el nombre `notas-<build>`: al cambiar el build del mock, cambia también `CACHE` en `sw.js`.
- `app/fly.toml` no lleva clave `app`: el nombre se pasa con `--app` desde `deploy.yml`.
- El backend escucha en 8080, que es lo que espera Fly; la variable de entorno `PUERTO` solo la usa `tools/arrancar.ps1` para probar en el PC.
- Mocks estáticos en `docs/`. `docs/index.html` es el mock vivo; los anteriores se archivan en `docs/mocks/NNN-nombre.html`.
- El índice `docs/mocks/index.html` lo genera `pages.yml` en cada publicación, leyendo el `<title>` y el `<meta name="build">` de cada mock archivado. No lo edites ni lo commitees: está en `.gitignore`.
- Cada mock lleva `<meta name="build" content="PR-B1-AAAAMMDD-NNN">` con un número nuevo en cada iteración.
- Nunca pongas claves, endpoints internos ni datos reales en `docs/`: el sitio es público.

## Documentación

- Cada documento de planificación, decisión o resumen de sesión se escribe en `docs/planificacion/` de este repo, y solo ahí se edita.
- Si existe `docs/plantilla/`, es el historial de la plantilla de origen que apartó `init-plantilla.yml`: referencia de solo lectura, nunca se edita ni se mezcla con `docs/planificacion/`.
- Si hay id de Drive en **Parámetros**, al cerrar sesión se sube copia como fichero, sin conversión a formato Google (`disableConversionToGoogleType=true`), tanto `.md` como `.html/.png/.svg`.
- No hay edición incremental en Drive: se vuelve a subir el fichero completo con el mismo nombre, o con sufijo de versión (`-v2`, `-v3`) si quieres conservar la copia anterior.

## Verificación antes de avisar

**El sandbox de la sesión no alcanza Pages, Fly ni el VPS**: `curl` a `*.github.io`, `*.fly.dev` o al VPS devuelve `CONNECT tunnel failed, response 403`. Tampoco hay daemon de Docker. Por eso **la verificación de un despliegue la hace siempre un workflow**, que corre en el runner de GitHub y sí tiene salida a internet:

- `pages.yml` da por bueno el despliegue con el paso `deploy-pages`, **pero eso solo prueba que el artefacto se subió**, no que se esté sirviendo. Si tras el push aparece además un run `pages build and deployment` con un paso `Build with Jekyll`, el **Source** de Pages sigue en «Deploy from a branch»: el sitio sirve la raíz del repo (README en `/`, `docs/` colgando de `/docs/`) y los runs de `pages.yml` salen verdes sin efecto. Comprobarlo es parte de la verificación; el arreglo es manual, en Settings.
- `deploy.yml` tiene un paso final que hace `curl` a `/salud` y falla el run si la respuesta no contiene el SHA del commit.

No anuncies "puedes probarlo" hasta confirmar por la API de GitHub Actions que el run del workflow para el SHA que acabas de enviar está en `success`. Si en 5 minutos no está, avisa del fallo con la causa leída en los logs, no del éxito. Al avisar, da siempre: SHA, URL y número de `build`.

Si necesitas comprobar algo desde la sesión, hazlo contra la API de GitHub (`https://api.github.com/repos/npiobject-labs/prueba/actions/runs/...`), que sí es accesible.

`pages.yml` solo se puede validar en `main`: el entorno `github-pages` únicamente despliega desde la rama por defecto, así que un `workflow_dispatch` sobre una rama de trabajo no sirve de verificación. `deploy.yml` sí acepta cualquier rama.

## Despliegue

- Estático: GitHub Pages vía `.github/workflows/pages.yml` (push a `main` publica `docs/`). Requiere **Settings → Pages → Source: GitHub Actions** una vez a mano. Se aplica también a este repo: el sitio de la plantilla estuvo sirviendo el README hasta que se hizo.
- Backend (opcional): Fly.io vía `.github/workflows/deploy.yml`. `FLY_API_TOKEN` **llega heredado de la organización `npiobject-labs`** (secreto de organización, repos públicos); no hay que crear ni guardar ningún token por proyecto. Nunca lo imprimas en los logs.
- Si el proyecto usa además un VPS con rama `release`, solo tocas `release` cuando el usuario lo pida explícitamente.
- No intentes SSH, scp, rsync ni curl al VPS, a Fly ni a `*.github.io` desde la sesión: el sandbox los bloquea.

## Aterrizaje en el PC

- Solo a petición y solo con Claude Desktop conectado: `tools/aterrizar.ps1` (idempotente, sobrescribe la copia local sin preguntar). "¿Estoy al día?" = `tools/estado.ps1`. Ambos aceptan `-Proyecto`, `-Owner`, `-Remote`, `-Root` y `-Rama`.
- `tools/arrancar.ps1` levanta la app entera en el PC sin tocar la nube: compila el backend, lo sirve en `localhost:8080` y publica `docs/` en `localhost:8081`. Acepta `-PuertoApi`, `-PuertoWeb`, `-Release` y `-SinNavegador`. Necesita Rust; no necesita Docker.
- `tools/eliminar.ps1` borra el proyecto entero: app de Fly, repositorio y copia local. Sin `-Confirmar` solo enseña el plan; con él pide escribir el nombre. Drive y las sesiones quedan a mano. Solo se ejecuta si el usuario lo pide explícitamente.
- Servidas desde `localhost`, las páginas de `docs/` llaman al backend local en vez de al de Fly, tomando el puerto de `?api=` (8080 por defecto). En Pages no cambia nada.

## Cierre de sesión

- Termina cada sesión con un resumen de 5 líneas (qué cambió, SHA, URL para probar, resultado en Drive, qué falta), guárdalo en el repo en `docs/planificacion/sesiones/AAAAMMDD-HHMM.md` y, si hay id de Drive, sube copia a Drive en `sesiones/`.
- Además, entrada nueva en `docs/bitacora/AAAAMMDD-HHMM.json` (ver sección **Bitácora**).

## Bitácora

Página pública: https://npiobject-labs.github.io/prueba/bitacora.html · formato en `docs/bitacora/README.md`.

BITACORA: al cerrar sesión, además del resumen en docs/planificacion/sesiones/,
crea SIEMPRE un fichero nuevo docs/bitacora/AAAAMMDD-HHMM.json. Nunca edites ni
borres entradas anteriores, y nunca toques docs/bitacora.html ni
docs/bitacora/index.json (lo genera el workflow de Pages).

Campos: fecha (ISO con zona), titulo, objetivo, prompts (array de objetos con
texto y nota opcional), cambios (array), sha, sha_completo, run (id del run de
Actions), mock (ruta relativa al mock archivado de esa sesión), build, pagina,
fly (URL de /salud o null), pendiente (array), enlaces (array de {texto,url}),
notas. Obligatorios: fecha y titulo; el workflow falla si faltan.

Los prompts son una transcripción fiel de lo que pidió el usuario en esa sesión,
en sus términos, no un resumen de lo que hiciste. Si la sesión fue larga y la
transcripción es aproximada, dilo en el campo notas.

docs/ es público: nunca copies a la bitácora prompts que contengan claves,
rutas internas, datos personales o nombres de clientes. Si un prompt los
contiene, resúmelo en su lugar y anótalo en notas.
