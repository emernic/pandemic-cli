---
name: write-tests
description: Write tests for new or changed code. **ALWAYS** load this when adding or changing tests for a feature and read it very carefully.
---

# Writing Tests

Many AI models have some really counter-productive tendencies when it comes to writing tests... (bikeshedding, excessive redundancy, lazily failing to use existing fixtures or follow existing conventions, neglecting meaningful tests because they sound "hard" etc.). A lot of this comes down to treating test writing as an unserious "filler" task or trying to write a lot of tests that "look nice" instead of doing the hard work of actually thinking about the code, identifying where the complexity lives, and thinking about data configurations that meaningfully exercise those paths in a clear, realistic way.

## Think first

Before writing any test code, read the code you're testing and trace through it. Ask: **where am I genuinely unsure this will be correct?** Where would I have to actually think to predict the output? Those are the tests worth writing.

The first cases that come to mind are usually the trivially obvious ones — "create a thing, check it exists" or "set a name, assert the name." Skip those. Find the minimal set of cases that cover the parts where something could actually go wrong, and write those.

For each test you're about to write, ask: "Could this plausibly fail?" If you already know it'll pass without running it, don't write it.

## What makes a good test

**Each test should cover a different code path.** If two tests go through the same logic with slightly different inputs, that's one test (or a parameterized case), not two. Write separate tests when the logic actually differs.

**Assert computed outputs.** Checking `state.paused == true` after you construct a state with `paused: true` is not a test. Checking that `tick()` correctly reduces susceptible population when immunity is high, that medicine deployment deducts the right cost and updates the right region, that cross-region spread eventually reaches uninfected regions — those are tests.

**Keep it readable.** A test should be obvious about what it's checking and why. Minimal setup, clear action, focused assertions. Don't bury the important assertion in a wall of property checks. If you're asserting 10 fields and only 2 of them involve real logic, just assert the 2.

**No padding.** A file with 3 tests that each catch a different real bug beats 15 tests that all pass trivially. Don't test simple field access, trivial constructors, or anything where the compiler does the work.

**Use real state, not abstractions.** `engine::new_game(seed)` and the real `tick()`/`apply_action()` functions, not mock objects. The whole engine is pure and deterministic — there's no reason to mock anything.

## Use existing patterns

Before writing new test infrastructure, check what already exists:

- **`engine::new_game(seed)`** — fully bootstrapped game state matching production startup (corporations, board, budget). Use seed `42` by convention. From engine submodule tests: `crate::engine::new_game(42)`. From integration tests: `pandemic_cli_lib::engine::new_game(42)`.
- **`AppState::new_default(seed)`** — raw pre-bootstrap state (no corporations, board, or budget). Only for tests that intentionally need uninitialized state.
- **`tick(&state) -> AppState`** and **`apply_action(&state, &Action) -> AppState`** — the two pure functions that drive everything. Clone-and-mutate, so you can chain them.
- **`render_to_string(&state) -> String`** and **`run_snapshot(state, steps) -> SnapshotResult`** — for rendering and integration smoke tests. Use structural assertions (`assert!(output.contains("..."))`), NOT exact-match snapshot libraries.
- **Inline `#[cfg(test)] mod tests`** in each source file for unit tests.
- **`tests/snapshots.rs`** for integration snapshot tests.

**Only build custom state when you need a specific configuration no default provides** — a region with specific immunity levels, a disease with particular stats, resources set to a specific amount. If you just need "a game state," use `engine::new_game(42)`. Do not manually call `generate_corporations()` + `generate_board_members()` — use `new_game()` instead.

**Don't define helper functions at the top of your test module** unless they're genuinely reused across multiple tests in that module. More than 2-3 bespoke helpers means you're over-specifying.

## Structure

- Inline `#[cfg(test)] mod tests { use super::*; ... }` at the bottom of each source file.
- Integration tests in `tests/` directory.
- Snapshot tests use `insta::assert_snapshot!` for visual regression.
- Add assertions to existing test functions when appropriate rather than creating new ones.
- Don't create separate test files for sub-features when they fit in the existing module.

## Examples

### Example 1: Testing non-obvious simulation behavior

Two tests that verify different emergent behaviors of the disease spread engine:

