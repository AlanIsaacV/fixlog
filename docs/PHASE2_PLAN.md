# Fase 2 — Indexación + Tailing + Query DSL

Plan atómico y ordenado para la Fase 2. Se arranca con Fase 1 cerrada (benchmarks y parser estables). Cada tarea tiene objetivo, archivos afectados, criterio de aceptación y dependencias. Ejecutar en orden, una por sesión.

**Convención de estado**:

- `[ ]` pendiente
- `[~]` en curso
- `[x]` completa

---

## Resumen de entregables

1. Nuevo crate `fixlog-index`: índice primario (offsets) append-friendly + índice secundario para tags "hot".
2. Nuevo crate `fixlog-query`: DSL de filtros (AST + parser + evaluador) contra `RawMessage`.
3. Subcomando `fixlog grep <file> --filter "<expr>"`, con variantes `--json`, `--follow`.
4. Watcher de archivos con `notify` para tailing + detección de rotación.
5. Persistencia opcional del índice (caché binario) — ver P2-T09 (stretch, puede posponerse a Fase 5).

**Baseline de performance** (de Fase 1, `parse_known_soh/8MiB`): ~243 MiB/s single-thread. Objetivo de Fase 2: indexación paralela ≥ 3× en 4 cores, filtros simples en <100 ms sobre archivos de 1 GB (vía índice secundario).

---

## P2-T01 · Stub del crate `fixlog-index`

- **Estado**: `[ ]`
- **Depende de**: Fase 1 cerrada.
- **Objetivo**: crate con tipos públicos compilables (sin lógica).
- **Archivos**:
  - `crates/fixlog-index/Cargo.toml` (deps: `fixlog-parser`, `memchr`, `thiserror`, `tracing`).
  - `crates/fixlog-index/src/lib.rs` con:
    - `pub struct MessageOffset { start: u64, len: u32 }`
    - `pub struct LogIndex { messages: Vec<MessageOffset>, file_size: u64, ... }`
    - `pub struct SecondaryIndex` (stub, implementado en P2-T03).
    - `pub enum IndexError` con `thiserror`.
  - Añadir miembro al workspace en `Cargo.toml`.
- **Criterio de aceptación**: `cargo build -p fixlog-index` + `cargo clippy -p fixlog-index -- -D warnings` limpios.

---

## P2-T02 · Indexación single-thread sobre buffer

- **Estado**: `[ ]`
- **Depende de**: P2-T01
- **Objetivo**: construir `LogIndex` recorriendo el buffer con el parser actual (sin paralelizar todavía).
- **Archivos**:
  - `crates/fixlog-index/src/builder.rs`:
    - `pub fn build_from_bytes(buf: &[u8], format: &LogFormat) -> LogIndex`.
    - Usa `parse_all_with_format` y guarda `(offset, len)` de cada mensaje válido.
    - Mensajes con error se descartan silenciosamente (ya hay `tracing::warn!` en el parser).
  - Tests inline sobre las tres fixtures sintéticas; conteo esperado igual al del parser.
- **Criterio de aceptación**:
  - Conteo de mensajes en index == conteo de mensajes en parser.
  - `index.messages[i].start == index.messages[i-1].start + index.messages[i-1].len`, módulo bytes entre mensajes.
  - Test sobre `fixtures/real/fix44-om.log` (5419 mensajes) y `fixt11-md.log` (8229).

---

## P2-T03 · Índice secundario configurable

- **Estado**: `[ ]`
- **Depende de**: P2-T02
- **Objetivo**: para un set de tags "hot" (default: `35, 49, 56, 11, 34, 37`), construir `HashMap<(Tag, Value), Vec<MessageIdx>>`.
- **Archivos**:
  - `crates/fixlog-index/src/secondary.rs`:
    - `pub struct HotTags(SmallVec<[u32; 8]>)`.
    - `pub struct SecondaryIndex { by_tag_value: HashMap<(u32, SmallBytes), Vec<u32>> }`.
    - Decisión: `Vec<u32>` inicialmente, no `RoaringBitmap`, para reducir deps. Migrar a `roaring` solo si los tests muestran que la memoria explota.
  - Integrar en `build_from_bytes`: al parsear, anotar cada tag hot.
  - Tests: sobre el fixture real, `secondary.lookup(35, b"D")` devuelve N mensajes conocidos.
- **Criterio de aceptación**:
  - Lookup O(1) por `(tag, value)`.
  - Memoria del índice secundario < 30% del tamaño del archivo para fixtures reales.
  - Bench (opcional) comparando lookup vs full-scan.

---

## P2-T04 · Append-only: `append_from_offset`

