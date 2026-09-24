# Plan: análisis de notas con varios roles (informe único)

Fecha: 2026-09-24 · Estado: v1, **mock de diseño** (`docs/mocks/020-analisis.html`, build `PR-B1-20260924-028`); sin desarrollar · Continúa D11/D12 (documentos) y `plan-subproyectos.md` (hasta D53) · Fuente de verdad: este fichero.

## 1. Qué se pide (en palabras del usuario)

> A la hora de generar, lo que quiero es que tengas varios agentes con distintas funcionalidades: por ejemplo un agente que sea crítico, un agente analítico, un agente económico, un agente que mide la parte del marketing, un agente que mire la usabilidad… no sé qué más cosas puede mirar, eso me lo tienes que decir tú. La idea es que pueda seleccionar uno o varios roles de agentes a la hora de generar el documento. Ese resultado ha de tener un icono de copiar para llevarlo a otro sitio y analizar, y también estaría bien descargar. Tienes acceso con OpenRouter a todos los modelos: estaría bien que en Ajustes se pueda seleccionar algún modelo más de OpenRouter para que se haga este análisis de la mejor forma, porque a lo mejor el que tenemos por defecto no es suficiente para generar este informe compactado, formateado y bien estructurado.

> (Tras el resumen ejecutivo) Informe único; dos, ambos; tres, sí. Adelante con el mock.

Respuestas: (1) un **informe único**, no una pestaña por rol; (2) el modelo se elige **en Ajustes y también en la hoja de Generar**; (3) **sí a roles propios**.

## 2. Lo que ya hay y lo que se ha comprobado

- Documentos (D11/D12): `POST /documentos` con 1 a 20 notas, generación en segundo plano, sondeo cada 3 s, «Notas de origen» escritas por el backend, y en la vista del documento ya están **Copiar, Compartir y Descargar** (.md). El informe hereda todo eso.
- El servicio `openrouter` (repo público `npiobject-labs/openrouter`, leído en esta sesión):
  - `POST /v1/chat/completions` reenvía el cuerpo tal cual y solo rellena `model` si falta: **cualquier modelo del catálogo pasa**.
  - `GET /v1/models` da el catálogo con `id`, `nombre`, `contexto`, precio de `entrada` y `salida` (dólares por millón), `json` (salida estructurada) y `modalidades`, con caché de una hora y filtros `texto`, `proveedor`, `contexto_min`, `gratis`.
  - Cada aplicación puede tener **presupuesto** (402 `presupuesto_agotado`) y **cuota por minuto**, y hay un **cortacircuitos** que corta con 429 la misma petición repetida cinco veces en un minuto. Las peticiones de los roles son distintas entre sí, así que no lo disparan.
  - La cabecera `X-Operacion` agrupa llamadas, y `/v1/uso/resumen?operacion=` dice cuánto costaron; con clave de aplicación solo se ve lo propio.
  - El modelo por defecto del servicio es `google/gemini-2.5-flash-lite`. [SUPUESTO] Los documentos salen hoy con él, porque `LLM_MODELO_DOCUMENTO` no está fijada (el documento de `deploy.yml` tarda unos 5 s). Plan B: si está fijada, esa pasa a ser «Por defecto del servidor» en el selector.
  - El proxy espera **120 s** al proveedor. Una sola llamada que escriba un informe entero con un modelo grande puede pasarse; por eso la síntesis solo escribe la parte de arriba (D57).

## 3. Decisiones y supuestos