```rust
#[test]
fn immune_reduces_susceptible_pool() {
    let mut state = AppState::new_default(42);
    // Set immunity to near-total in Asia to shrink the susceptible pool
    state.regions[4].get_or_create_infection(0).immune = 4_000_000_000.0;
    let before = state.regions[4].disease_state(0).unwrap().infected;
    let after = tick(&state);
    let growth = after.regions[4].disease_state(0).unwrap().infected - before;

    // Compare against a baseline with no immunity
    let state2 = AppState::new_default(42);
    let after2 = tick(&state2);
    let growth2 = after2.regions[4].disease_state(0).unwrap().infected
        - state2.regions[4].disease_state(0).unwrap().infected;

    assert!(
        growth < growth2,
        "immunity should reduce infection growth: {} vs {}",
        growth, growth2
    );
}

#[test]
fn disease_can_spread_into_vaccinated_region() {
    let mut state = AppState::new_default(42);
    // Pre-vaccinate North America — disease should still arrive
    state.regions[0].infections.push(RegionDiseaseState {
        disease_idx: 0,
        infected: 0.0,
        dead: 0.0,
        immune: 100_000_000.0,
    });
    let mut s = state;
    for _ in 0..200 {
        s = tick(&s);
    }
    let na_imm = s.regions[0]
        .infections
        .iter()
        .find(|i| i.disease_idx == 0)
        .map(|i| i.immune)
        .unwrap_or(0.0);
    assert!(na_imm >= 100_000_000.0, "immune count should be preserved");
}
```

Why this works: the first test constructs a specific scenario (near-total immunity) and asserts a **computed comparison** — not just "did something change" but "did the growth rate decrease relative to baseline." The second test verifies a subtle design decision: vaccination entries with `infected: 0.0` should not block disease arrival via cross-region spread. Both would catch real bugs in the spread math.

### Example 2: Testing a multi-step UI flow with side effects

From the medicine deployment tests — one test covers the full vaccination flow including state transitions and resource deduction:

```rust
#[test]
fn medicine_vaccination_deployment() {
    let mut state = AppState::new_default(42);
    state = apply_action(&state, &Action::OpenMedicines);
    assert_eq!(state.ui.open_panel, Panel::Medicines);
    state = apply_action(&state, &Action::Confirm);
    assert!(matches!(
        state.ui.medicine_ui,
        Some(MedicineUiState::SelectRegion { medicine_idx: 0 })
    ));
    state = apply_action(&state, &Action::Confirm);
    assert!(matches!(
        state.ui.medicine_ui,
        Some(MedicineUiState::SelectTarget { .. })
    ));
    let funding_before = state.resources.funding;
    state = apply_action(&state, &Action::Confirm);
    // Computed outputs: cost deducted, immunity applied, UI returns to SelectRegion
    assert_eq!(state.resources.funding, funding_before - 100.0);
    let na_inf = state.regions[0]
        .infections.iter()
        .find(|i| i.disease_idx == 0)
        .unwrap();
    assert_eq!(na_inf.immune, 10_000.0);
    assert!(matches!(
        state.ui.medicine_ui,
        Some(MedicineUiState::SelectRegion { medicine_idx: 0 })
    ));
}
```

Why this works: one test, one situation (successful vaccination deployment), but it verifies the three things that matter: resource cost was deducted correctly, immunity was applied to the right region, and the UI returned to the right state for rapid multi-region deployment. The same file has separate tests for genuinely different paths: treatment (different deploy target math), insufficient funds (rejection), zero targets (edge case), and ESC backstep (UI state machine reversal). Each would catch a different bug.

Note: When writing some of these tests, we actually had to make some improvements to the existing state setup to keep things clean. That's ok and expected. We own this whole codebase. If something is the right call, do it.

## Cover Your Bases

Keeping all of the guidance above in mind will help you write genuinely **useful** tests instead of slop, but don't forget about the basics either:
1. You should have at least some coverage of all the layers. This doesn't always mean one test **per** layer, but it usually does mean at least one (often a couple) snapshot or integration tests.
2. Find the relevant existing tests. Finding the closest examples to the tests you're trying to write is important for a variety of reasons (understanding existing patterns, knowing what cases are already covered or are typically covered for similar features, and just knowing what modules you could add to). Very often, it makes more sense to add one or two assertions to _existing_ tests rather than write entirely new ones.
3. One test = one situation, not one assertion. Don't waste time and space creating 4 different tests that assert different aspects of the same test path/scenario like `test_deploy_deducts_funds`, `test_deploy_updates_immunity`, `test_deploy_returns_to_select_region`. Instead, these can just be 3 simple assertions on a single `medicine_vaccination_deployment` test.
4. Keep it simple. You should think carefully and use the criteria above when deciding what/how to test, but don't go overboard and get lost in a rabbit hole. Just write sane, readable tests that cover the important cases.
5. Lastly, be adversarial and critical in your test crafting. Very often, while thinking through test cases, you will realize (or suspect) that there's a fundamental, basic, but important oversight in your code. You may be tempted to shy away from testing this scenario because you suspect the test may fail. In fact, shying away from testing these cases is the **most destructive** thing you can do. Always **TEST FOR THE BEHAVIOR YOU WANT**, not the behavior you have (that's the essence of TDD).
