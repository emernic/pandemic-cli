# Target Architecture

Current state and ongoing discipline for the codebase structure.

## Layering

The codebase has four layers. Dependencies flow downward only.

```
main.rs / snapshot.rs     I/O boundary (terminal, files, CLI)
        ↓
lib.rs                    Coordination (apply_action routes input)
        ↓
ui/  |  engine/           Rendering + UI state  |  Game logic (mutations)
        ↓                         ↓
state.rs                  Domain model: data structures, derived computations,
                          and UI state machines (UiState methods)
```

**UI and engine are peers — neither imports from the other.** Both depend on state.rs. The coordination layer in lib.rs connects them: UI state machines translate user intent into `GameCommand`s, and `apply_action()` passes those to `engine::execute_command()`.

### What state.rs actually is

state.rs is a **domain model layer** — not just passive data. It contains three things:

1. **Data structures** — the raw stored game state (`GameState`, `Region`, `Disease`, `Medicine`, `RegionPolicy`, etc.)
2. **Derived computations** — read-only methods that compute values both the engine and UI need: `approval_target()`, `funding_income_rate()`, `policy_unlocked()`, `decree_unlocked()`, `tech_pressure()`, `available_projects()`, `has_zero_agency()`, and many more
3. **UI state machines** — `UiState` methods for panel navigation, wizard steps, and confirm handlers

The derived computations *must* live in state.rs because both the engine (to drive game logic) and the UI (to display information to the player) read them. If they lived in engine/, the UI couldn't access them. They are all pure reads — no mutations.

**The mutation boundary:** Only engine/ mutates game data (via `tick()` and `execute_command()`). Nearly everything in state.rs is a read-only computation or accessor. The handful of mutating functions are:
- `UiState` mutation methods — UI state machines, not game state
- Struct mutation helpers (`get_or_create_infection`, `add_resistance`, `set_bool`) — thin helpers called by engine, living on their data types
- `migrate()` — called once after deserialization to fix save-file compatibility

### How input flows

```
keypress → action.rs: key_to_action() → Action
         → lib.rs: apply_action()
             UI actions (navigate, select) → UiState methods (state.rs)
             ToggleExtra → lib.rs resolves UI context → GameCommand → execute_command()
             Confirm → lib.rs: handle_confirm() → Option<GameCommand>
               → engine::execute_command() → CommandResult { message, success }
               → lib.rs maps result to UI navigation (inline in apply_action)
```

**All persistent game state mutations go through `execute_command()`.** This includes standing orders, auto-deploy, and auto-research preferences. The only exceptions are:
- `UiState` mutations (panel navigation, selection indices, wizard steps) — handled directly in `apply_action()` and UiState methods
- `sim_state` pause/resume — simulation control handled directly in `apply_action()`
- `auto_resolve_crises` preference — saved alongside `ResolveCrisis` command in the crisis-handling path of `apply_action()`

`execute_command()` never touches `UiState`. It returns a result and the caller handles UI updates.

**Confirm handling lives in lib.rs, not state.rs.** `handle_confirm()` and the four panel-specific wizard handlers (`handle_medicine_confirm`, `handle_research_confirm`, `handle_policy_confirm`, `handle_operations_confirm`) are free functions in lib.rs. They take `&mut UiState` and `&GameState`, advance wizard state machines, do UX pre-validation (e.g. funding checks), and synthesize `GameCommand`s. This is coordination logic — it belongs in the coordination layer. The boundary is:

**Validation contract:** The engine is always the authoritative validation layer — game commands validate their preconditions (funding, doses, personnel) and return an error message on failure. UI wizard handlers may add *best-effort preview checks* to fail early and prevent players from advancing multi-step wizards to dead ends. When a UI pre-check exists, it MUST use the same cost calculation as the engine to prevent drift. The canonical example is medicine deployment: `medicine_deploy_cost(medicine_idx, region_idx)` on `GameState` computes the full actual cost (base × disruption × discount) and is called by both the UI pre-check in `handle_medicine_confirm()` and the engine in `deploy_medicine()`. Never recompute these formulas inline — add a method to `GameState` and call it from both places.

- **state.rs `impl UiState`:** Navigation and panel state mutations (`toggle_panel`, `close_panel`, `select_*`, `panel_selection_max`, `sync_panel_region`). These only need `&mut self` plus scalar counts — they don't make decisions requiring full `GameState` context.
- **lib.rs wizard handlers:** What happens when you press Enter. These read game state broadly (medicines, diseases, research slots) to decide how wizard steps advance and which command to synthesize.