- **Estado**: `[ ]`
- **Depende de**: P2-T03
- **Objetivo**: soportar crecimiento incremental del archivo sin reindexar desde cero.
- **Archivos**:
  - `crates/fixlog-index/src/builder.rs`:
    - `impl LogIndex { pub fn append_from_offset(&mut self, buf: &[u8], from: u64); }`.
    - Requiere que `from == self.file_size` (o validar consistencia de lo contrario).
    - Actualiza índice primario y secundario.
  - Test: construir índice sobre `prefix` de un archivo, luego `append_from_offset` sobre el resto, comparar contra índice total construido de una vez → deben ser idénticos.
- **Criterio de aceptación**:
  - Equivalencia build-once vs build-prefix-then-append.
  - No re-parseo del contenido previo.
  - Thread-safety: documentar invariantes en un comentario (el struct no es `Sync`; el caller sincroniza).

---

## P2-T05 · Indexación paralela con rayon

- **Estado**: `[ ]`
- **Depende de**: P2-T02 (P2-T03 puede quedarse single-thread inicialmente)
- **Objetivo**: partir el archivo en chunks, cada worker encuentra su primer `8=FIX`, indexa, se combinan resultados.
- **Archivos**:
  - `crates/fixlog-index/src/parallel.rs`:
    - `pub fn build_from_bytes_parallel(buf: &[u8], fmt: &LogFormat) -> LogIndex`.
    - Chunk boundary: `N = rayon::current_num_threads()`, cada worker procesa `buf[start..end]` pero avanza su `start` hasta el próximo marker `8=FIX` y extiende `end` hasta el próximo marker posterior.
    - Combina via `reduce`; los offsets ya son absolutos (no relativos al chunk).
  - Añadir dep `rayon` en `fixlog-index/Cargo.toml`.
- **Criterio de aceptación**:
  - Resultado idéntico al de `build_from_bytes` single-thread (ordenar primero si hace falta, aunque rayon mantiene orden al reducir secuencialmente).
  - Bench: ≥ 2× speedup en 4 cores sobre `fixt11-md.log` amplificado a 100 MB.
  - No double-counting en solape de chunks.

---

## P2-T06 · Stub y parser del DSL (`fixlog-query`)

- **Estado**: `[ ]`
- **Depende de**: Fase 1 (parser)
- **Objetivo**: crate nuevo con AST + parser para expresiones tipo `35=D AND 55=AAPL`.
- **Gramática (EBNF simplificada)**:
  ```
  expr     := term ( ("AND"|"OR") term )*
  term     := "NOT" term | atom
  atom     := predicate | "(" expr ")"
  predicate := tag op value
  op       := "=" | "!=" | "~"
  tag      := digit+
  value    := string (sin comillas hasta espacio/operador) | "\"" string "\""
  ```
  Precedencia: `NOT` > `AND` > `OR`.
- **Archivos**:
  - `crates/fixlog-query/Cargo.toml` (deps: `fixlog-parser`, `regex`, `thiserror`).
  - `crates/fixlog-query/src/ast.rs`: `pub enum Expr { Pred(Predicate), And(Box<Expr>,Box<Expr>), Or(...), Not(...) }`.
  - `crates/fixlog-query/src/parser.rs`: recursive-descent tokenizer + parser.
  - Tests: ~10 casos incluyendo errores (paréntesis desbalanceados, operador inválido).
- **Criterio de aceptación**: parser acepta la gramática completa y rechaza entradas inválidas con errores posicionales.

---

## P2-T07 · Evaluador de expresiones contra `RawMessage`

- **Estado**: `[ ]`
- **Depende de**: P2-T06
- **Objetivo**: evaluar `Expr` contra un `RawMessage` sin allocations en el hot path.
- **Archivos**:
  - `crates/fixlog-query/src/eval.rs`:
    - `pub fn matches(expr: &Expr, msg: &RawMessage<'_>) -> bool`.
    - Para `op = ~` usa `regex::bytes::Regex` (pre-compilado al parsear la expresión para no recompilar por mensaje).
  - Tests: para cada combinación de operador + estructura, ≥ 2 casos (match + no-match).
- **Criterio de aceptación**:
  - `matches` retorna correctamente para los 10+ tests.
  - Zero allocations por mensaje (verificar con heaptrack o inspección manual).

---

## P2-T08 · Subcomando `grep` en la CLI

- **Estado**: `[ ]`
- **Depende de**: P2-T07
- **Objetivo**: `fixlog grep <file> --filter "expr" [--format json|pretty]`.
- **Archivos**:
  - `crates/fixlog-cli/src/commands/grep.rs`.
  - Pipeline: sniff → parse stream → `matches(expr, &msg)` → print.
  - Reutiliza el renderizador de `parse` (mismo formato pretty/json).
  - `fixlog-core` re-exporta `fixlog_query::{parse_expr, matches, QueryError}`.
