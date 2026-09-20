# F2 — Título y etiquetas por IA, a través del servicio `openrouter`

Fecha: 2026-09-20 · Estado: implementado, a la espera de la clave (paso 1 de «Puesta en marcha»).

## La decisión

D5 dejaba abierto qué LLM se usaría. La respuesta ya existía en el otro proyecto del
usuario: **`npiobject-labs/openrouter`**, un servicio propio que habla el contrato de
OpenAI, guarda la clave de OpenRouter en su backend, mide cada llamada, reparte el gasto
por aplicación y corta por presupuesto. Notas no necesita una clave de OpenRouter ni saber
qué modelo hay detrás: se da de alta como **una aplicación más** del servicio.

Qué gana Notas por ir por ahí en vez de llamar a OpenRouter directamente:

- **La clave de OpenRouter no entra en este repo ni en Fly.** Notas solo guarda una clave
  de aplicación, revocable con `DELETE /v1/apps/{id}` sin tocar este despliegue.
- **Presupuesto y cuota por aplicación.** Un bucle de guardado no se lleva el crédito:
  `PUT /v1/apps/{id}/presupuesto` pone tope por día o por mes, y el cortacircuitos del
  servicio corta la misma consulta repetida.
- **Gasto medido.** Cada llamada va con `X-Operacion: notas-titulo`, así que
  `GET /v1/uso/resumen?agrupar=operacion` dice exactamente lo que cuesta titular notas.
- **El modelo se cambia sin tocar Notas**: sin `LLM_MODELO`, manda el modelo por defecto
  del servicio.

## Dónde apunta

| Variable | Por defecto | Para qué |
|---|---|---|
| `LLM_CLAVE` | — (secreto de repositorio) | Clave de aplicación del servicio. **Sin ella no hay IA**: la nota se guarda con título de respaldo. |
| `LLM_URL` | `https://apisor.oracle402.com` (VPS, producción) | Variable de repositorio. Para usar Fly: `https://openrouter-npiobject-labs.fly.dev`. |
| `LLM_MODELO` | — (el del servicio) | Variable de repositorio, por si Notas quiere un modelo distinto al de por defecto. |

Los dos despliegues del servicio (VPS y Fly) llevan **claves de aplicación distintas**: no
se sincronizan. Si cambias `LLM_URL`, cambia también `LLM_CLAVE`.

## Cómo funciona

1. `POST /notas` guarda la nota **antes de llamar a nadie**, con el título de respaldo
   (primera frase) y `pendiente_ia = 1`. La nota nunca se pierde por culpa de la IA (D6).
2. `app/src/llm.rs` llama a `POST {LLM_URL}/v1/chat/completions` con `Authorization: Bearer
   {LLM_CLAVE}`, `X-Operacion: notas-titulo` y **salida estructurada**
   (`response_format: json_schema`, `strict`), así que la respuesta es
   `{"titulo": "...", "etiquetas": ["...", "..."]}` y no texto libre que haya que adivinar.
3. Al prompt se le pasan **las 40 etiquetas ya en uso**, las más frecuentes primero: el
   vocabulario controlado de D5, para que reutilice antes de inventar.
4. Si contesta a tiempo (8 s), la nota se actualiza en una transacción: título, etiquetas y
   `pendiente_ia = 0`. El índice FTS5 lo mantiene el trigger de `UPDATE`, así que el título
   generado es buscable al momento.
5. Si falla, tarda o el servicio no está configurado, el error se queda en el log del
   backend y la nota conserva el título de respaldo con `pendiente_ia = true`. La app la
   marca como «título provisional».
6. `POST /notas/{id}/reintentar-ia` vuelve a intentarlo. `503` si el backend no tiene
   `LLM_CLAVE`; `502` si el servicio no pudo titular; `404` si la nota ya no está.

El límite de 8 s es el de D6: por encima de eso el usuario está esperando delante del
móvil y vale más una nota guardada con título provisional que una pantalla colgada.
El contenido que viaja se recorta a 4000 caracteres: una nota dictada más larga no mejora
el título y sí encarece la llamada.

## Lo que el servicio ve

Solo el texto de la nota y las etiquetas en uso, con la operación `notas-titulo`. El
histórico del servicio guarda tokens, coste, latencia y modelo, no el contenido. Si el
contenido de las notas se considerara sensible, el plan B es un despliegue del servicio
solo para Notas, o un modelo que no entrene con los datos; hoy no aplica.

## Puesta en marcha (lo que falta, y es manual)

1. **Dar de alta la aplicación** en el servicio, con la clave de administración:

   ```powershell
   # desde el repo openrouter, en el PC
   pwsh -File tools\apps.ps1 -Crear notas
   ```

   Enseña la clave **una sola vez** (del servicio solo se guarda su hash).

2. **Ponerle presupuesto**, para que un fallo en bucle no se lleve el crédito:
   `PUT /v1/apps/{id}/presupuesto` con tope por mes y umbral de aviso. La consola
   (https://npiobject-labs.github.io/openrouter/consola.html) lo hace desde el móvil.

3. **Guardar la clave en este repo**: Settings → Secrets and variables → Actions →
   *New repository secret* → `LLM_CLAVE`. Opcional: variables `LLM_URL` y `LLM_MODELO`.

4. El siguiente run de `deploy.yml` la vuelca a Fly. Se comprueba en
   https://prueba-npiobject-labs.fly.dev/salud → `"ia": true`, y en la app en
   ⚙ Ajustes, que lee ese mismo `/salud`.

Mientras el secreto no exista, todo sigue funcionando: las notas se guardan con título
provisional y sin etiquetas, y el run de `deploy.yml` lo avisa en el resumen.

## Coste

[SUPUESTO] Con el modelo por defecto del servicio (`google/gemini-2.5-flash-lite`), una
nota dictada normal son unos cientos de tokens de entrada y menos de 50 de salida: del
orden de 0,00002 $ por nota, es decir, céntimos al año para un uso personal. Plan B si se
dispara: bajar `MAX_CONTENIDO`, o fijar `LLM_MODELO` a un modelo aún más barato del
catálogo. El gasto real sale de `GET /v1/uso/resumen?agrupar=operacion` en el servicio.
