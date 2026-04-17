# Fase 1 — Plan detallado

Plan atómico y ordenado para la Fase 1. Cada tarea tiene objetivo claro, archivos afectados, criterio de aceptación y dependencias. Ejecutar en orden.

**Convención de estado**:

- `[ ]` pendiente
- `[~]` en curso
- `[x]` completa

---

## T01 · Setup del workspace Cargo

- **Estado**: `[ ]`
- **Depende de**: —
- **Objetivo**: estructura raíz del proyecto lista para recibir crates.
- **Archivos**:
  - `Cargo.toml` (workspace con `members = ["crates/*"]`, `resolver = "2"`, perfiles `release` con LTO y `codegen-units = 1`)
  - `.gitignore` (target/, \*.fixlog-idx, .DS_Store, etc.)
  - `README.md` (stub: título, one-liner, link a `docs/ROADMAP.md`)
  - `rust-toolchain.toml` (channel = "stable", components = ["rustfmt", "clippy"])
  - `rustfmt.toml` (edition = "2021", max_width = 100)
  - `clippy.toml` (avoid-breaking-exported-api = false)
- **Criterio de aceptación**: `cargo build --workspace` ejecuta sin crates (o con stubs) sin errores.

---

## T02 · Corpus de fixtures inicial

- **Estado**: `[x]`
- **Depende de**: T01
- **Objetivo**: set mínimo de logs para validar el parser con datos reales y sintéticos.
- **Archivos**:
  - `fixtures/real/` con 3-5 logs reales del usuario (anonimizados si aplica).
  - `fixtures/synthetic/minimal_4.4.log` con 10 mensajes FIX 4.4 SOH, sin prefijo.
  - `fixtures/synthetic/with_timestamp_prefix.log` con prefijo `YYYYMMDD-HH:MM:SS.sss : ` estilo QuickFIX.
  - `fixtures/synthetic/pipe_separated.log` con `|` en vez de SOH.
  - `fixtures/synthetic/malformed.log` con mezcla de válidos + 3 tipos de corrupción (checksum malo, body length inconsistente, tag sin valor).
  - `fixtures/README.md` explicando qué contiene cada archivo.
- **Criterio de aceptación**: fixtures comiteados, README describe cada uno, ningún log contiene datos sensibles.

---

## T03 · Stub del crate `fixlog-parser` con tipos base

- **Estado**: `[x]`
- **Depende de**: T01
- **Objetivo**: tipos públicos definidos sin lógica, compilables.
- **Archivos**:
  - `crates/fixlog-parser/Cargo.toml` (deps: `smallvec`, `thiserror`, `tracing`)
  - `crates/fixlog-parser/src/lib.rs` con:
    - `pub struct RawMessage<'a>`
    - `pub struct ParseError`
    - `pub const TAG_BEGIN_STRING: u32 = 8;` y demás tags críticos (8, 9, 10, 34, 35, 49, 52, 56)
  - `crates/fixlog-parser/src/tokenizer.rs` (stub con firma de función principal)
- **Criterio de aceptación**: `cargo build -p fixlog-parser` pasa. `cargo clippy -p fixlog-parser -- -D warnings` pasa.

---

## T04 · Tokenizer SOH (parser MVP sin sniffer)

- **Estado**: `[ ]`
- **Depende de**: T03
- **Objetivo**: convertir bytes (asumiendo separador SOH, sin prefijo) en `RawMessage`.
- **Archivos**:
  - `crates/fixlog-parser/src/tokenizer.rs` implementado.
  - `crates/fixlog-parser/src/validator.rs` con validación de BeginString, BodyLength, CheckSum.
  - Tests inline: parseo de 1 mensaje, parseo con tag sin valor, parseo con checksum incorrecto.
- **Detalles técnicos**:
  - Función principal: `pub fn parse_one(buf: &[u8]) -> Result<(RawMessage<'_>, usize), ParseError>`. Retorna el mensaje y cuántos bytes consumió.
  - Iterador: `pub fn parse_all(buf: &[u8]) -> impl Iterator<Item = Result<RawMessage<'_>, ParseError>>`.
  - CheckSum: suma de bytes módulo 256, validada contra tag 10 (3 dígitos ASCII).
