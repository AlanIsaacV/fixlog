# Arquitectura — fixlog

## Principios de diseño

1. **Separación de capas**: cada crate tiene una responsabilidad. Un cambio en el diccionario no recompila el parser. Un cambio en el TUI no toca el indexador.
2. **Zero-copy por defecto**: los tipos intermedios referencian el buffer original (`&'a [u8]`) en vez de copiarlo. Solo se materializan `String`/`Vec` al presentar.
3. **Agnóstico a versión en el parser**: el parser no sabe qué es un `Side` o un `OrderType`. Solo sabe de pares `tag=value`. La interpretación vive en el resolver.
4. **Append-friendly desde Fase 1**: aunque Fase 1 no implementa tailing, los tipos base ya asumen que el archivo puede crecer. Esto evita refactors dolorosos en Fase 2.
5. **Errores explícitos**: nada de `Option` para indicar fallos. `Result<T, E>` con errores tipados.

## Estructura de workspace

```
fixlog/
├── Cargo.toml                      # Workspace
├── CLAUDE.md
├── README.md
├── docs/
│   ├── ROADMAP.md
│   ├── ARCHITECTURE.md
│   ├── PHASE1_PLAN.md
│   └── USAGE.md                    # Se escribe en Fase 5
├── fixtures/                       # Logs reales + sintéticos
│   ├── real/
│   │   ├── quickfix_cpp_sample.log
│   │   ├── quickfix_java_sample.log
│   │   └── ...
│   └── synthetic/
│       ├── minimal_4.4.log
│       ├── malformed.log
│       └── ...
├── benches/                        # Benchmarks globales
│   └── baselines.json              # Baseline de performance versionado
├── crates/
│   ├── fixlog-format/
│   ├── fixlog-parser/
│   ├── fixlog-dict/
│   ├── fixlog-index/
│   ├── fixlog-query/
│   ├── fixlog-core/
│   ├── fixlog-cli/
│   └── fixlog-tui/
├── dictionaries/                   # XMLs fuente de QuickFIX (gitignored o sparse)
│   ├── FIX44.xml
│   ├── FIX50.xml
│   ├── FIX50SP1.xml
│   ├── FIX50SP2.xml
│   └── FIXT11.xml
└── .claude/
    └── commands/
```

## Dependencias entre crates

```
                    ┌─────────────┐
                    │ fixlog-tui  │  (Fase 3+)
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │ fixlog-cli  │  (Fase 1+)
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │fixlog-core  │  (re-export facade)
                    └──────┬──────┘
             ┌─────────────┼──────────────┬──────────────┐
       ┌─────▼────┐  ┌─────▼─────┐  ┌────▼─────┐  ┌─────▼────┐
       │  format  │  │  parser   │  │   dict   │  │  index   │
       └──────────┘  └───────────┘  └──────────┘  └─────┬────┘
                                                         │
                                                   ┌─────▼────┐
                                                   │  query   │
                                                   └──────────┘
```

Reglas:

- `fixlog-parser` **no depende** de `fixlog-dict`. El parser es agnóstico.
- `fixlog-dict` depende de `fixlog-parser` (consume `RawMessage`).
- `fixlog-index` depende de `fixlog-parser` (indexa sin resolver).
- `fixlog-query` depende de `fixlog-parser` (evalúa contra `RawMessage`).
- `fixlog-core` re-exporta todo para consumidores (CLI/TUI).

## Tipos núcleo

### `fixlog-format::LogFormat`

Producido por el sniffer. Consumido por el parser.

```rust
pub struct LogFormat {
    pub separator: Separator,
    pub line_prefix: Option<LinePrefix>,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
    pub message_boundary: MessageBoundary,
}

pub enum Separator {
    Soh,          // \x01
    Pipe,         // |
    Caret,        // ^A
    Semicolon,    // ;
    Custom(u8),
}

pub enum LinePrefix {
    None,
    Regex(regex::Regex),  // e.g. r"^\d{8}-\d{2}:\d{2}:\d{2}\.\d{3} : "
}

pub enum MessageBoundary {
    Line,       // un mensaje por línea
    Checksum,   // mensaje termina cuando ve tag 10=xxx\x01
}
```

### `fixlog-parser::RawMessage`

Producido por el parser. Referencia bytes del buffer original.

```rust
pub struct RawMessage<'a> {
    pub offset: u64,                    // Offset en el archivo original
    pub raw: &'a [u8],                  // Bytes crudos del mensaje completo
    pub tags: SmallVec<[(u32, &'a [u8]); 32]>,  // Stack-allocated hasta 32 tags
}
```

Decisiones:

- `SmallVec<[_; 32]>` evita allocation heap para mensajes pequeños (la mayoría tienen <32 tags).
- `offset` permite que el índice referencie mensajes sin guardar el contenido.
- `raw` permite re-parsear o exportar el mensaje original sin reconstruirlo.

