# Plan: subcarpetas (subproyectos) dentro de los proyectos

Fecha: 2026-09-24 · Estado: v2, **F23 y F24 hechas** (mock de diseño: `docs/mocks/019-subproyectos.html`, build `PR-B1-20260924-025`; app: mock 19, build `PR-B1-20260924-026`) · Continúa `plan-proyectos.md` (D14 a D26, que dejó los subproyectos fuera en D26) y `plan-entrevista.md` (hasta D42) · Fuente de verdad: este fichero.

## 1. Qué se pide (en palabras del usuario)

> Quiero modificar la parte de proyectos en notas de forma que se puedan añadir subcarpetas a cada una de las carpetas del proyecto, o dicho de otra forma subproyectos a cada uno de los proyectos, que sería lo mismo, de forma que luego cada nota se pueda asignar a la carpeta padre o a cualquiera de sus hijos. Hazme un mock para ver cómo quedaría, no desarrolles.

> (Después de ver el mock) Uno sí, dos sí, tres sí, cuatro subcarpeta, adelante con el desarrollo.

Las cuatro preguntas de la sección 7 quedan contestadas: un nivel, el padre enseña también lo de sus subcarpetas, el borrado como D51 y, en la interfaz, «subcarpeta».

## 2. Decisiones y supuestos

| # | Decisión | Estado |
|---|---|---|
| D43 | **Un solo nivel**: proyecto → subcarpetas. Una subcarpeta no puede tener subcarpetas. El modelo (`padre_id`) admite más niveles; el tope lo pone la API (400 si el padre ya tiene padre). | [SUPUESTO] Con un nivel basta («la carpeta padre o cualquiera de sus hijos»). Plan B: quitar el tope y sangrar el árbol por nivel; la tira de carpetas pasaría a mostrar migas en lugar de hermanas. |
| D44 | **Una nota, un sitio** (sigue D14): `notas.proyecto_id` apunta al padre o a una subcarpeta. Nada cambia en la tabla `notas`. | Decidido. |
| D45 | **Entrar en un proyecto enseña lo suyo y lo de sus subcarpetas**, cada nota de subcarpeta con un chip 📂. Debajo de la cabecera, una tira que se desliza: **Todo · Sin subcarpeta · 📂 cada subcarpeta · ＋ Subcarpeta**. Tocar una subcarpeta es entrar en ella (pasa a ser el proyecto activo, recordado en `notas-proyecto` como hoy). | [SUPUESTO] Al entrar en el padre se quiere ver todo. Plan B: que el padre enseñe solo sus notas sueltas y «Todo» sea el chip opcional. |
| D46 | **Dentro de una subcarpeta**, la cabecera dice «App de notas › Diseño» (tocar el padre vuelve a él) y la tira sigue visible con la subcarpeta activa marcada, para saltar a las hermanas. | Decidido en el mock. |
| D47 | **Crear subcarpeta** desde tres sitios: el chip «＋ Subcarpeta» de la tira, «＋ Añadir subcarpeta» en la hoja del proyecto y el campo **Dentro de** al crear o editar. «Dentro de» también mueve una subcarpeta a otro proyecto o la saca a proyecto principal; está bloqueado si el proyecto ya tiene subcarpetas (D43). | Decidido. |
| D48 | **Nombre único dentro de su padre**: «Diseño» puede existir en dos proyectos. Los proyectos principales siguen siendo únicos entre sí. | Decidido. Ver sección 3 para hacerlo sin reconstruir la tabla. |
| D49 | **Pantalla Proyectos**: cada tarjeta enseña recuento total (con subcarpetas), «N subcarpetas» y los chips 📂 de sus subcarpetas con su recuento; tocar un chip entra directo. Buscar encuentra también subcarpetas, que salen solas con «📁 Padre ›» encima. El orden (actividad, nombre…) se aplica a los principales; las subcarpetas van por nombre. | [SUPUESTO] Subcarpetas por nombre, no por actividad. Plan B: el mismo criterio que la lista. |
| D50 | **Mover a…** y el desplegable **Proyecto** del editor enseñan el árbol: en la hoja, las subcarpetas sangradas bajo su padre; en el `<select>`, con ruta completa («App de notas › Diseño»), porque el select cerrado solo enseña la opción elegida. «Crear y mover» gana un selector «Como proyecto nuevo / Dentro de X». | Decidido. |
| D51 | **Borrar**: una subcarpeta → sus notas **suben al padre**. Un proyecto con subcarpetas → sus notas sueltas van a «Sin proyecto» y **sus subcarpetas pasan a proyectos principales** con sus notas. Nunca se borra una nota (D19). | [SUPUESTO] Plan B: borrar la familia entera con todas sus notas a «Sin proyecto», o preguntar en la confirmación. |
| D52 | **Archivar** es de proyectos principales y arrastra a sus subcarpetas; una subcarpeta sola no se archiva. | [SUPUESTO] Plan B: archivar subcarpetas sueltas, con «Archivadas · N» al final de la tira. |
| D53 | **Documentos, entrevistas, etiquetas y exportación** siguen el mismo ámbito que la lista: `proyecto=<padre>` incluye sus subcarpetas; `proyecto=<subcarpeta>`, solo ella. Un documento hecho con notas de varias subcarpetas del mismo padre queda en el padre, y al modelo se le pasan la descripción del padre y la de la subcarpeta. La exportación usa `## Padre › Subcarpeta (N)`. | Decidido. |

