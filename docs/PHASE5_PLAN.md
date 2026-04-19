# Fase 5 — Extensibilidad y polish

Plan atómico y ordenado para la Fase 5. Se arranca con Fases 1–4 cerradas + rediseño TUI (Fases A/B, 2026-04-18) + **4 items de Fase 5 ya aterrizados** en el commit parcial del 2026-04-18 (`fixlog-render`, FIX 5.0/SP1 dicts, `QueryExpr: Clone`, analysis `parse_one_with_format`). Esta Fase 5 formaliza lo ya hecho y estructura el backlog restante.

**Contexto**: Fase 4 entregó las herramientas de análisis semántico (sessions, orders, histogram, diff, bookmarks, export, hot-tag pre-filter). Fase 5 cierra la transición de "prototipo de alto rendimiento" a "herramienta production-ready": persistencia de configuración y de caché, diccionarios extensibles por el usuario, tabs / multi-file, y un `--strict` explícito para entornos que no toleran checksums rotos. Priorizamos los items que habilitan ciclos de debugging más rápidos (reopen de logs grandes en <1 s, bookmarks persistidos) antes que features de scripting.

**Convención de estado**:

- `[ ]` pendiente
- `[~]` en curso
- `[x]` completa

---

## Resumen de entregables

1. **Ya aterrizado (2026-04-18)**:
   - Crate `fixlog-render` (writers compartidos `write_pretty` / `write_jsonl` / `write_fix` / `write_csv_*`).
   - Diccionarios FIX 5.0 y 5.0 SP1 con routing por `ApplVerID` 7 y 8.
   - `QueryExpr: Clone` via `Arc<Regex>`; sin re-parsing en `Esc` / `n` / `N`.
   - `fixlog-analysis` + `fixlog-cli orders` resuelven logs pipe-separated (regression test dedicado).
2. **Por aterrizar**:
   - `fixlog index` subcommand con caché serializado `<file>.fixlog-idx` (hash-invalidated).
   - Flag `--strict` en `parse` / `grep` que escala checksum mismatches a error.
   - Config persistente `~/.config/fixlog/config.toml` (theme, hot tags, saved filters, keybindings, bookmarks).
   - Diccionarios híbridos: embebidos como fallback + override desde `~/.config/fixlog/dictionaries/<VERSION>.xml` con hot reload opcional.
   - Multi-file / tabs en el TUI (`gt` / `gT` vim-style) + diff entre archivos.
   - Repeating groups (`NoMDEntries`, `NoAllocs`, `NoPartyIDs`, …) en el resolver.
   - Symbolic query names (`MsgType=NewOrderSingle`) — promovido desde P4-T15 diferido.

**Baselines a respetar** (no regresar ±5%, de `docs/agent/state.md` 2026-04-17):

- `parse_real/fixt11_md` ~1.1 GiB/s · `parse_real/fix44_om` ~314 MiB/s.
- `index_amplified/parallel_40MiB` ~1.08 GiB/s.
- `tui_frame/list_detail_status_200x50` ~737 µs · `tui_bootstrap/1M_messages` ~123 ms.
- `tui_filter/apply_35eqD_1M` ~156 µs (hot-tag pushdown).
- `analysis/session_build_1M` ~1.16 s · `order_lookup_1M` ~5 µs · `histogram_build_1M` ~500 ms.

**Nuevos targets Fase 5**:

- **Reopen con caché**: <1 s para 100 MiB+ cuando `<file>.fixlog-idx` existe y su hash coincide.
- **Cold open no regresa**: tiempo de `bootstrap` sin caché queda dentro del ±5% de la baseline de Fase 3.
- **Hybrid dict override**: cargar XML custom <50 ms adicionales sobre bootstrap.

---

## Invariantes que NO romper

Heredadas de Fases 1–4. Si una PR las rompe, re-hacer — no flexibilizar.

