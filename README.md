# fixlog

Parser y viewer de logs FIX (Financial Information eXchange) en Rust. Pensado para procesar millones de mensajes de forma eficiente, con zero-copy desde el archivo hasta la salida.

**Estado actual**: Fase 1 — CLI con `sniff`, `parse`, `stats` sobre FIX 4.4, FIXT.1.1 y FIX 5.0SP2. TUI interactivo llegará en fases posteriores.

## Qué puede hacer hoy

- **Auto-detectar formato** de un log: separador (`SOH`, `|`, `^`, `;`), prefijos de línea (timestamps, logback), line endings.
- **Parsear sin asumir un formato único**: un mismo binario procesa logs QuickFIX puros, logs re-renderizados con `|`, logs envueltos en prefijos variables.
- **Resolver tags a nombres**: tag `54` → `Side`, valor `1` → `BUY`. Múltiples diccionarios (FIX 4.4, FIXT.1.1, FIX 5.0SP2) con selección automática según `BeginString` y `ApplVerID`.
- **Resumir un archivo** de millones de mensajes en segundos: sesiones, rango temporal, top MsgTypes.
- **Emitir JSON** línea-por-línea para piping a `jq` u otras herramientas.

## Instalación

Requisitos: Rust estable reciente (probado con 1.95, edition 2024).

```sh
git clone <repo> fixlog
cd fixlog
cargo build --release -p fixlog-cli
```

El binario queda en `target/release/fixlog`. Opcionalmente:

```sh
cargo install --path crates/fixlog-cli
```

## Uso

Los ejemplos asumen que hiciste `alias fixlog=./target/release/fixlog` o instalaste con `cargo install`.

### `fixlog sniff` — detectar formato

```sh
$ fixlog sniff fixtures/synthetic/with_timestamp_prefix.log
File:           fixtures/synthetic/with_timestamp_prefix.log
Separator:      SOH (0x01)
Line prefix:    fixed 24 bytes
Line ending:    LF (\n)
Encoding:       UTF-8 / ASCII
Msg boundary:   line
```

### `fixlog parse` — imprimir mensajes

Formato `pretty` (por defecto) con nombres de tags y labels de enums:

```sh
$ fixlog parse fixtures/synthetic/minimal_4.4.log --first 1
Message @ offset 0 (95 bytes) Logon
      8  BeginString     = FIX.4.4
      9  BodyLength      = 73
     35  MsgType         = A (Logon)
     34  MsgSeqNum       = 1
     49  SenderCompID    = SENDER
     52  SendingTime     = 20260416-13:30:00.000
     56  TargetCompID    = TARGET
     98  EncryptMethod   = 0 (NONE_OTHER)
    108  HeartBtInt      = 30
    141  ResetSeqNumFlag = Y (YES_RESET_SEQUENCE_NUMBERS)
     10  CheckSum        = 161
```

Formato `json` (JSONL, un objeto por línea) — pipe-friendly:

```sh
$ fixlog parse my.log --format json | jq '. | select(.msg_type_name == "NewOrderSingle") | .tags[] | select(.name == "Symbol").value' | sort -u
```

Opciones:
- `--first N` — procesar solo los primeros `N` mensajes.
- `--format pretty|json` — formato de salida.

### `fixlog stats` — resumen ejecutivo

```sh
$ fixlog stats my-fix44-trading.log
File:            my-fix44-trading.log
Messages parsed: 5419
Parse errors:    0
Time range:      20260416-13:30:04.713 .. 20260416-19:42:09.279
Sessions:        2
        3354  BROKER → OMS
        2065  OMS → BROKER
Message types:   6 (6 shown)
        3124  8    ExecutionReport
        1535  D    NewOrderSingle
         489  0    Heartbeat
         268  F    OrderCancelRequest
           2  A    Logon
           1  9    OrderCancelReject
```

### Verbosidad

```sh
fixlog -v  parse my.log   # tracing nivel info en stderr
fixlog -vv parse my.log   # tracing nivel debug (incluye checksum mismatches, etc.)
```

También respeta `RUST_LOG` si quieres control fino: `RUST_LOG=fixlog_parser=debug fixlog parse my.log`.

## Formatos soportados

| Aspecto | Soporte |
|---------|---------|
| Versiones FIX | 4.4, FIXT.1.1 (session) + 5.0SP2 (application) |
| Separador | SOH (`\x01`), `\|`, `^`, `;` — autodetectado |
| Prefijos de línea | cualquiera (timestamp, logback, PID…) — el parser escanea `8=FIX` |
| Line ending | LF, CRLF |
| Encoding | UTF-8 / ASCII (bytes no-UTF-8 se muestran lossy) |
| Checksums inválidos | **no-fatales**: se emite el mensaje y se loggea en `-vv` |

Añadir una versión FIX extra es ~10 líneas: descargar el XML de QuickFIX/J a `dictionaries/` y añadirlo a `DICTIONARIES` en `crates/fixlog-dict/build.rs`.

## Arquitectura

Workspace Cargo con crates desacoplados:

```
crates/
├── fixlog-format/   # Sniffer del formato (separador, prefijo, encoding)
├── fixlog-parser/   # Parser zero-copy → RawMessage
├── fixlog-dict/     # Diccionarios FIX (build.rs genera desde XML)
├── fixlog-core/     # Facade re-exportando los anteriores
└── fixlog-cli/      # Binario `fixlog`
```

El parser no sabe nada del diccionario — emite pares `(tag, &[u8])` y el resolver decora con nombres. Esto permite reindexar millones de mensajes sin materializar strings.

Más detalles en [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) (intención de diseño) y [`docs/agent/`](docs/agent/) (estado actual, organizado para LLMs).

## Desarrollo

```sh
cargo test --all                             # 35 tests
cargo clippy --all-targets -- -D warnings    # lint estricto
cargo fmt --all                              # formato
cargo run -p fixlog-cli -- sniff <file>      # ejecutar sin instalar
```

Fixtures:
- `fixtures/synthetic/` — logs pequeños versionados para tests golden.
- `fixtures/real/` — gitignored, logs reales para validación manual.

Ver [`fixtures/README.md`](fixtures/README.md) para detalles del corpus.

## Roadmap

- **Fase 1** (actual): CLI + parser + diccionarios.
- **Fase 2**: indexación append-friendly con `rayon` + `roaring` + DSL de filtros.
- **Fase 3**: TUI estilo fixparser.targetcompid.com (ratatui + crossterm).
- **Fase 4**: tailing en vivo (`notify`).
- **Fase 5**: pulido, docs, release.

Detalle completo en [`docs/ROADMAP.md`](docs/ROADMAP.md).

## Licencia

MIT OR Apache-2.0.