## 3. Modelo de datos (migración 3)

```sql
-- user_version 2 -> 3
ALTER TABLE proyectos ADD COLUMN padre_id TEXT REFERENCES proyectos(id);
CREATE INDEX proyectos_padre ON proyectos(padre_id);
PRAGMA user_version = 3;
```

- **Unicidad por padre sin reconstruir la tabla**: `nombre_clave` es `UNIQUE` a nivel de columna y SQLite no deja quitarlo sin copiar la tabla. En las subcarpetas se guarda `nombre_clave = '<padre_id>/<clave>'`; los principales siguen con la clave a secas. El `UNIQUE` global pasa a significar «único dentro del padre» y el id del padre (ulid) no cambia al renombrarlo. Mover una subcarpeta recalcula su clave. Al sacarla a principal (a mano o por D51) puede chocar con un proyecto que ya se llame igual: a mano, 409; al borrar el padre, la subcarpeta se renombra «Diseño (App de notas)» para que el borrado no falle.
- Sin `ON DELETE` en `padre_id`: borrar lo resuelve el handler en una transacción (D51), igual que hoy hace con notas y documentos.
- `notas`, `documentos` y `entrevistas` no cambian.

## 4. API

| Ruta | Cambio |
|---|---|
| `GET /proyectos` | Cada proyecto trae `padre` (`{id, nombre}` o `null`) y `subcarpetas` (recuento). `notas` y `ultima_nota` del padre suman las de sus subcarpetas; añade `notas_propias`. Con `q`, las subcarpetas que coinciden salen como filas propias. |
| `POST /proyectos`, `PATCH /proyectos/{id}` | Campo `padre` (`id` o `null`). 400 si el padre es una subcarpeta, si el proyecto ya tiene subcarpetas y se quiere meter en otro, o si el padre no existe; 409 si el nombre choca dentro del padre. |
| `DELETE /proyectos/{id}` | D51 en una transacción. |
| `PATCH` con `archivado` | Sobre un principal, arrastra a sus subcarpetas (D52); sobre una subcarpeta, 400. |
| `GET /notas`, `/etiquetas`, `/documentos`, `/entrevistas` con `proyecto=<id>` | Incluye las subcarpetas de `<id>` (`proyecto_id IN (id, hijos)`). Nuevo `solo=1` para «Sin subcarpeta». Cada nota trae `proyecto: {id, nombre, padre?}`. |
| `POST /documentos` | `proyecto_id` = el común de las notas, o su padre si todas cuelgan del mismo (D53). |

`deploy.yml` crecería con: crear padre y subcarpeta, nota en la subcarpeta, `GET /notas?proyecto=<padre>` la trae y `solo=1` no, mismo nombre en dos padres (201) y en el mismo (409), borrar la subcarpeta (la nota sube) y borrar el padre (la subcarpeta queda principal).