| # | Decisión | Estado |
|---|---|---|
| D54 | **Generar pregunta qué se quiere**: `📄 Encargo` (el documento de hoy, sin cambios) o `🔎 Análisis` (informe con roles). La hoja recuerda la última opción. | Decidido en el mock. |
| D55 | **Nueve roles de serie**: 🧐 Crítico, 📊 Analítico, 💶 Económico, 📣 Marketing, 🧭 Usabilidad, 🛠 Técnico, ⚖️ Legal y privacidad, 🎯 Producto, 👤 Usuario final. Viven en el código (`app/src/analisis.rs`), no en la base, para que mejoren con cada versión. Se eligen uno o varios con tarjetas; «Todos / Ninguno»; la app recuerda la última elección en `localStorage` (`notas-roles`). | Decidido. [SUPUESTO] Estos nueve cubren lo habitual. Plan B: añadir o quitar de serie es tocar una lista. |
| D56 | **Tope de 8 roles por informe**. | [SUPUESTO] Más roles encarecen y alargan sin añadir mucho. Plan B: subir el tope, que es una constante. |
| D57 | **Cómo se genera**: una llamada por rol, **todas a la vez** (máximo 4 simultáneas), cada una con todas las notas y la estructura común (Veredicto, Hallazgos, Recomendaciones, Preguntas abiertas). Después, una **síntesis** que recibe notas y análisis y escribe solo: título, Resumen ejecutivo, Conclusiones clave, **Tensiones entre roles**, Plan de acción (con prioridad) y Preguntas abiertas. El backend compone el informe: síntesis + «Análisis por rol» (lo de cada rol, tal cual) + «Notas de origen» + «Ficha». | Decidido. La síntesis corta cabe en los 120 s del proxy y cuesta poco; lo de cada rol no se reescribe ni se pierde. |
| D58 | **Salida en markdown, sin `json_schema`**, en roles y síntesis: así vale cualquier modelo del catálogo, también los que no tienen salida estructurada. El título es el primer `# ` de la síntesis, normalizado; si falta, el de la primera nota. | Decidido. |
| D59 | **Roles propios**: nombre (único, 30 caracteres), icono (un emoji) y «qué mira este rol» (hasta 600 caracteres), al que el backend añade la estructura común. Se crean desde la hoja («＋ Rol propio», que lo deja marcado) o desde Ajustes; se editan y borran en Ajustes. Un rol de serie se ve pero no se edita: **«Duplicar como propio»** lo copia para ajustarlo. Se guardan en el servidor (tabla `roles`), así que salen en todos los dispositivos. Borrar un rol no cambia los informes hechos. | Decidido (respuesta 3). |
| D60 | **Modelo**: en ⚙ Ajustes, «Modelo para documentos e informes» (vale para los dos; titular notas sigue con el barato del servidor). En la hoja se puede cambiar **solo para ese informe**. Selector con buscador: arriba «Por defecto del servidor», luego «Recomendados para informes» (lista corta en el código, solo los que existan en el catálogo) y después todo el catálogo de texto, con precio y contexto. La elección de Ajustes se guarda en `localStorage` (`notas-modelo`). | Decidido (respuesta 2). [SUPUESTO] Basta con guardarlo por dispositivo. Plan B: tabla `ajustes` en el backend. |
| D61 | **El backend valida el modelo** contra el catálogo (400 si no existe). La app nunca habla con el servicio: `GET /modelos` del backend lo reenvía con la clave de la aplicación, que no sale del servidor. | Decidido. |
| D62 | **Coste antes de generar**: la hoja estima llamadas y dólares con el precio del modelo (unos 3,6 caracteres por token; 1.200 tokens por rol y 1.500 la síntesis). Por encima de **0,25 $** el aviso sale en amarillo. Al terminar, la ficha dice **lo que costó**: cada llamada va con `X-Operacion: informe-<id>` y el backend pregunta a `/v1/uso/resumen`. | [SUPUESTO] 0,25 $ como umbral. Plan B: umbral configurable en Ajustes. [SUPUESTO] La clave de la app puede leer su resumen por operación (así lo dice la documentación del servicio); plan B: coste con los tokens de `usage` y el precio del catálogo. |
| D63 | **Progreso rol a rol**: `GET /documentos/{id}` trae el estado de cada rol y de la síntesis; la app lo pinta con ⏳/✓/✗ y barra, sondeando cada 3 s como hoy. Sigue aunque se cierre la app; al arrancar el servidor, lo que estaba a medias pasa a fallido. | Decidido. |
| D64 | **Si un rol falla**, el informe se compone con los demás y lo dice arriba («⚠️ Económico no respondió… · Reintentar Económico»). Reintentar rehace solo los fallidos y la síntesis, como los trozos de las entrevistas. Si fallan todos, el informe queda fallido. 402 del presupuesto → «Se acabó el presupuesto de IA de esta aplicación». | Decidido. |
| D65 | **El informe** se lee **formateado** (por defecto) o se edita en markdown (`Leer · Editar`). **📋 Copiar** arriba (como en nota y entrevista) y abajo, Compartir, **⬇ .md** y **🖨 PDF** (imprimir del navegador, con estilo de impresión). El markdown se pinta con un intérprete mínimo propio (títulos, listas, negrita, cursiva, código): nada de librerías de CDN, porque la PWA funciona sin red. | Decidido. |
| D66 | **Documentos** lista encargos e informes juntos; el informe lleva «🔎 Informe · N roles» y sus iconos. **Regenerar** reabre la hoja con los mismos roles, modelo e instrucción, que se pueden cambiar. | Decidido. |
| D67 | **Límites**: siguen de 1 a 20 notas y 30.000 caracteres de material. `max_tokens` 2.500 por rol y 2.500 la síntesis; espera de 120 s por llamada (la del proxy). | [SUPUESTO] Plan B: con modelos de contexto grande, subir notas y caracteres según el `contexto` del catálogo. |

## 4. Modelo de datos (migración 4)

