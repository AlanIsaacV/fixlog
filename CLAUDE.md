# fixlog

Parser y viewer TUI de logs FIX (Financial Information eXchange) en Rust. Target: procesar millones de mensajes FIX de forma eficiente, con TUI interactivo estilo fixparser.targetcompid.com pero local.

## Stack

- **Lenguaje**: Rust
- **TUI**: `ratatui` + `crossterm`
- **Parsing**: implementación propia zero-copy con `&[u8]`
- **Concurrencia**: `rayon` para indexación paralela
- **I/O**: `memmap2` para archivos grandes, `notify` para watching
- **Errores**: `thiserror` (biblioteca) + `anyhow` (binario)
- **Logging**: `tracing` + `tracing-subscriber`
- **Benchmarks**: `criterion`
- **Testing**: fixtures reales en `fixtures/` + tests sintéticos

## Estructura

Workspace Cargo con crates independientes:

```
crates/
├── fixlog-format/   # Format sniffer (separador, prefijo, encoding)
├── fixlog-parser/   # Parser zero-copy → RawMessage
├── fixlog-dict/     # Diccionarios FIX (build.rs genera desde XML)
├── fixlog-index/    # Indexación + tailing (append-only)
├── fixlog-query/    # DSL de filtros (AST + evaluador)
├── fixlog-core/     # Re-exporta todo lo anterior
├── fixlog-cli/      # Binario CLI (Fases 1-2)
└── fixlog-tui/      # Binario TUI (Fase 3+)
```

Detalles completos: ver `docs/ARCHITECTURE.md`.

## Convenciones

**Código**:

- Zero-copy por defecto. Parsers y resolvers trabajan con `&[u8]`, no `String`.
- Nunca `panic!` ni `unwrap()` en código de producción. En tests está permitido.
- Errores públicos con `thiserror`. Errores internos del binario con `anyhow`.
- `unsafe` requiere comentario `// SAFETY:` justificando invariantes.
- No allocaciones en hot paths sin justificación medible.
- Preferir `&str` sobre `String`, `&[T]` sobre `Vec<T>` en APIs públicas.

**Formato y calidad**:

- `cargo fmt --all` antes de cada commit.
- `cargo clippy --all-targets --all-features -- -D warnings` debe pasar.
- `cargo test --all` debe pasar.
- Coverage no es requisito estricto pero los paths críticos del parser deben tener tests.

**Commits**:

- Conventional commits: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`, `perf:`, `bench:`.
- Un commit por unidad lógica, no commits monstruo.

**Tests**:

- Tests unitarios en el mismo archivo bajo `#[cfg(test)] mod tests`.
- Tests de integración en `crates/<crate>/tests/`.
- Fixtures reales en `fixtures/` (nunca committear datos sensibles, anonimizar antes).
- Golden tests para el parser: input FIX → output JSON esperado.

## Comandos frecuentes

```bash
cargo build --all                                    # Build todo
cargo test --all                                     # Run all tests
cargo clippy --all-targets -- -D warnings            # Lint estricto
cargo fmt --all                                      # Format
cargo bench -p fixlog-parser                         # Bench parser
cargo run -p fixlog-cli -- sniff fixtures/sample.log # Run CLI
```

## Roadmap y plan actual

- Roadmap completo de 5 fases: `docs/ROADMAP.md`
- Plan detallado de la fase en curso: `docs/PHASE1_PLAN.md`
- Arquitectura de módulos (intención de diseño): `docs/ARCHITECTURE.md`

Fase actual: **Fase 1 — Core Parser (CLI, sin TUI)**.

## Documentación LLM-oriented (cargar selectivamente)

`docs/agent/INDEX.md` es un índice modular para cargar **solo** las secciones relevantes a la tarea en curso. Empieza ahí para cualquier trabajo no trivial — tiene una tabla que mapea tipo de tarea → archivos a leer.

- `docs/agent/state.md` — estado real del proyecto (autoritativo sobre `PHASE1_PLAN.md` cuando discrepan).
- `docs/agent/crates/{parser,format,dict,cli,core}.md` — internals de cada crate.
- `docs/agent/reference/{fix-protocol,fixtures}.md` — primer de protocolo FIX y catálogo de fixtures.
- `docs/agent/patterns.md` — idioms transversales (zero-copy, errores, tracing, mmap).

## Principios de colaboración con el agente

- **Plan Mode primero** para tareas arquitectónicas. No codear sin confirmar approach.
- **Una tarea atómica por sesión**. Al terminar, `/clear` antes de la siguiente.
- **Tests antes de avanzar**. Cada tarea del PHASE\*\_PLAN.md tiene criterio de aceptación.
- **Subagentes bajo demanda**. Preferir `Task()` para delegación dinámica sobre subagentes custom.
- **Fixtures reales siempre**. Validar cambios contra `fixtures/` antes de declarar una tarea completa.

## Anti-patrones

- NO asumir un único formato/separador/versión FIX. El sniffer decide.
- NO acoplar parser a diccionario. El parser emite tags crudos; el diccionario resuelve.
- NO cargar archivos completos a RAM. Usar mmap desde Fase 1.
- NO escribir el parser para un caso feliz. Los logs reales tienen mensajes truncados, líneas vacías, checksums rotos.