- **Criterio de aceptación**: tests pasan, clippy limpio, benchmark básico (puede ser println del throughput) sobre `fixtures/synthetic/minimal_4.4.log`.

---

## T05 · Tests de integración del parser contra fixtures sintéticas

- **Estado**: `[ ]`
- **Depende de**: T02, T04
- **Objetivo**: validar parser contra múltiples fixtures pequeños, incluyendo edge cases.
- **Archivos**:
  - `crates/fixlog-parser/tests/synthetic.rs` (golden tests: parsea y compara conteos y tags esperados).
  - `fixtures/synthetic/*.expected.json` con outputs esperados (conteo de mensajes, MsgTypes detectados).
- **Criterio de aceptación**: todos los tests sintéticos pasan. El test con `malformed.log` verifica que el parser loggea warnings pero produce los mensajes válidos.

---

## T06 · Stub + implementación del crate `fixlog-format` (sniffer)

- **Estado**: `[ ]`
- **Depende de**: T03
- **Objetivo**: detectar separador, prefijo, encoding y line endings en las primeras ~1000 líneas.
- **Archivos**:
  - `crates/fixlog-format/Cargo.toml` (deps: `regex`, `memchr`, `thiserror`, `tracing`)
  - `crates/fixlog-format/src/lib.rs` con `pub struct LogFormat`, `pub enum Separator`, etc. según `docs/ARCHITECTURE.md`.
  - `crates/fixlog-format/src/sniffer.rs` con `pub fn sniff(sample: &[u8]) -> Result<LogFormat, SniffError>`.
  - Heurísticas:
    - Separador: contar frecuencia de candidatos (`\x01`, `|`, `^A`, `;`) entre pares `<digits>=`.
    - Prefijo: detectar patrones comunes antes de `8=FIX...`.
    - Encoding: BOM check, luego validar UTF-8.
    - Line ending: conteo `\r\n` vs `\n`.
  - Tests inline por heurística.
- **Criterio de aceptación**: el sniffer identifica correctamente los 4 fixtures sintéticos y los reales. Tests integrados para cada heurística.

---

## T07 · Integración parser ↔ sniffer

- **Estado**: `[ ]`
- **Depende de**: T04, T06
- **Objetivo**: el parser consume `LogFormat` del sniffer, soporta prefijos y separadores no-SOH.
- **Archivos**:
  - `crates/fixlog-parser/src/lib.rs`: añadir `pub fn parse_all_with_format<'a>(buf: &'a [u8], format: &LogFormat) -> impl Iterator<...>`.
  - `fixlog-parser` gana dep en `fixlog-format` (o se mueve `LogFormat` a un crate común si se quiere evitar ciclo — ver decisión abajo).
  - Tests contra fixtures con pipe y con prefijo de timestamp.
- **Decisión a tomar en esta tarea**: ¿`LogFormat` vive en `fixlog-format` (parser depende de format) o se extrae a un `fixlog-types` común? Default: vive en `fixlog-format`, es más simple.
- **Criterio de aceptación**: parser parsea correctamente `pipe_separated.log` y `with_timestamp_prefix.log` via sniffer → parser.

---

## T08 · Stub del crate `fixlog-dict` y pipeline XML → código

- **Estado**: `[ ]`
- **Depende de**: T03
- **Objetivo**: `build.rs` que convierte `dictionaries/FIX44.xml` en código Rust estático.
- **Archivos**:
  - `crates/fixlog-dict/Cargo.toml` (deps: `fixlog-parser`, `thiserror`. build-deps: `quick-xml`, `heck`)
  - `crates/fixlog-dict/build.rs` con lógica de parseo XML → generación de `fix44_dict.rs` en `OUT_DIR`.
  - `dictionaries/FIX44.xml` (descargar de QuickFIX/J repo, committeado en `dictionaries/`).
  - `crates/fixlog-dict/src/lib.rs` con `pub struct FieldDef`, `pub struct MessageDef`, `pub enum FixVersion`.
  - `crates/fixlog-dict/src/dict_fix44.rs` que hace `include!(concat!(env!("OUT_DIR"), "/fix44_dict.rs"));`.
