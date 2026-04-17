Ejecuta la tarea T$ARGUMENTS de docs/PHASE1_PLAN.md.

Flujo obligatorio:

1. **Lee la tarea específica** en docs/PHASE1_PLAN.md: objetivo, archivos afectados, dependencias, criterio de aceptación.

2. **Verifica dependencias**: si la tarea depende de otras, confirma que están marcadas como `[x]`. Si no, para y avisa.

3. **Revisa el contexto relevante**:
   - docs/ARCHITECTURE.md para contratos entre módulos.
   - CLAUDE.md para convenciones.
   - Código ya existente en los crates involucrados (usa Task/Explore si es mucho).

4. **Propón el approach** en 3-5 bullets concisos antes de codear:
   - Qué archivos vas a crear/modificar.
   - Decisiones de diseño no triviales.
   - Cómo vas a validar (qué tests escribes).
   - Riesgos o dudas.

5. **ESPERA MI CONFIRMACIÓN** antes de escribir código. No asumas OK.

6. **Implementa** una vez aprobado el plan:
   - Sigue las convenciones de CLAUDE.md estrictamente.
   - Escribe tests junto con el código.
   - Zero-copy por defecto.
   - No `unwrap()` / `panic!` en producción.

7. **Valida al terminar**:
   - `cargo build --all`
   - `cargo test --all`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo fmt --all`

8. **Marca la tarea** como `[x]` en docs/PHASE1_PLAN.md.

9. **Propón el commit message** siguiendo conventional commits (feat/fix/refactor/test/docs/chore/perf/bench).

10. **No avances** a la siguiente tarea. Termina aquí para que yo pueda `/clear` y empezar la siguiente en sesión limpia.