- **Parser zero-copy**: `RawMessage.raw` / `tags` siguen referenciando `&[u8]` del mmap.
- **`LogIndex.consumed` ≠ `buf.len()`** y `append_from_offset(buf, from)` exige `from == self.consumed`.
- **`fixlog-parser` no depende de `fixlog-dict`**; los adapters de symbolic names viven en el lado que resuelve.
- **`fixlog-render` depende sólo de `dict` + `parser`**; no añadir deps de análisis ni de TUI.
- **Secondary index representation** (`HashMap<(tag, SmallVec<[u8;16]>), Vec<u32>>`) sigue intacta — Roaring sigue diferido. Nuevos callers pasan por `lookup(tag, value)`.
- **Mensajes corruptos = warn + skip** en todos los analizadores. Cambio a `--strict` es opt-in y sólo afecta exit code + mensaje, no el builder del índice.
- **TUI: `#![deny(unsafe_code)]`** con `#[allow]` sólo en `io.rs`. Event loop síncrono. Multi-tab NO introduce `tokio`.
- **Config file format**: TOML. No YAML, no JSON. Resolución con `dirs::config_dir()`; fallback a `$XDG_CONFIG_HOME/fixlog/config.toml` explícito.
- **Caché de índice**: formato con `rkyv` + content hash (blake3). Mismatch → rebuild, nunca fail.

---

## P5-T01 · Crate `fixlog-render`

- **Estado**: `[x]` (2026-04-18)
- **Objetivo**: deduplicar writers pretty/jsonl/fix/csv entre `fixlog-cli` y `fixlog-tui`.
- **Entregado**:
  - Nuevo crate `crates/fixlog-render` v0.1.0 con `write_pretty` / `write_jsonl` / `write_fix` / `write_csv_header` + `write_csv_row`.
  - Depende sólo de `fixlog-dict` + `fixlog-parser`.
  - Re-exportado como `fixlog_core::render`.
  - Consumers: `fixlog-cli` (`parse`, `grep`) y `fixlog-tui` (`:export`).
- **Resultado**: ~200 LOC duplicados eliminados.

---

## P5-T02 · Diccionarios FIX 5.0 y 5.0 SP1

- **Estado**: `[x]` (2026-04-18)
- **Objetivo**: soportar logs con `ApplVerID` 7 y 8.
- **Entregado**:
  - `dictionaries/FIX50.xml` y `dictionaries/FIX50SP1.xml` vendored desde QuickFIX v1.15.1.
  - `FixVersion::{Fix50, Fix50Sp1}` en `fixlog-dict`.
  - Chains `CHAIN_FIXT11_FIX50` y `CHAIN_FIXT11_FIX50SP1`.
  - `chain_for` rutea `ApplVerID=7` → Fix50, `ApplVerID=8` → Fix50Sp1. SP2 se mantiene como default para `ApplVerID` desconocido o ausente.

---

## P5-T03 · `QueryExpr: Clone` (evitar re-parse en TUI)

- **Estado**: `[x]` (2026-04-18)
- **Objetivo**: permitir snapshot de filtros activos y reutilización de expresiones compiladas en `n` / `N`.
- **Entregado**:
  - `Op::Re` envuelve `Arc<Regex>` (no `Regex` directo).
  - `QueryExpr: Clone` via derive estándar.
  - `FilterSnapshot` almacena la expresión compilada; `iterate_search` usa `search_last.clone()`.
- **Resultado**: `Esc` rollback + `n`/`N` no re-parsean. Frame bench neutral (~156 µs).

---

## P5-T04 · Analysis + CLI `parse_one_with_format` migration

- **Estado**: `[x]` (2026-04-18)
- **Objetivo**: `fixlog-analysis` y `fixlog-cli orders` deben resolver logs pipe-separated (o cualquier otro separador no-SOH).
- **Entregado**:
  - `SessionMap::build`, `OrderTimeline::build`, `Histogram::build` y `fixlog-cli orders` consumen `parse_one_with_format` en lugar de `parse_one` (que asume SOH).
  - Regression test: `crates/fixlog-analysis/tests/pipe_separated.rs`.

---

## P5-T05 · `fixlog index` subcommand + caché serializado

- **Estado**: `[ ]`
- **Depende de**: nada — self-contained.
- **Objetivo**: serializar `LogIndex` + metadata a `<file>.fixlog-idx` para reopen <1 s de archivos grandes.
- **Archivos**:
  - `crates/fixlog-index/src/cache.rs` nuevo módulo con `save(index, path)` / `load(path) -> Option<LogIndex>`.
  - `crates/fixlog-cli/src/commands/index.rs` nuevo: subcomando `fixlog index <file>`.
  - `crates/fixlog-cli/src/main.rs`: wiring del subcomando.
  - `crates/fixlog-tui/src/state.rs`: `bootstrap` intenta cargar caché antes de `build_from_bytes_parallel`.
