# Medicine System Design

## Concept

Medicines are the player's primary tool to fight disease. You select a medicine, deploy it to a region, and choose who receives it: susceptible people (vaccination — prevents infection) or infected people (treatment — cures them). All medicines are unlocked by default; research gating comes later.

## Data Model

### Medicine

```rust
struct Medicine {
    name: String,
    target_diseases: Vec<usize>,  // usually 1, occasionally multiple
    cost: f64,                     // funding per deployment
    doses: f64,                    // people treated per deployment
    unlocked: bool,                // always true for now
}
```

### RegionDiseaseState — add `immune: f64`

Tracks vaccinated + cured people per disease per region. Changes the susceptible formula:

```
Before: susceptible = pop - infected - dead
After:  susceptible = pop - infected - dead - immune
```

Vaccination before disease arrival creates entries with `infected: 0, immune: X`. The cross-region spread check changes from "entry exists" to "infected > 0" so the disease can still seed into vaccinated regions — it just finds fewer susceptible hosts.

### UiState — add medicine flow state

```rust
enum MedicineUiState {
    BrowseMedicines,
    SelectRegion { medicine_idx: usize },
    SelectTarget { medicine_idx: usize, region_idx: usize },
}
```

The existing `panel_selection` index is reused at each step (reset on transitions). Opening the Medicines panel (via `OpenMedicines`) always resets to `BrowseMedicines` — switching away and back shouldn't leave you mid-flow.

## Player Flow

```
[M] Open medicines panel
 │
 ├─ BrowseMedicines: up/down to browse, Enter to select
 │   └─ SelectRegion: up/down to pick region, Enter to select
 │       └─ SelectTarget: up/down to pick target, Enter to deploy
 │           → funding deducted, doses applied, back to SelectRegion
 │             (keeps medicine selected for rapid multi-region deployment)
 │
 └─ Esc goes back one step (or closes panel from BrowseMedicines)
```

### Target options

For a medicine targeting diseases [0, 1], the list is:
1. Vaccinate susceptible (Disease 0)
2. Vaccinate susceptible (Disease 1)
3. Treat infected (Disease 0)
4. Treat infected (Disease 1)

Vaccinate options first, then treat. With one target disease (the common case), you just see two options.

## Deployment Mechanics

1. Check `funding >= medicine.cost`, refuse if insufficient
2. Find or create `RegionDiseaseState` for the target disease in the region
3. Compute `actual_doses = min(medicine.doses, available_targets)`
4. If `actual_doses == 0`, refuse (nothing to treat)
5. Deduct `medicine.cost` (flat rate regardless of actual doses — creates strategic incentive to deploy at the right time, when there are enough targets to justify the cost)
6. Apply: vaccinate adds to `immune` from susceptible pool; treat moves `infected` to `immune`
7. Return to SelectRegion (same medicine selected) for quick follow-up deployments

## New Action: `Confirm` (Enter key)

In the Medicines panel, Enter advances through the flow steps and executes deployment on the final step. In other panels, it's a no-op for now.

## Modified Action: `ClosePanel` (Esc)

When inside a medicine sub-step, Esc goes back one level instead of closing the panel. Only closes from the top-level BrowseMedicines state.

## SelectNext/SelectPrev bounds

```
BrowseMedicines  → unlocked_medicines.len() - 1
SelectRegion     → regions.len() - 1
SelectTarget     → 2 * target_diseases.len() - 1
```

BrowseMedicines filters to unlocked medicines only. The selection index maps into this filtered list.

## Engine Changes

**tick():** One-line change — susceptible calculation subtracts `immune`.

**Cross-region spread:** Check `infected > 0` instead of entry existence. When seeding into a region that has a vaccination-only entry, set `infected = 1.0` on the existing entry instead of pushing a new one.

**apply_action(Confirm):** State machine for medicine flow + deployment execution.

**apply_action(ClosePanel):** Back-step logic when in medicine sub-states.

## Starting Medicines

Two medicines targeting Strain Alpha, offering a cost/efficiency tradeoff:

| Medicine | Cost | Doses | Notes |
|---|---|---|---|
| Antiviral-A | $100 | 10K | Cheap, small batches |
| Broad-Spectrum Antiviral | $300 | 50K | Better value per dose, bigger commitment |

Starting infection is 50K in Asia. Antiviral-A's 10K doses can't outpace the ~7,500/tick growth rate alone — the player needs sustained spending to suppress. Broad-Spectrum can make a dent at 50K doses but costs $300. The intent is that focused effort can protect a small region but saving everyone requires research/policy tools that come later.

## Future Compatibility

- **Research:** The `unlocked` field gates access. Research system flips it to true.
- **Policy:** Policies can modify `cost` or `doses` fields (e.g., "Pharmaceutical subsidies").
- **Multiple diseases:** `target_diseases` vec handles this naturally.
- **Save compatibility:** All new fields use `#[serde(default)]`.