- **Criterio de aceptación**: `cargo build -p fixlog-dict` regenera el diccionario desde el XML. `cargo test -p fixlog-dict` valida que hay N campos esperados (ej: >900 para FIX 4.4).

---

## T09 · Resolver: `RawMessage` → `ResolvedMessage`

- **Estado**: `[ ]`
- **Depende de**: T08
- **Objetivo**: función que toma `RawMessage` y devuelve `ResolvedMessage` con nombres y enums decodificados.
- **Archivos**:
  - `crates/fixlog-dict/src/resolver.rs` con `pub fn resolve<'a>(msg: &'a RawMessage<'a>) -> ResolvedMessage<'a>`.
  - Selección de diccionario: en Fase 1 hardcoded a FIX 4.4. En T14 se generaliza.
  - Tests: resolver un ExecutionReport → verificar que `tag 35 = 8` se resuelve a `MsgType = ExecutionReport`, `tag 54 = 1` a `Side = Buy`.
- **Criterio de aceptación**: resolver correcto sobre al menos 5 tipos de mensajes distintos (D, 8, 0, 1, 3).

---

## T10 · Stub del crate `fixlog-cli`

- **Estado**: `[ ]`
- **Depende de**: T07, T09
- **Objetivo**: binario CLI con esqueleto de comandos usando `clap`.
- **Archivos**:
  - `crates/fixlog-cli/Cargo.toml` (deps: `fixlog-format`, `fixlog-parser`, `fixlog-dict`, `clap` con features `derive`, `anyhow`, `tracing-subscriber`, `memmap2`).
  - `crates/fixlog-cli/src/main.rs` con `#[derive(Parser)]` y subcomandos: `sniff`, `parse`, `stats`.
  - Logging init con `tracing-subscriber` y flag `-v`/`-vv`.
- **Criterio de aceptación**: `cargo run -p fixlog-cli -- --help` muestra subcomandos. Cada subcomando ejecuta con `todo!()` o mensaje placeholder.

---

## T11 · Comando `sniff`

- **Estado**: `[ ]`
- **Depende de**: T10
- **Objetivo**: `fixlog sniff <file>` imprime el `LogFormat` detectado.
- **Archivos**:
  - `crates/fixlog-cli/src/commands/sniff.rs`.
  - mmap del archivo, toma primeros 64KB o 1000 líneas (lo que sea menor), llama al sniffer, imprime resultado.
- **Criterio de aceptación**: salida legible para los 4 fixtures sintéticos y logs reales.

---

## T12 · Comando `parse`

- **Estado**: `[ ]`
- **Depende de**: T10
- **Objetivo**: `fixlog parse <file> [--first N] [--format json|pretty]` imprime mensajes.
- **Archivos**:
  - `crates/fixlog-cli/src/commands/parse.rs`.
  - Formato `pretty`: tabla legible con tag, nombre, valor (decodificado si enum).
  - Formato `json`: un objeto JSON por mensaje, una línea cada uno (JSONL), para piping a `jq`.
- **Criterio de aceptación**: ambos formatos funcionan sobre fixtures. Output JSON es válido (valida con `jq empty`).

---

## T13 · Comando `stats`

- **Estado**: `[ ]`
- **Depende de**: T10
- **Objetivo**: `fixlog stats <file>` imprime resumen ejecutivo.
- **Archivos**:
  - `crates/fixlog-cli/src/commands/stats.rs`.
  - Métricas: total mensajes parseados, total con error, conteo por MsgType (top 10), rango temporal de tag 52 (SendingTime), sesiones únicas detectadas (combinaciones SenderCompID + TargetCompID).