**UI pre-check contract:** Wizard handlers may add *best-effort preview checks* to fail early, before a multi-step wizard reaches a dead end. These must use the same cost calculation as the engine — never recompute formulas inline. Add a method to `GameState` and call it from both places. `GameState::medicine_deploy_cost(medicine_idx, region_idx)` is the canonical example: it encapsulates base × disruption × discount, and is called by both `handle_medicine_confirm()` (preview) and `deploy_medicine()` (authoritative check). Most game commands validate preconditions in the engine command handler; crisis resolution is an exception where callers check affordability before calling `resolve_crisis()`.

### How simulation flows

The canonical way to advance the game is `lib::tick_and_process(state)` — it calls `engine::tick()` then `ui::process_events()` as a single logical operation and is the only public API for advancing the simulation. Both `engine::tick()` and `ui::process_events()` are `pub(crate)`, so external callers cannot bypass `tick_and_process`. Engine unit tests may call `engine::tick()` directly to test game logic in isolation without UI state updates.

Each tick, `engine::tick()` orchestrates subsystems in order:

1. Disease spread (within-region, cross-region) + mutation
2. `research::tick_research()` — advance/complete projects, scientist burnout/recovery
3. `medicine::try_auto_deploy()` + `tick_shipments()`
4. `infrastructure::tick_infrastructure()` — hospital/intel degradation
5. `crisis::tick_crisis_operations()` — temporary personnel commitments
6. `policy::tick_enforce_costs()` — suspend unaffordable policies, deduct costs
7. Loans — queue offers after policy suspensions, accrue interest
8. Policy: governor cooperation/actions, standing orders, screening
9. `contracts::tick_check/offer_contracts()` — funding contract conditions + new offers
10. `corporations::tick_corporations()` + `board::update_board_satisfaction()`
11. Resource income (funding), personnel upkeep, attrition, authority drift
12. Disease emergence (mid-game new threats), detection, threat escalation
13. Pending crises, board meetings, `crisis::generate_crisis()`
14. *RNG write-back*
15. Regional collapse (may trigger refugee crisis)
16. Defeat conditions + history recording

After each tick, `lib::tick_and_process()` calls `ui::process_events()` to translate `GameEvent`s into UI responses (status messages, panel resets). Game-rule state transitions (pausing on game-over, entering event mode for crises) happen in `tick()` itself — the UI layer only handles presentation responses.

## Engine Module Structure

The engine is organized as an orchestrator + subsystem modules:

```
engine/
  mod.rs            — tick(), execute_command(), initialize_game(): orchestration + cross-cutting logic
  research.rs       — Research project commands + per-tick completion logic + scientist burnout/recovery
  medicine.rs       — Medicine deployment, shipment delivery, auto-deploy
  policy.rs         — Policy toggle, decrees, governor actions, infrastructure builds
  crisis.rs         — Crisis event generation + resolution, board budget calculation
  spread.rs         — Within-region spread, cross-region spread, mutation, adaptation
  disease.rs        — Disease emergence (spawning new scaled diseases mid-game)
  board.rs          — Board member generation and satisfaction
  corporations.rs   — Corporation generation, manufacturer assignment, stock price ticks
  contracts.rs      — Funding contract offers, condition checking, acceptance/rejection
  loans.rs          — Emergency loans, interest accrual
  infrastructure.rs — Hospital/intel infrastructure degradation
```

### Subsystem conventions

Each subsystem module follows the same pattern:

- **Visibility:** `pub(super)` — only `mod.rs` calls into subsystems
- **Dependencies:** Only `crate::state`. Never other subsystem modules, never UI
- **Two function types:**
  - **Tick helpers** (called from `tick()`) — advance ongoing processes. Named `tick_*()`. Examples: `tick_research()`, `tick_enforce_costs()`, `tick_spread_within()`
  - **Command handlers** (called from `execute_command()`) — handle player actions. Examples: `start_research()`, `deploy_medicine()`, `toggle_policy()`
- **No cross-subsystem calls.** If research completion needs to modify medicines (e.g., unlocking one), it does so through `GameState` directly — not by calling into the medicine module. Subsystems share data through state, not through each other.
- **Tests live with the code they test.** Each subsystem module has its own `#[cfg(test)] mod tests`. Integration tests that exercise multiple subsystems through `tick()` or `apply_action()` stay in `engine/mod.rs`.

