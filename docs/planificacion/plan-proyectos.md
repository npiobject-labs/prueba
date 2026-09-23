# Plan — notas por proyectos

Fecha: 2026-09-23 · Estado: v2, **F13 a F16 hechas** (backend, pantalla de proyectos, mover, documentos y exportación por proyecto); F17 opcional sin hacer · Continúa `plan-notas.md` (F0 a F12) · Fuente de verdad: este fichero (la copia en Drive es solo copia).

## 1. Qué se pide (notas del usuario, en sus términos)

1. Además de las etiquetas, las notas se han de poder ir guardando **por proyectos**: se abre un proyecto y ahí se van poniendo notas.
2. Los distintos proyectos se han de poder **buscar, ordenar y demás**.
3. Es una mejora sustancial en la utilización. En esta sesión, solamente planificación.
4. (Misma sesión, después) «Ahora lo que quiero es que desarrolles ese plan esa planificación que has hecho y lo despliegues para probar cómo funciona».

## 2. Qué es un proyecto aquí

Un proyecto es un **contenedor** de notas, no una etiqueta más. Las etiquetas las inventa la IA, son muchas y una nota lleva varias; el proyecto lo nombra el usuario, son pocos y una nota está en uno. Las dos cosas se cruzan: dentro de un proyecto se sigue filtrando por etiqueta, buscando por palabra y por día, seleccionando notas y generando documentos, exactamente como hoy, pero sobre las notas de ese proyecto.

La app deja de abrirse en «todas las notas» y pasa a abrirse en el **último proyecto que se estaba usando**. Desde ahí, un toque lleva a la pantalla de proyectos, que es donde se busca, se ordena, se crea y se archiva.

## 3. Decisiones y supuestos

Numeración continua con `plan-notas.md` (última: D13).