```sql
-- user_version 3 -> 4
ALTER TABLE documentos ADD COLUMN tipo TEXT NOT NULL DEFAULT 'encargo';  -- 'encargo' | 'informe'
ALTER TABLE documentos ADD COLUMN modelo TEXT;                           -- el pedido; null = el del servidor
ALTER TABLE documentos ADD COLUMN coste REAL;                            -- dólares, lo que anotó el servicio
CREATE TABLE IF NOT EXISTS documento_roles (
  documento_id TEXT NOT NULL REFERENCES documentos(id) ON DELETE CASCADE,
  rol_id TEXT NOT NULL, orden INTEGER NOT NULL,
  nombre TEXT NOT NULL, icono TEXT NOT NULL, enfoque TEXT NOT NULL,     -- copia: el informe no cambia si se edita el rol
  estado TEXT NOT NULL DEFAULT 'pendiente',                              -- pendiente | hecho | fallido
  texto TEXT, error TEXT, modelo_servido TEXT,
  PRIMARY KEY (documento_id, rol_id)
);
CREATE TABLE IF NOT EXISTS roles (
  id TEXT PRIMARY KEY, nombre TEXT NOT NULL, nombre_clave TEXT NOT NULL UNIQUE,
  icono TEXT NOT NULL, enfoque TEXT NOT NULL, creado_en TEXT NOT NULL
);
PRAGMA user_version = 4;
```

La síntesis se guarda en las columnas que ya existen (`texto`, `estado`, `error`); el informe completo se compone al leer, para que reintentar un rol no obligue a guardar dos copias.

## 5. API

| Ruta | Cambio |
|---|---|
| `GET /roles` | Los de serie (`propio:false`) y los propios, en ese orden. |
| `POST /roles`, `PUT /roles/{id}`, `DELETE /roles/{id}` | Solo propios; 409 si el nombre choca (también con uno de serie); 400 en un rol de serie. |
| `GET /modelos` | Reenvía `GET /v1/models` del servicio, solo modelos de texto, con `recomendado` y `defecto` marcados. 503 sin `LLM_API_KEY`. |
| `POST /documentos` | Nuevo: `tipo` (`encargo` por defecto), `roles` (ids, 1 a 8, obligatorio si es informe) y `modelo` (opcional, también para encargo). 400 con modelo desconocido o sin roles. |
| `GET /documentos/{id}` | Nuevo: `tipo`, `modelo`, `coste`, `roles: [{id, nombre, icono, estado, error}]`; en un informe, `texto` es el informe compuesto. |
| `POST /documentos/{id}/regenerar` | Acepta `roles`, `modelo` e `instruccion` nuevos. |
| `POST /documentos/{id}/reintentar` | Rehace los roles fallidos y la síntesis. |
| `GET /documentos` | Nuevo: `tipo`, `roles` (iconos y nombres) y `modelo`, para la lista. |

`deploy.yml` crecería con: `GET /roles` trae los nueve de serie; crear, renombrar (409 con un nombre de serie) y borrar un rol propio; `GET /modelos` trae el modelo por defecto; y, con clave de IA, un informe de dos roles con el modelo por defecto que acaba `listo` con «## Resumen ejecutivo», «## Tensiones entre roles», las dos secciones de rol, «## Notas de origen» y `coste` no nulo.

## 6. Qué se ve en el mock

`docs/mocks/020-analisis.html`, navegable y con datos de ejemplo inventados en memoria (un club de pádel; nada se guarda). La banda de arriba tiene el botón «Qué cambia».

1. **Modo selección** como el de hoy; «Generar» abre la hoja nueva.
2. **Hoja de Generar**: Encargo o Análisis, rejilla de roles con su enfoque, «＋ Rol propio», botón de modelo, coste estimado y la instrucción de siempre.
3. **Selector de modelo**: buscador, por defecto del servidor, recomendados y el resto, con precio y contexto (de ejemplo).
4. **Progreso** rol a rol y síntesis.
5. **Informe** formateado, con título, ficha (modelo, notas, coste), chips de roles, Leer/Editar, 📋 Copiar arriba y abajo, Compartir, ⬇ .md, 🖨 PDF, Regenerar y Borrar.
6. **Un rol que falla** (`?v=fallo`): aviso y «Reintentar» solo de ese.
7. **Ajustes**: modelo por defecto y lista de roles; uno de serie se ve y se duplica, uno propio se edita o borra.
8. **Documentos**: encargos e informes juntos.

Atajos: `?v=hoja`, `progreso`, `informe`, `fallo`, `ajustes`, `modelo`, `rol`, `docs`.

## 7. Fases

| Fase | Entregable |
|---|---|
| F25 | Backend: migración 4, `app/src/analisis.rs` (roles de serie, prompts y composición del informe), `/roles`, `/modelos`, informe en `/documentos` con roles en paralelo, síntesis, reintento, coste con `X-Operacion`, y verificación en `deploy.yml`. El encargo de hoy no cambia; la app actual sigue funcionando. |
| F26 | App: hoja de Generar con Encargo/Análisis, roles, modelo y coste; selector de modelo; roles en Ajustes; progreso; informe formateado con copiar, descargar y PDF; lista de documentos. Pruebas con Chromium a 390 px contra el backend local, como F24. |

## 8. Preguntas al usuario

1. ¿Te valen los **nueve roles de serie** (D55) o cambias alguno?
2. ¿El modelo de Ajustes vale **también para los encargos** de siempre, o solo para los informes (D60)?
3. ¿**0,25 $** como umbral del aviso amarillo de coste (D62)?
4. ¿**Tope de 8 roles** por informe (D56)?