### Adding a new game system

1. Create `engine/newsystem.rs` with `pub(super)` functions
2. Add `mod newsystem;` in `engine/mod.rs`
3. If it has per-tick behavior: add a `tick_*()` function, call it from `tick()`
4. If the player interacts with it: add a `GameCommand` variant in `state.rs`, add a handler function, dispatch in `execute_command()`
5. If tick events need UI feedback: add a `GameEvent` variant, handle in `ui::process_events()`

### What stays in mod.rs

`tick()`, `execute_command()`, and `initialize_game()` are the public entry points — they stay in mod.rs. `tick()` also contains cross-cutting logic that spans multiple subsystems: resource income/upkeep, board approval drift, personnel attrition, disease detection, threat escalation alerts, threat level computation, disease emergence orchestration, regional collapse, defeat conditions, and history recording. This is ~300 lines of domain logic, not pure orchestration — but it's logic that touches multiple subsystems simultaneously and doesn't have a natural single-module home. If any chunk grows large enough to warrant extraction, it follows the same subsystem pattern.

## Event System

`tick()` produces `GameEvent` variants (stored in `state.events`, cleared each tick). These are ephemeral signals — `#[serde(skip)]`, not persisted.

**Game-rule transitions live in the engine:** When the game ends, `tick()` sets `outcome` and `sim_state = Paused`. When a crisis appears, `crisis::activate_crisis()` sets `active_crisis` and `sim_state = Event { was_running }`. Exception: collapse-triggered refugee crises are set inline in `tick()` directly (bypassing `activate_crisis()`). Disease detection no longer pauses the simulation; it fires a `DiseaseDetected` event shown in the top-right notification area. When a crisis is resolved, `crisis::resolve_crisis()` restores `sim_state` from `Event { was_running }` back to Running or Paused. The engine owns the full lifecycle — entry and exit. The UI layer does not touch `sim_state`.

**UI responses live in `ui::process_events()`:** Called by `lib::tick_and_process()` after each tick, `process_events()` handles UI-specific reactions (close panels on game-over, reset crisis selection) and formats events into status messages. It does not mutate `sim_state`, `outcome`, or other game-rule state. It also performs noise-reduction filtering — for example, `DiseaseMutated` events are suppressed when no player medicine is affected by the mutation, since there is nothing actionable to show.

When adding new event types: game-rule transitions go in `tick()`. Presentation responses go in `process_events()`.

## Two Feedback Pipelines

There are two distinct pipelines for player-visible messages, each serving a different purpose:

1. **Tick-time events** → `GameEvent` enum → `ui::process_events()` → event log + status bar. These are asynchronous notifications (disease detected, shipment delivered, region collapsed). They need priority ordering, log persistence, and may trigger UI state changes (panel resets). Some events produce an enriched notification for the top-right status area that includes contextual action hints (e.g., "Use [R] Research"); the event log always receives the plain message without hints.

2. **Command responses** → `CommandResult.message` → `status_message`. These are synchronous feedback to a player action (deployed medicine, started research, toggled policy). Formatted directly in engine command handlers. Shown once in the status bar, not logged.

This is intentional. Command handlers have the context needed to compose feedback (amounts, names, reasons) and the messages are simple enough that structured result types would add boilerplate without functional benefit. The convention is: new tick events → `GameEvent` variant + `process_events()` handler. New commands → return `(success, Option<String>)` from the handler.

## Panel Selection Convention

`UiState.panel_selection` is a **shared generic cursor** — a single `usize` that means different things depending on the active panel and wizard substate. This is a deliberate design choice (not a hack), but it creates coupling that must be actively managed.

**How it works:** Each panel+substate combination treats `panel_selection` as an index into its own list. The system is safe because:
1. `panel_selection` is reset to `0` on every wizard step transition
2. `panel_selection_max()` bounds it correctly for each panel+substate combination
3. Renderers and confirm handlers both read `panel_selection` in their own context and MUST agree on the index mapping

**Named constants that tie the pieces together:**
- `RESEARCH_TRACK_COUNT` — Field(0), Applied(1), Basic(2); UpgradeLab is always at index `RESEARCH_TRACK_COUNT`. Used by `panel_selection_max()`, `handle_research_confirm()`, and `render_categories()`.
- `STANDING_ORDER_COUNT` — number of standing orders in Policy/BrowseRegions. Used by `panel_selection_max()` and asserted at render time in `ui/policy.rs`.
- `FIELD_OP_TYPE_COUNT` — number of deployable op types in Operations/BrowseOps. Used by `panel_selection_max()` and the ops array in `ui/operations.rs`.
- `MANAGE_*` constants — positions within Policy/ManagePolicies (policy toggles, infra repair, priority, appease, bargain). Shared between renderer and handler.