- **Diseño**:
  - Formato: `rkyv` sobre `LogIndex` + blake3 hash de los primeros/últimos 1 MiB del archivo + `file_size: u64` + `format: LogFormat` + `version: u32` (schema).
  - Path del caché: `<file>.fixlog-idx` al lado del log (simple, mismo fs). Si no es writable, silently skip.
  - Invalidación: mismatch de hash **o** mismatch de `file_size` **o** mismatch de `version` → rebuild; nunca fail.
  - `--strict-cache` (flag nuevo): fail si el caché existe pero es inválido, en vez de rebuild silencioso (útil en CI).
- **Criterio de aceptación**:
  - `fixlog index <file>` crea el `.fixlog-idx` en <5× el tiempo de `build_from_bytes_parallel` sobre el mismo archivo.
  - Reopen de `fixtures/real/fixt11-md.log` (8.7 MiB) con caché <100 ms.
  - Bench `cache_reload_100MiB` <1 s.
  - Hash tamper → rebuild silencioso.
  - Nuevo test integration que corrompe el caché y verifica rebuild.

---

## P5-T06 · Flag `--strict` en `parse` / `grep`

- **Estado**: `[ ]`
- **Depende de**: nada.
- **Objetivo**: escalar checksum mismatches a error (exit 2) en vez de `warn + skip`. Feature solicitado en sesiones anteriores.
- **Archivos**:
  - `crates/fixlog-cli/src/commands/parse.rs` y `grep.rs`: añadir `--strict` al `clap` derive.
  - `crates/fixlog-parser/src/lib.rs`: exponer un `StrictMode` que el CLI consume; el builder de índice no cambia.
- **Diseño**:
  - Sin `--strict`: comportamiento actual (mensaje emitido, checksum mismatch loggeado a `-vv`).
  - Con `--strict`: primer checksum mismatch → imprime mensaje a stderr + exit code 2.
  - El flag NO afecta al TUI ni al indexer: el indexer debe seguir tolerando logs sucios.
- **Criterio de aceptación**:
  - Nuevo test integración usa un fixture sintético con checksum roto, verifica exit 2 con `--strict` y exit 0 sin él.
  - Documentación en `README.md` sección Verbosidad / Flags.

---

## P5-T07 · Config persistente `~/.config/fixlog/config.toml`

- **Estado**: `[ ]`
- **Depende de**: nada.
- **Objetivo**: permitir al usuario personalizar theme, hot tags, filtros guardados, keybindings y persistir bookmarks.
- **Archivos**:
  - Nuevo crate `crates/fixlog-config` (library): structs `Config`, `ThemeConfig`, `Keybindings`, con `serde + toml`. Depende de `fixlog-query` para validar `saved_filters`.
  - `crates/fixlog-tui/src/theme.rs`: consumir `ThemeConfig` al bootstrap.
  - `crates/fixlog-index/src/secondary.rs`: hot tags configurables.
  - `crates/fixlog-tui/src/state.rs`: cargar y persistir `bookmarks` en `~/.config/fixlog/bookmarks.<sha1(log_path)>.toml` (per-log).
- **Diseño**:
  - Path resolution: `dirs::config_dir().join("fixlog/config.toml")` con fallback explícito a `$XDG_CONFIG_HOME`.
  - Missing config = defaults. Malformed TOML = warn en stderr, fallback a defaults, continuar.
  - Schema versionado (`version = 1`).
  - Ejemplo embebido documentado en `README.md` sección "Configuración".
  - Keybindings override por acción (`quit = "q"`, `toggle_mode = "F"`, etc.) — rebinding de todo requiere parse más robusto, arrancar con subset.
- **Criterio de aceptación**:
  - TUI carga config custom correctamente; si el archivo no existe, funciona igual que antes.
  - Bookmarks sobreviven reapertura del mismo archivo.
  - Cambios incompatibles de schema dan warn, no crash.

---

## P5-T08 · Diccionarios híbridos (override por usuario)

- **Estado**: `[ ]`
- **Depende de**: P5-T07 (resolver `~/.config/fixlog/`).
- **Objetivo**: permitir al usuario sobreescribir diccionarios embebidos con XMLs en `~/.config/fixlog/dictionaries/FIX44.xml` para tags custom de su broker / exchange.
- **Archivos**:
  - `crates/fixlog-dict/src/resolver.rs`: estrategia de merge. Embebido = fallback; override = hoja.
  - `crates/fixlog-dict/src/runtime.rs` nuevo: parseo runtime de XML (hoy sólo se parsea en build.rs).
  - Opcional: hot reload con `notify` detrás de un flag `--hot-reload-dicts`.