| # | Decisión | Estado |
|---|---|---|
| D14 | **Una nota está en un proyecto como máximo** (`notas.proyecto_id`, nullable). Compartir una nota entre proyectos es lo que ya resuelven las etiquetas; duplicar el mecanismo confundiría. | [SUPUESTO] No hace falta N:M. Plan B: tabla `nota_proyectos` con la misma API (`proyecto` pasaría a lista); la columna se dejaría de usar sin migrar nada más. |
| D15 | **Proyecto activo**: la app trabaja siempre dentro de un proyecto. La cabecera enseña su nombre en lugar de «Notas», la lista trae solo sus notas, el buscador y los chips de etiquetas son los de ese proyecto y «Nueva nota» la crea dentro. Se recuerda en `localStorage` (`notas-proyecto`); si al abrir no existe (borrado desde otro sitio), se va a la pantalla de proyectos. | Decidido. Es la lectura literal de «se abre un proyecto y ahí se van poniendo notas». |
| D16 | **«Sin proyecto»** es un contenedor más, no creable ni borrable, que agrupa las notas con `proyecto_id NULL`. Todas las notas de hoy caen ahí en la migración, así que el día que se despliegue nada desaparece: se ven donde siempre y se van moviendo a proyectos con calma. Al guardar, la nota nunca exige proyecto. | [SUPUESTO] El usuario no quiere que crear nota obligue a elegir proyecto. Plan B: aviso «Vas a guardar sin proyecto» con selector, sin bloquear. |
| D17 | **Nombre puesto por el usuario**, no por la IA. Es la única cosa de la app que el usuario nombra a propósito, y es correcto: la IA pone título a lo que ya está dicho; el proyecto es una intención previa. Nombre obligatorio, único sin distinguir mayúsculas (409 si se repite), hasta 60 caracteres; descripción opcional. Sin color ni icono en esta versión. | Decidido. [SUPUESTO] No hace falta color/icono para distinguirlos: el nombre basta con pocos proyectos. Plan B: columna `color` con paleta fija, un chip de color en la lista. |
| D18 | **Buscar y ordenar proyectos** (`GET /proyectos?q=&orden=&archivados=`). Búsqueda por nombre y descripción con `LIKE` sobre texto sin acentos: son decenas de filas, no miles, y FTS sobraría. Orden por defecto **actividad** (fecha de la nota más reciente, y si no tiene, la creación del proyecto); alternativas `nombre`, `creacion` y `notas` (recuento). La elección se recuerda en `localStorage` (`notas-orden-proyectos`). | Decidido. [SUPUESTO] Con decenas de proyectos `LIKE` va sobrado. Plan B: `proyectos_fts` calcada de `notas_fts` si el recuento pasa de unos cientos. |
| D19 | **Archivar antes que borrar.** Un proyecto acabado se archiva: desaparece de la lista principal (queda tras un desplegable «Archivados · N», como las etiquetas de F10), sus notas siguen intactas y se puede desarchivar. Borrar existe, pide confirmación y dice cuántas notas se van a quedar sin proyecto: **borrar un proyecto nunca borra notas** (`ON DELETE SET NULL`). | Decidido. Misma filosofía que D12: nada del usuario se pierde por una acción sobre otra cosa. |
| D20 | **Mover notas** entre proyectos, de una en una desde el detalle («Mover a…», hoja con la lista de proyectos, «Sin proyecto» y «Nuevo proyecto») y **en bloque** desde el modo selección (F9), que gana un botón «Mover». Es la vía para repartir las notas de hoy. Mover no toca título, etiquetas ni fecha. | Decidido. |
| D21 | **Documentos y proyectos.** El modo selección opera dentro del proyecto activo, así que un documento sale normalmente de notas del mismo proyecto. `documentos.proyecto_id` guarda cuál, y el nombre y la descripción del proyecto se le pasan al modelo como contexto: es justo el plan B que D11 dejó apuntado («campo proyecto fijo que se antepone»), resuelto sin campo en Ajustes. Con «buscar en todos» (D22) pueden mezclarse notas de varios proyectos: se permite y el documento va sin contexto de proyecto, como hoy. La pantalla de documentos se filtra por el proyecto activo, con «todos» a mano. | Decidido. `documento_notas` no cambia. |
| D22 | **Búsqueda dentro y fuera.** `q`, etiqueta y día filtran dentro del proyecto activo. Un conmutador «En todos los proyectos» en el buscador, apagado por defecto y no recordado, hace la misma búsqueda contra toda la base; en ese modo cada nota de la lista lleva un chip con su proyecto. | Decidido. |
| D23 | **Migración con versión.** Hoy el esquema es todo `CREATE ... IF NOT EXISTS`; añadir columnas exige `PRAGMA user_version`: 0 → 1 crea `proyectos` y añade `proyecto_id` a `notas` y `documentos`. Idempotente y dentro de una transacción. `GET /notas` **sin** `proyecto` sigue devolviendo todas: la app antigua que el service worker tenga cacheada (F12) sigue funcionando contra el backend nuevo hasta que el usuario pulse «Actualizar». | Decidido. Nada de lo actual cambia de forma ni de código de respuesta. |
| D24 | **Orden de las notas dentro del proyecto**: por fecha, la más reciente arriba, como hoy. Sin orden manual. | [SUPUESTO] No hace falta arrastrar notas. Plan B: columna `orden` y `PATCH /notas/{id} {orden}`. |
| D25 | **Exportación (F11)** agrupada por proyecto (`## Proyecto (N)` → `### Nota`, bajo el `# Notas` de siempre), «Sin proyecto» al final. Y desde la pantalla de un proyecto, «Exportar este proyecto» con el mismo formato y solo sus notas. | Decidido. |
| D26 | **Fuera de esta versión**: subproyectos, compartir proyectos, orden manual, color, IA que sugiera proyecto al guardar. Lo último es lo único tentador (una nota que dice «para la app de notas» podría ir sola a su proyecto), y queda como F17 opcional. | Decidido. |

## 4. Modelo de datos

```sql
-- user_version 0 -> 1
CREATE TABLE proyectos (
  id             TEXT PRIMARY KEY,                    -- ulid
  nombre         TEXT NOT NULL,
  nombre_clave   TEXT NOT NULL UNIQUE,                -- nombre en minúsculas y sin acentos (D17)
  descripcion    TEXT NOT NULL DEFAULT '',
  archivado      INTEGER NOT NULL DEFAULT 0,
  creado_en      TEXT NOT NULL,                       -- ISO-8601 UTC
  actualizado_en TEXT NOT NULL                        -- última edición del propio proyecto
);
ALTER TABLE notas      ADD COLUMN proyecto_id TEXT REFERENCES proyectos(id) ON DELETE SET NULL;
ALTER TABLE documentos ADD COLUMN proyecto_id TEXT REFERENCES proyectos(id) ON DELETE SET NULL;
CREATE INDEX notas_proyecto      ON notas(proyecto_id, creada_en DESC);
CREATE INDEX documentos_proyecto ON documentos(proyecto_id, creado_en DESC);
PRAGMA user_version = 1;
```

- `nombre_clave` se calcula en el backend con la misma normalización que ya usan las etiquetas; `COLLATE NOCASE` no vale porque solo ignora mayúsculas ASCII.
- «Actividad» y «recuento» no se guardan: se calculan en la consulta (`MAX(creada_en)` y `COUNT(*)` de `notas` por `proyecto_id`). Con decenas de proyectos y cientos de notas, es inmediato.
- `notas_fts` no cambia: la búsqueda por proyecto es un `AND n.proyecto_id = ?` más en `listar_notas`, junto a los filtros que ya hay.
- `ALTER TABLE ... ADD COLUMN ... REFERENCES` es válido en SQLite porque el valor por defecto es `NULL`.

