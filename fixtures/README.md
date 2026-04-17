# Fixtures

Corpus de logs FIX para validar el parser y el sniffer. Dos categorías:

- `real/` — logs reales del usuario (gitignored, no forman parte del repo).
- `synthetic/` — logs pequeños, deterministas y versionados, diseñados para tests golden.

## `synthetic/`

Cada archivo sintético contiene mensajes FIX 4.4 con **BodyLength y CheckSum coherentes con el separador usado en el log** (es decir, los checksums fueron calculados sobre los bytes reales del archivo, no sobre el equivalente con SOH). Esto permite que cualquier validador construido durante la Fase 1 pueda verificar el fixture sin conocimiento previo del formato original.

| Archivo | Mensajes | Separador | Prefijo de línea | Propósito |
|---|---|---|---|---|
| `minimal_4.4.log` | 10 válidos | `SOH` (`\x01`) | — | Baseline del parser MVP. Cubre Logon, Heartbeat, TestRequest, NewOrderSingle, ExecutionReport (New/Partial/Fill), OrderCancelRequest, Logout. |
| `pipe_separated.log` | 10 válidos | `|` (`0x7C`) | — | Variante con pipe en lugar de SOH. Misma semántica que `minimal_4.4.log` para permitir diff de comportamiento del sniffer. |
| `with_timestamp_prefix.log` | 10 válidos | `SOH` | `YYYYMMDD-HH:MM:SS.sss : ` | Estilo QuickFIX: cada línea tiene un timestamp antes del mensaje FIX. Prueba el stripping de prefijo del sniffer. |
| `malformed.log` | 3 válidos + 3 corruptos + 2 líneas vacías | `SOH` | — | Robustez. Ver detalle abajo. |

### Detalle de `malformed.log`

Las 3 clases de corrupción están presentes:

1. **CheckSum incorrecto** — mensaje con `10=999` donde el valor esperado es distinto. Detectable por validación de checksum.
2. **BodyLength inconsistente** — `9=999` en un mensaje cuyo body real tiene otra longitud. Detectable comparando el valor declarado con los bytes entre el separador después de `9=` y el separador previo a `10=`.
3. **Tag sin valor** — `38=` seguido del separador (campo `OrderQty` vacío). El mensaje está estructuralmente bien formado (BodyLength y CheckSum correctos), pero semánticamente inválido: el parser debe emitir un warning y permitir que el consumidor decida si descartarlo.

El archivo además contiene dos líneas vacías intercaladas para simular ruido común en logs de producción (rotación, líneas de diagnóstico separadoras, etc.).

El criterio de aceptación del parser (T05) sobre este fixture es: emitir correctamente los 3 mensajes válidos, loggear warnings para los 3 corruptos, no crashear.

### Regenerar los fixtures sintéticos

Los 4 archivos son deterministas. Si se modifica la especificación de los mensajes o se corrige un bug en el generador, regenerar con el script original (no committeado). Los mensajes están diseñados con timestamps fijos (`20260416-...`) para que las re-generaciones produzcan el mismo output byte a byte.

## `real/`

Logs del usuario, anonimizados manualmente antes de añadirlos. **Están en `.gitignore`** y nunca deben committearse. Sirven para validación end-to-end (T17): `fixlog sniff`, `fixlog stats` y `fixlog parse` sobre cada uno deben ejecutarse sin crashes.

Convención de nombre: `<versión>-<tipo>.log` (p. ej. `fix44-om.log` = FIX 4.4 Order Management, `fixt11-md.log` = FIXT.1.1 Market Data).