- **Criterio de aceptación**: output útil sobre un log real. Tiempo de ejecución razonable (<5s para archivos de 100MB como benchmark informal).

---

## T14 · Ampliación de diccionarios: 5.0, 5.0SP1, 5.0SP2, FIXT.1.1

- **Estado**: `[ ]`
- **Depende de**: T08, T09
- **Objetivo**: soportar todas las versiones requeridas y selección automática.
- **Archivos**:
  - `dictionaries/FIX50.xml`, `FIX50SP1.xml`, `FIX50SP2.xml`, `FIXT11.xml`.
  - `crates/fixlog-dict/build.rs` extendido para generar todos los diccionarios.
  - `crates/fixlog-dict/src/resolver.rs`: lógica de selección. Si `BeginString = FIXT.1.1`, buscar tag 1128 (ApplVerID) y elegir diccionario de aplicación. Si no hay tag 1128, fallback configurable.
  - Tests: mensaje FIXT con ApplVerID=9 (FIX 5.0SP2) se resuelve correctamente.
- **Criterio de aceptación**: resolver correcto para mensajes en las 4 versiones. Fixture sintético con FIXT incluido.

---

## T15 · `fixlog-core` como facade

- **Estado**: `[ ]`
- **Depende de**: T07, T09, T14
- **Objetivo**: crate que re-exporta los tipos públicos de los crates inferiores.
- **Archivos**:
  - `crates/fixlog-core/Cargo.toml`.
  - `crates/fixlog-core/src/lib.rs` con `pub use fixlog_parser::*; pub use fixlog_format::*; pub use fixlog_dict::*;`.
- **Criterio de aceptación**: `fixlog-cli` puede importar todo desde `fixlog_core::` sin perder funcionalidad.

---

## T16 · Benchmark baseline con `criterion`

- **Estado**: `[ ]`
- **Depende de**: T07, T13
- **Objetivo**: establecer métricas de performance como referencia para futuras fases.
- **Archivos**:
  - `crates/fixlog-parser/benches/parse.rs` con benches: `parse_10k_messages_soh`, `parse_10k_messages_pipe`, `parse_with_prefix`.
  - `benches/baselines.json` con resultados iniciales (opcional, automatizable después).
- **Criterio de aceptación**: `cargo bench -p fixlog-parser` corre sin fallos y reporta medidas repetibles.

---

## T17 · Validación end-to-end

- **Estado**: `[ ]`
- **Depende de**: todas las anteriores
- **Objetivo**: validar que todo funciona sobre el corpus real, no solo sintético.
- **Pasos**:
  1. Correr `fixlog sniff` sobre cada archivo en `fixtures/real/`.
  2. Correr `fixlog stats` sobre cada uno.
  3. Correr `fixlog parse --first 5 --format pretty` sobre cada uno.
  4. Ningún crash. Logs de warning aceptables para mensajes corruptos.
- **Criterio de aceptación**: el usuario revisa manualmente los outputs y confirma que se ven razonables. Issues encontrados se documentan y fixean en una T18 (si aplica) antes de declarar Fase 1 cerrada.

---

## Definición de "Fase 1 completa"

- Todas las tareas T01-T17 en `[x]`.
- `cargo test --all` pasa.
- `cargo clippy --all-targets -- -D warnings` pasa.
- `cargo fmt --all --check` pasa.
- Tag git `v0.1.0-phase1` creado.
- Commit de cierre: `chore(phase1): close Fase 1 — Core Parser`.
- Documento `docs/PHASE2_PLAN.md` iniciado con ideas recogidas durante la Fase 1.

---

## Notas sobre ejecución con Claude Code

- **Una tarea por sesión**. Al terminar, `/clear`.
- **Plan Mode obligatorio** para T04, T06, T08, T09. Son tareas de diseño.
- **Invocar `/phase-task <número>`** para empezar cada tarea.
- **Invocar `/validate`** antes de cerrar cada tarea.
- **Invocar `/fixture-check`** al menos en T05, T11, T12, T13, T17.
- **Commit al cerrar cada tarea** con conventional commit.