## 5. API (todas con CORS abierto y bajo el token de D8, como `/notas*`)

| Método y ruta | Qué hace |
|---|---|
| `GET /proyectos?q=&orden=actividad|nombre|creacion|notas&archivados=0|1` | Lista con `notas` (recuento), `ultima_nota` (fecha o `null`) y `archivado`. Sin `archivados=1` no salen los archivados. Incluye siempre una fila virtual `{id:null, nombre:"Sin proyecto", notas:N}` si N > 0, para que la app no tenga que pedirla aparte. |
| `POST /proyectos` `{nombre, descripcion?}` | 201 con el proyecto. 400 sin nombre o con más de 60 caracteres; 409 si `nombre_clave` ya existe. |
| `GET /proyectos/{id}` | Uno, con recuento y última nota. |
| `PATCH /proyectos/{id}` `{nombre?, descripcion?, archivado?}` | Renombrar, describir, archivar o desarchivar. Mismos 400/409 que crear. |
| `DELETE /proyectos/{id}` | 204. Sus notas y documentos pasan a `proyecto_id NULL` (D19). |
| `GET /notas?proyecto=<id>|ninguno` | Nuevo filtro, combinable con `q`, `etiqueta`, `desde`, `hasta`. Sin él, todas (D23). Cada nota trae `proyecto: {id, nombre}` o `null`. |
| `POST /notas` `{contenido, proyecto?}` | Guarda dentro del proyecto. 400 si el proyecto no existe. |
| `PATCH /notas/{id}` `{proyecto: <id>|null}` | Mueve una nota (D20). |
| `POST /notas/mover` `{notas[], proyecto: <id>|null}` | Mueve varias en una transacción. 400 si alguna no existe. Devuelve el recuento. |
| `GET /etiquetas?proyecto=<id>|ninguno` | Etiquetas con recuento **dentro** del proyecto, para los chips. Sin él, todas. |
| `POST /documentos` `{notas[], instruccion?}` | Sin campo nuevo: el backend deduce `proyecto_id` si todas las notas comparten uno y pasa nombre y descripción al modelo (D21). Si no lo comparten, `null` y sin contexto. |
| `GET /documentos?proyecto=<id>|ninguno` | Nuevo filtro; sin él, todos. Cada documento trae `proyecto` como las notas. |

`deploy.yml` crece con un paso «Verificar /proyectos»: crear proyecto, nota dentro, `GET /notas?proyecto=` la encuentra y sin filtro también, `q` dentro del proyecto, mover a «ninguno», `DELETE` del proyecto y comprobar que la nota sigue existiendo sin proyecto, y el 409 del nombre repetido.

## 6. Pantallas (mock 13 en adelante)

1. **Proyectos** (nueva; es adonde lleva el botón ◧ de la cabecera y adonde se abre la app la primera vez). Buscador de proyectos; selector de orden (actividad · nombre · creación · notas); tarjeta por proyecto con nombre, recuento, «hace N días» de la última nota y la descripción en una línea; «Sin proyecto · N» arriba si hay huérfanas; «Archivados · N ▾» plegado al final; botón `+` que abre una hoja con nombre y descripción y entra en el proyecto recién creado. Pulsación larga sobre una tarjeta: renombrar, archivar, borrar.
2. **Notas** (la pantalla actual, ahora dentro de un proyecto). En `#cab` el nombre del proyecto en lugar de «Notas», con un botón ◧ delante para volver a Proyectos. Todo lo demás igual, pero acotado al proyecto: lista, búsqueda, chips, día, modo selección, «Nueva nota». En el buscador, conmutador «En todos» (D22). Cabecera de proyecto plegable con la descripción y «Exportar este proyecto».
3. **Detalle de nota**: chip del proyecto bajo la fecha; «Mover a…» junto a Borrar.
4. **Modo selección**: la barra pasa de tres botones a cuatro (Todas/Ninguna · Mover · Generar · Cancelar). Ya va en dos filas desde F9; hay que reprobar a 390 px que no desborde. «Mover» abre la misma hoja que el detalle.
5. **Documentos**: filtrados por el proyecto activo, con «Todos» arriba; cada documento enseña su proyecto.
6. **Ajustes**: la exportación completa agrupa por proyecto (D25).

La primera vez tras desplegar, el usuario aterriza en Proyectos con solo «Sin proyecto · N»: crea el primero, entra, y desde selección mueve en bloque lo que le corresponda. No hay asistente de migración y no hace falta.

## 7. Fases