- **Diseño**:
  - Load order al bootstrap del dict resolver:
    1. Embedded (generado por build.rs).
    2. `~/.config/fixlog/dictionaries/<VERSION>.xml` si existe — merge por tag, el override gana.
  - Tags desconocidos siguen renderizando `?` en pretty / `null` en json.
  - Performance: parseo XML por versión <10 ms; cache en memoria, nunca se re-parsea en el mismo proceso salvo hot reload.
- **Criterio de aceptación**:
  - Fixture sintético con tag custom del usuario (ej. 6001) resuelve a nombre en pretty.
  - Sin override, comportamiento idéntico a hoy (tests existentes pasan).
  - Hot reload detecta modificación al XML y actualiza la resolución en vivo (si flag on).

---

## P5-T09 · Multi-file / tabs en el TUI

- **Estado**: `[ ]`
- **Depende de**: nada crítica; más fácil con P5-T05 (caché) para no pagar bootstrap de cada tab.
- **Objetivo**: abrir varios logs a la vez; `gt`/`gT` vim-style para navegar tabs; diff entre archivos de sesiones distintas.
- **Archivos**:
  - `crates/fixlog-tui/src/state.rs`: `pub tabs: Vec<TabState>`, `pub active_tab: usize`. `AppState` actual se convierte en `TabState`.
  - `crates/fixlog-tui/src/input.rs`: `gt` / `gT` via two-key sequences (reutiliza `pending_prefix`).
  - `crates/fixlog-cli/src/commands/tui.rs`: aceptar múltiples paths.
  - Nuevo overlay o modo diff para comparar sessions entre archivos (extender diff view existente).
- **Diseño**:
  - Cada tab posee su `Arc<Mmap>`, `LogIndex`, filter state, bookmarks.
  - Follow opera sólo sobre el tab activo (para no pelearse con el watcher stat-based).
  - Title bar muestra tabs como `[1:file1.log] 2:file2.log` (activo resaltado).
  - Memory budget: abrir 10 archivos de 100 MiB cada uno = 1 GiB mmap — documentar como límite pragmático.
- **Criterio de aceptación**:
  - `fixlog tui a.log b.log c.log` abre 3 tabs.
  - `gt` y `gT` ciclan entre tabs.
  - Filter / bookmarks son per-tab (no se cruzan).
  - Bench: switching de tab <5 ms en 3× 1M msgs.

---

## P5-T10 · Repeating groups en el resolver

- **Estado**: `[ ]`
- **Depende de**: nada.
- **Objetivo**: parseo correcto de `NoMDEntries`, `NoAllocs`, `NoPartyIDs`, etc., para que el detail panel muestre la estructura anidada.
- **Archivos**:
  - `crates/fixlog-dict/src/chain.rs`: metadatos de grupos (primer tag del grupo, miembros, terminador implícito).
  - `crates/fixlog-dict/src/resolver.rs`: `resolve` agrupa fields por repeating-group cuando la chain lo indica.
  - `crates/fixlog-tui/src/view/detail.rs`: indentación visual.
- **Diseño**:
  - La info ya vive en los XML de QuickFIX (`<group>`). El build.rs de `fixlog-dict` hoy la ignora — parsearla y exponerla en `ChainField`.
  - Parser permanece ignorante de groups (emite tags planos, como hoy).
  - Resolver detecta el NumInGroup counter y agrupa los N bloques siguientes.
  - Edge: grupos mal formados (counter dice 3 pero hay 2) → fallback a presentación plana + warn.
- **Criterio de aceptación**:
  - Fixture MD con `NoMDEntries` se renderiza con bloques visualmente anidados.
  - Performance: resolve <5% más lento que hoy.

---

## P5-T11 · Symbolic query names (ex P4-T15)

- **Estado**: `[ ]`
- **Depende de**: nada.
- **Objetivo**: permitir `MsgType=NewOrderSingle`, `Side=Buy` en el DSL sin acoplar `fixlog-query` a `fixlog-dict`.
- **Archivos**:
  - `crates/fixlog-query/Cargo.toml`: feature flag `symbolic-names` (off por default).
  - `crates/fixlog-query/src/adapter.rs` nuevo (gated por feature): `resolve_symbols(expr: &str, chain: &Chain) -> String` que reemplaza nombres por tags antes del parse.
  - `crates/fixlog-cli` + `crates/fixlog-tui`: activar la feature y llamar al adapter antes de `parse_query`.