**The invariant:** when adding a new item to any panel list, you must update both the renderer AND `panel_selection_max()`. The named constants enforce this for the fragile cases — if `STANDING_ORDER_COUNT` drifts from the actual array length, a `debug_assert` fires at render time.

**Why not typed per-panel selections?** Replacing `panel_selection: usize` with typed enum variants per panel+substate would make the coupling compiler-enforced, but would require every renderer and every state transition to carry a different selection type. The boilerplate cost outweighs the safety benefit at current scale, since the existing constants + debug_asserts + conventions are sufficient. Revisit if the panel count grows significantly or selection drift causes actual bugs.

## What The Architecture Enforces (And What It Doesn't)

The architecture is "one giant mutable state blob plus conventions." This section is honest about which boundaries are real and which are social.

**Enforced by the compiler:**
- `pub(super)` on subsystem functions — external code can't call `research::start_research()` directly, only through `execute_command()`
- Module visibility — `engine/` doesn't `use crate::ui`, `ui/` doesn't `use crate::engine`. A new import would be a visible `use` statement in the diff.
- `GameCommand` enum — every variant is dispatched through `execute_command()`. `apply_action()` calls `handle_confirm(&mut ui, &state)` (a free function in lib.rs) to get a `GameCommand` and passes it to `execute_command()` unconditionally. There is no intercept or bypass. Preference toggles (`auto_deploy`, `auto_research`, `standing_orders`) go through `GameCommand` variants too.

**What `apply_action()` mutates directly (without `GameCommand`):**
- `sim_state` (pause/unpause) and `ui.speed_multiplier` — pure UI controls, no game logic involved
- `auto_resolve_crises` — crisis preference toggled inline in the crisis confirm path (reads crisis tag, manages a HashMap). This is the one remaining bypass; it could be moved to a `GameCommand` if it grows more complex.
- All other persistent game state mutations go through `execute_command()`.

**Enforced by convention only (can be violated without compiler errors):**
- Engine code should not read or write `state.ui.*` fields. Nothing prevents it — `UiState` is a public field of `GameState`, which engine functions receive as `&mut GameState`.
- UI code should not mutate game state (beyond `UiState`). Again, nothing prevents it — UI functions also receive `&mut GameState`.
- Subsystems should not call each other. They share `&mut GameState`, so any subsystem could call any other subsystem's logic through state methods.
- Command response strings are composed in the engine. Tick-time event strings are composed in the UI. No type prevents mixing these.

**Why this is acceptable at current scale:** This is a single-binary game worked on by AI agents that read CLAUDE.md and architecture docs. The conventions are documented, the boundaries are visible in code review, and violations are caught by the agents' instruction-following. Type-level enforcement (splitting the state blob, wrapper types restricting access) would add significant complexity for a problem that isn't causing bugs. If the codebase grows to the point where convention violations become a recurring issue, revisit this decision.

## What NOT to Change

- **Single `GameState` struct** — One serializable blob = trivial save/load.
- **`UiState` inside `GameState`** — The UI state is part of the save. The issue isn't where it lives in the struct, it's which code touches it.
- **Clone-and-mutate in `tick()`** — Fine at current scale. Don't optimize prematurely.
- **Snapshot mode** — Keep this exactly as-is. It's one of the best things about the codebase.

## Completed Migrations

These are done. Listed for historical context only.

1. **UI state machines extracted from engine** — `apply_action()` moved to `lib.rs`. Panel navigation and selection indices live in `UiState` methods. Wizard confirm handlers (what happens when you press Enter) are free functions in `lib.rs`. Engine only exports `tick()` and `execute_command()`.
2. **Query functions moved to state.rs** — `project_costs()` → `ResearchKind::costs()`. `available_field_projects()`, `available_applied_projects()` → `GameState` methods. UI no longer imports from engine.
3. **`CommandResult` type** — `execute_command()` returns `CommandResult { message, success }` instead of directly modifying UI state.
4. **Engine god file broken up** — Research, medicine, policy, crisis, spread, disease emergence, and personnel logic extracted into 7 subsystem modules.