- **Criterio de aceptación**:
  - Output correcto sobre fixtures reales (`35=8 AND 150=F` en `fix44-om.log`).
  - Exit code 0 si matchea ≥ 1, 1 si 0 matches (convención estilo `grep`).
  - `--format json` produce JSONL válido (`jq empty`).

---

## P2-T09 · Tailing con `notify` + `--follow`

- **Estado**: `[ ]`
- **Depende de**: P2-T04 (append), P2-T08 (grep)
- **Objetivo**: `fixlog grep <file> --follow --filter "..."` se queda leyendo el archivo como `tail -f`.
- **Archivos**:
  - `crates/fixlog-cli/src/commands/grep.rs` (extender).
  - Nueva dep en CLI: `notify = "7"`.
  - Pseudocódigo:
    1. Abrir mmap inicial, imprimir lo que haga match.
    2. Con `notify`, watch del archivo. En cada evento: re-mmap (el tamaño cambió), parsear desde `previous_size`, evaluar, imprimir.
    3. Detectar truncado (tamaño nuevo < previo) → reabrir desde 0.
    4. Detectar rotación (inode cambió) → reabrir el path y continuar desde 0.
  - Tests: no unit (requiere fs events); test manual documentado en `docs/agent/state.md`.
- **Criterio de aceptación**:
  - Añadir líneas a un fixture con `tee -a` mientras `--follow` está corriendo y verificar que aparecen en stdout en <1s.
  - Rotación simulada (`mv a b && touch a && cat b >> a`) no crashea.

---

## P2-T10 · Subcomando `index` (opcional, stretch)

- **Estado**: `[ ]`
- **Depende de**: P2-T05
- **Objetivo**: `fixlog index <file>` construye índice y lo serializa a `<file>.fixlog-idx`.
- **Archivos**:
  - `crates/fixlog-cli/src/commands/index.rs`.
  - Serialización: `bincode` o `rkyv`. Decisión en la tarea: si `rkyv` complica build times, usar `bincode 2`.
  - Hash del archivo (xxhash o blake3) embebido en el header del índice para invalidar si cambió.
- **Criterio de aceptación**:
  - Reabrir con índice en caché < 1s para un archivo de 100 MB.
  - Cache inválido si cambia el archivo → fallback a rebuild + aviso por `tracing::warn!`.

Puede moverse a Fase 5 si complica el scope. No bloquea P2-T01…T09.

---

## Criterios de "Fase 2 completa"

- P2-T01 a P2-T09 en `[x]` (T10 opcional).
- `cargo test --all` pasa.
- `cargo clippy --all-targets --all-features -- -D warnings` pasa.
- `cargo fmt --all --check` pasa.
- Bench `parse` de Fase 1 no regresa (±5%).
- `docs/agent/state.md` actualizado con el estado y métricas de Fase 2.
- Commit de cierre: `chore(phase2): close Fase 2 — Indexación + Tailing + Query`.
- Tag git `v0.2.0-phase2`.

---

## Riesgos y decisiones deferidas

- **Repeating groups**: siguen fuera de scope. El evaluador hace match por `(tag, value)` sin entender la estructura jerárquica. Si un usuario filtra `448=BROKER1`, matchea en cualquier `PartyID` del mensaje — esto es lo deseable para un CLI tipo `grep`.
- **Dic-aware filtering**: la versión inicial del DSL solo acepta tags numéricos (`35=D`). Soporte para nombres (`MsgType=NewOrderSingle`) espera a Fase 3/5 cuando el diccionario esté más integrado.
- **RoaringBitmap**: ver P2-T03. Default a `Vec<u32>`; cambiar solo si la telemetría lo pide.
- **Mmap vs read**: seguimos con `memmap2` como en Fase 1. Con `--follow`, re-mmap en cada cambio de tamaño es barato (kernel remapea páginas).
- **Async / tokio**: no se introduce. Notify expone canal `Receiver`; el bucle principal es síncrono.

---

## Notas sobre ejecución

- **Una tarea por sesión**; `/clear` antes de la siguiente.
- Invocar `/phase-task P2-T0<N>` (si se adapta el comando) o empezar la tarea explícitamente citando el ID.
- Invocar `/validate` antes de cerrar cada tarea.
- Invocar `/fixture-check` al cerrar P2-T02, P2-T05, P2-T08, P2-T09.
- Commit al cerrar cada tarea con conventional commit (`feat(index):`, `feat(query):`, `feat(cli):`).