- **Diseño**:
  - Reemplazo léxico (no AST). `MsgType=NewOrderSingle` → `35=D` pre-parse.
  - Ambiguedad (ej. `Side` entre FIX44 y FIX50) resuelta por la chain activa del contexto (log abierto).
  - Error cuando el símbolo no existe en la chain → status bar con sugerencia ("unknown name `NewORderSingle` — did you mean `NewOrderSingle`?").
- **Criterio de aceptación**:
  - `fixlog grep my.log --filter "MsgType=NewOrderSingle"` funciona.
  - `:filter Side=Buy` en TUI funciona.
  - Sin la feature activa, el core DSL queda sin cambios (tests de `fixlog-query` siguen pasando como hoy).

---

## Criterios de "Fase 5 completa"

- P5-T01..T04 en `[x]` (ya lo están, pendiente formalización documental).
- P5-T05..T11 en `[x]` — **estado actual: 0 de 7**.
- `cargo test --all` pasa con todos los nuevos tests de integración.
- `cargo clippy --all-targets --all-features -- -D warnings` pasa.
- `cargo fmt --all --check` pasa.
- Baselines Fase 1-4 no regresan (±5%).
- `cache_reload_100MiB` <1 s (P5-T05).
- `reopen_with_cache` <100 ms sobre fixture real 8.7 MiB.
- `docs/agent/state.md` actualizado por tarea.
- `README.md` sección Configuración documentada (ejemplo + paths).
- Commit de cierre: `chore(phase5): close Fase 5 — Extensibilidad y polish`.
- Tag git `v0.5.0-phase5` (usuario decide cuándo).

---

## Riesgos y decisiones deferidas

- **rkyv vs bincode para cache**: `rkyv` es zero-copy en deserialización (mmap-friendly) pero añade dep. `bincode` es más liviano. Default: `rkyv` por la mmap friendliness; re-evaluar si la dep tree crece demasiado.
- **Scripting (Rhai)**: el ROADMAP.md menciona scripting opcional en Fase 5. Por simplicidad está **fuera de scope** de este plan; si el usuario lo prioriza, se añade como P5-T12.
- **Auto-update del caché bajo `--follow`**: P5-T05 escribe el caché una vez; si el log crece bajo follow, ¿invalida o extiende el caché? Default: invalida al salir limpio (SIGTERM handler) y reescribe. Warn si el proceso termina por señal no capturada.
- **Persistent bookmarks y rotación de logs**: el key del archivo es `sha1(canonical_path)`. Si el usuario rota manualmente y preserva el nombre, los bookmarks "persisten" pero apuntan a ordinals viejos — comportamiento sorprendente pero aceptable.
- **Keybinding rebinding completo**: permitir re-mapear toda la tabla (incluyendo two-key sequences) requiere un mini lenguaje en el TOML. Arrancar con rebinding plano (solo keys de acción única) y postergar el resto.
- **Hybrid dicts y performance**: parsear XMLs runtime añade ~5–50 ms al bootstrap. Si un usuario tiene 4 versiones override, puede llegar a 200 ms extra. Cachear el resultado parseado en `~/.config/fixlog/dict-cache.bin` si se vuelve problemático.
- **Multi-file memory budget**: mmap no reserva físicamente, pero el OS cargará páginas al navegar. Documentar el efecto y ofrecer un `:close` para liberar tabs.
- **Symbolic names y case sensitivity**: los XML de QuickFIX usan CamelCase exactamente. Decidir si aceptar `newordersingle` o sólo `NewOrderSingle`. Default: case-sensitive (más simple, evita ambigüedad).

---

## Notas sobre ejecución

- **Una tarea atómica por sesión**; `/clear` antes de la siguiente.
- **Plan Mode obligatorio** para P5-T05 (formato de caché), P5-T07 (schema de config), P5-T09 (refactor a multi-tab), P5-T10 (contrato de repeating groups). El resto puede avanzar directo.
- Invocar `/validate` antes de cerrar cada tarea.
- Invocar `/fixture-check` al cerrar P5-T05, P5-T06, P5-T08, P5-T10 (tareas que tocan el camino mmap → resolver → render contra fixtures reales).
- Commit al cerrar cada tarea con conventional commit (`feat(cache):`, `feat(config):`, `feat(dict):`, `feat(tui):`, `feat(query):`).
- Al cerrar cada tarea: actualizar `docs/agent/state.md` en ese mismo commit.