## 5. Qué se ve en el mock

`docs/mocks/019-subproyectos.html`, navegable y con datos de ejemplo en memoria (nada se guarda). Banda arriba con «Qué cambia», que resume las decisiones.

1. **Proyectos**: tarjetas con chips 📂; buscar «dis» encuentra la subcarpeta Diseño con su ruta.
2. **Notas de «App de notas»**: tira Todo · Sin subcarpeta · 📂 Backend · 📂 Diseño · 📂 Ideas sueltas · ＋ Subcarpeta; notas de subcarpeta con su chip.
3. **Dentro de Diseño**: cabecera «App de notas › Diseño», tira con Diseño marcada.
4. **Detalle de nota**: dos chips, «📁 App de notas» y «📂 Diseño», cada uno lleva a su sitio.
5. **Mover a…**: árbol con la ubicación actual desactivada; crear y mover como proyecto o subcarpeta.
6. **Editar proyecto**: «Dentro de», lista de subcarpetas con ⋯ y «＋ Añadir subcarpeta»; borrar explica qué pasa con notas y subcarpetas.
7. **Editor**: desplegable con rutas completas.

Atajos para abrir una pantalla directamente: `?v=lista`, `sub`, `detalle`, `mover`, `hoja`, `editor`, `buscar`.

## 6. Fases

| Fase | Entregable |
|---|---|
| F23 ✅ | Backend: migración 3, `padre` en `/proyectos`, filtros con hijos y `solo=1`, borrar y archivar según D51/D52, verificación en `deploy.yml`. Compatible con la app actual (sin subcarpetas, nada cambia). |
| F24 ✅ | App: tira de carpetas, migas en la cabecera, tarjetas con chips, hoja del proyecto con «Dentro de», árbol en Mover y en el editor, exportación. Pruebas con Chromium a 390 px, como F15. |

## 7. Preguntas al usuario (contestadas: sí a las tres primeras, «subcarpeta» en la cuarta)

1. ¿Basta con un nivel (D43)?
2. ¿Al entrar en el padre, ver también las notas de sus subcarpetas (D45)?
3. ¿Borrar un proyecto con subcarpetas las deja como proyectos principales (D51)?
4. ¿En la interfaz, «subcarpeta» o «subproyecto»? El mock usa «subcarpeta» porque es más corta a 390 px y casa con los iconos 📁/📂.

## 8. Cómo quedó (2026-09-24)

- **Migración 3**: probada creando la base con el binario de `main` (versión 2, con un proyecto y notas) y arrancando el nuevo encima: `user_version` pasa a 3, todo queda como proyecto principal y un segundo arranque no vuelve a migrar.
- **`nombre_clave`**: la clave de siempre distingue ñ de n («Diseño» y «DISENO» son nombres distintos); no se ha tocado.
- **Documentos**: con notas de subcarpetas hermanas el documento queda en el padre; con notas de una sola subcarpeta, en ella; mezclando proyectos, sin proyecto. Al modelo le llega «Padre › Subcarpeta» y las dos descripciones, igual que en la transcripción y el resumen de entrevistas.
- **App**: la tira se recarga con cada lista sin filtro (no en cada tecla del buscador), así que sus cifras siguen a lo que se crea y se mueve. En el modo selección no hay tira ni migas. «Mover» en la selección solo dice «Ahora está en…» si todas las notas comparten sitio. El proyecto activo guarda también su padre en `notas-proyecto`, así que al recargar dentro de una subcarpeta salen las migas.
- **Pruebas**: 50 comprobaciones con Chromium a 390×844 contra el backend real en local (proyectos, tira, migas, «Sin subcarpeta», crear subcarpeta y el aviso del nombre repetido, editor, detalle, «Mover a…» y «Crear y mover», selección, búsqueda, hoja del proyecto, mover una subcarpeta con «Dentro de», borrar padre y subcarpeta, entrevistas, recarga y exportación); sin desbordes. El único error de consola es el 409 que la prueba provoca a propósito. En el backend, además, 40 comprobaciones con `curl` y el paso nuevo «Verificar subcarpetas» de `deploy.yml` ejecutado contra el binario local.