### `fixlog-dict::ResolvedMessage`

Producido por el resolver. Útil para presentación, no para hot paths.

```rust
pub struct ResolvedMessage<'a> {
    pub raw: &'a RawMessage<'a>,
    pub version: FixVersion,
    pub msg_type: MsgType,              // e.g. NewOrderSingle, ExecutionReport
    pub fields: Vec<ResolvedField<'a>>,
}

pub struct ResolvedField<'a> {
    pub tag: u32,
    pub name: &'static str,             // e.g. "ClOrdID"
    pub field_type: FieldType,
    pub raw_value: &'a [u8],
    pub decoded: Option<DecodedValue>,  // Para enums: "1" -> "Buy"
}
```

### `fixlog-index::LogIndex`

```rust
pub struct LogIndex {
    pub format: LogFormat,
    pub messages: Vec<MessageOffset>,
    pub secondary: HashMap<IndexKey, RoaringBitmap>,  // Bitmaps comprimidos
    pub file_size: u64,                 // Para append-only tracking
}

pub struct MessageOffset {
    pub start: u64,
    pub len: u32,
}

pub enum IndexKey {
    ByTagValue(u32, SmallVec<[u8; 16]>),  // (35, "D") => mensajes con 35=D
}
```

## Data flow

### Fase 1: parseo simple

```
File bytes (mmap)
    │
    ▼
[fixlog-format] → LogFormat
    │
    ▼
[fixlog-parser] → Iterator<RawMessage<'a>>
    │
    ▼ (opcional, solo para presentación)
[fixlog-dict]  → ResolvedMessage<'a>
    │
    ▼
stdout (pretty o JSON)
```

### Fase 2: indexación + filtro

```
File bytes (mmap)
    │
    ▼
[fixlog-format] + [fixlog-parser] (parallel con rayon)
    │
    ▼
[fixlog-index] → LogIndex
    │
    │ ← filter expression "35=D AND 55=AAPL"
    │   [fixlog-query] parsea a AST
    ▼
[evaluator] → Vec<MessageIndex>
    │
    ▼
Re-parseo lazy de esos mensajes → output
```

### Fase 3: TUI

```
LogIndex + filter → visible_window (Vec<MessageIndex>)
                        │
                        ▼
                [ratatui render]
                        │
                        ▼
                Eventos de teclado
                        │
                        ├── navegación: update cursor
                        ├── filtro: recalcular visible_window
                        └── nuevo mensaje (tail): append a índice, re-render
```

## Decisiones de performance

- **memmap2 desde Fase 1**: aunque para archivos pequeños es overkill, instaurar el patrón temprano evita refactors.
- **SmallVec para tags**: reduce allocs, crítico al parsear millones de mensajes.
- **RoaringBitmap para índice secundario**: mucho más compacto que `Vec<usize>` cuando hay millones de mensajes.
- **Parseo lazy**: el índice guarda offsets, no `RawMessage`. Solo re-parseas los que vas a mostrar.
- **Rayon con chunks alineados a saltos de línea**: splitting naive rompe mensajes a la mitad. El worker busca primero el siguiente inicio de mensaje válido.

## Contratos y errores

Cada crate define sus propios errores con `thiserror`:

```rust
// fixlog-parser
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("invalid BeginString at offset {offset}")]
    InvalidBeginString { offset: u64 },
    #[error("checksum mismatch: expected {expected}, got {got}")]
    BadChecksum { expected: u8, got: u8 },
    // ...
}
```

El parser **no propaga errores fatales** por mensajes individuales corruptos. Los loggea con `tracing::warn!` y continúa. Solo retorna `Err` por errores de I/O del buffer.

## Testing strategy

- **Unit tests**: cada crate tiene `#[cfg(test)]` en cada módulo.
- **Integration tests**: `crates/<crate>/tests/` con fixtures.
- **Golden tests**: archivos `fixtures/<name>.log` + `fixtures/<name>.expected.json`. El test parsea y compara.
- **Property tests** (opcional, Fase 2+): `proptest` para generar mensajes FIX aleatorios y validar roundtrip.
- **Fuzzing** (opcional): `cargo-fuzz` sobre el parser para encontrar crashes.

## Convenciones de nombrado

- Crates: `fixlog-<role>`.
- Módulos dentro de crates: snake_case, roles específicos (`sniffer`, `tokenizer`, `resolver`).
- Tipos públicos: sustantivos (`RawMessage`, `LogFormat`, `ParseError`).
- Funciones públicas: verbos (`parse_message`, `sniff_format`, `resolve`).
- Constantes FIX: usar mayúsculas con prefijo de tag, e.g. `TAG_BEGIN_STRING: u32 = 8;`.