| Fase | Entregable | Verificación |
|---|---|---|
| **F13** ✅ | Este plan. | El usuario pidió implementarlo sin contestar la sección 8: se siguió con lo supuesto. |
| **F14** ✅ | Backend: migración con `user_version` (D23), tabla `proyectos`, las cinco rutas de `/proyectos`, filtro `proyecto` en `/notas`, `/etiquetas` y `/documentos`, `proyecto` al crear nota, `PATCH /notas/{id}` y `POST /notas/mover`, contexto de proyecto en `documentar()` (D21). Compatible con la app actual. | Paso «Verificar /proyectos» en `deploy.yml` (sección 5). Antes, en local contra SQLite: migrar una base con notas y documentos existentes y comprobar que todo queda en «Sin proyecto». |
| **F15** ✅ | App, parte 1 (mock 13, `PR-B1-20260923-015`): pantalla Proyectos con buscar, ordenar, crear, renombrar, archivar y borrar; proyecto activo recordado; cabecera con nombre y vuelta; «Nueva nota» dentro del proyecto; lista, búsqueda y chips acotados; conmutador «En todos». | Hecho: 37 comprobaciones con Chromium a 390×844 contra el backend real en local (base migrada desde el esquema de `main`) y una IA simulada, cubriendo F15 y F16: crear y entrar, orden por cada criterio, búsqueda de proyectos, archivar y desplegar archivados, borrar con las notas pasando a «Sin proyecto», arranque con proyecto recordado y con proyecto borrado, «En todos» con chip de proyecto en cada nota. Sin errores de consola. |
| **F16** ✅ | App, parte 2 (en el mismo mock 13: salieron en la misma sesión): «Mover a…» en el detalle y «Mover» en selección; documentos filtrados por proyecto y con su chip; exportación agrupada y por proyecto. | Chromium: mover una y varias (con y sin filtro activo, como F9), selección con cuatro botones sin desbordar a 390 px, documento generado dentro de un proyecto trae `proyecto`, `.md` exportado con una cabecera por proyecto y «Sin proyecto» al final. |
| **F17** (opcional, D26) | La IA propone proyecto al guardar una nota sin proyecto o desde «Sin proyecto» («¿Va a *X*?», un toque para aceptar). Sale de pasar al modelo los nombres y descripciones de los proyectos activos igual que hoy se le pasan las etiquetas. | Solo si el usuario lo pide tras usar F15/F16. |

F14 va primero y se puede mergear solo: no cambia nada visible y deja la base lista. F15 y F16 pueden ir en un mismo PR si salen en la misma sesión; la división es por si no.

Cada fase termina con push, verificación por workflow, entrada en bitácora y resumen de sesión, como marca `CLAUDE.md`.

## 8. Preguntas al usuario (antes de F14)

Todas tienen respuesta supuesta; con silencio, se sigue con ella.

1. ¿Una nota en un solo proyecto (D14)? Supuesto: sí.
2. ¿Guardar una nota sin proyecto es válido (D16)? Supuesto: sí, va a «Sin proyecto».
3. ¿Orden por defecto de los proyectos por actividad (D18)? Supuesto: sí.
4. ¿Hace falta color o icono por proyecto (D17)? Supuesto: no.
5. ¿Quiere que la IA proponga proyecto (F17)? Supuesto: no de momento.

## 9. Riesgos

- **Migración en producción**: `deploy.yml` despliega sobre el volumen con las notas reales. La migración es aditiva (columnas nullable y tabla nueva), pero conviene descargar antes la exportación de F11 como copia: es la única vía de sacar los datos, porque el sandbox no llega a Fly.
- **App cacheada**: tras desplegar F14, el móvil puede seguir con el mock 12 hasta que acepte «Actualizar» (F12). Funciona igual, porque las rutas viejas no cambian (D23).
- **Barra de selección con cuatro botones** a 390 px: probablemente haga falta un icono en lugar de texto en alguno. Se decide en F16 con la captura delante.

## 10. Cómo quedó (2026-09-23)

- **Migración**: probada creando la base con el binario de `main` y arrancando el nuevo encima: `user_version` pasa a 1, las notas quedan en «Sin proyecto» y un segundo arranque no vuelve a migrar.
- **`PATCH /notas/{id}`**: `proyecto` ausente o `null` deja la nota en «Sin proyecto». Hoy es el único campo; si un día se añaden más, habrá que distinguir ausente de `null`.
- **Búsqueda de proyectos**: se leen todos y se filtran en el backend con la misma normalización que el nombre (minúsculas, sin acentos), por nombre y descripción, AND de palabras.
- **Cabecera en el móvil**: el texto «mock 13 · host» se oculta por debajo de 430 px para que quepa el nombre del proyecto; los avisos y errores se siguen viendo.
- **Barra de selección**: los cuatro botones (Todas · Mover · Cancelar · Generar) caben a 390 px sin iconos.
