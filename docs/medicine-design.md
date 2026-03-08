# Medicine System Design

## Concept

Medicines are the player's primary tool to fight disease. You select a medicine, deploy it to a region, and choose who receives it: susceptible people (vaccination — prevents infection) or infected people (treatment — cures them).

Medicines start locked and must be developed through the research pipeline: Identify Threat → Develop Medicine → (optional) Clinical Trial → Deploy.

## Data Model

### Medicine

```rust
struct Medicine {
    name: String,
    target_diseases: Vec<usize>,  // usually 1, occasionally multiple
    cost: f64,                     // funding per deployment
    doses: f64,                    // people treated per deployment
    unlocked: bool,                // false until developed via research
    tested_against: Vec<usize>,    // diseases with completed clinical trials
}
```

### Untested Medicine Risk

Deploying a medicine that hasn't been clinically trialed against the target disease triggers a confirmation dialog. If the player proceeds, there's a 25% chance of adverse effects: 20% of the deployed doses kill instead of helping. This makes clinical trials strategically important but not mandatory — the player can gamble if the situation is desperate.

### RegionDiseaseState.immune

Tracks vaccinated + cured people per disease per region.

```
susceptible = pop - infected - dead - immune
```

Vaccination before disease arrival creates entries with `infected: 0, immune: X`. Cross-region spread checks `infected > 0` (not entry existence) so disease can still seed into vaccinated regions — it just finds fewer susceptible hosts.

## Player Flow

```
[M] Open medicines panel
 │
 ├─ BrowseMedicines: up/down to browse, Enter to select
 │   └─ SelectRegion: up/down to pick region, Enter to select
 │       └─ SelectTarget: up/down to pick target, Enter to deploy
 │           ├─ If tested: deploy immediately, back to SelectRegion
 │           └─ If untested: ConfirmDeploy warning dialog
 │               ├─ Enter: deploy (with adverse effect risk), back to SelectRegion
 │               └─ Esc: cancel, back to SelectTarget
 │
 └─ Esc goes back one step (or closes panel from BrowseMedicines)
```

After deployment, the player returns to SelectRegion (same medicine selected) for rapid multi-region deployment.

### Target options

For a medicine targeting diseases [0, 1], the list is:
1. Vaccinate susceptible (Disease 0)
2. Vaccinate susceptible (Disease 1)
3. Treat infected (Disease 0)
4. Treat infected (Disease 1)

Vaccinate options first, then treat. With one target disease (the common case), you just see two options.

## Deployment Mechanics

1. Check `funding >= medicine.cost` — show error message if insufficient
2. Find or create `RegionDiseaseState` for the target disease in the region
3. Compute `actual_doses = min(medicine.doses, available_targets)`
4. If `actual_doses == 0`, show message ("No susceptible/infected population") and stay on SelectTarget
5. If untested: require confirmation via ConfirmDeploy step
6. Deduct `medicine.cost` (flat rate regardless of actual doses — creates strategic incentive to deploy when there are enough targets to justify the cost)
7. If untested: roll for adverse effects (25% chance, 20% of doses cause deaths)
8. Apply: vaccinate adds to `immune` from susceptible pool; treat moves `infected` to `immune`
9. Show deployment feedback message (doses used, region, cost, adverse effects if any)
10. Return to SelectRegion for quick follow-up deployments

## Starting Medicines

| Medicine | Targets | Cost | Doses | Notes |
|---|---|---|---|---|
| Antiviral-A | Strain Alpha | $200 | 100K | Cheap, targeted |
| Broad-Spectrum Antiviral | Both | $500 | 500K | Expensive, versatile, high-impact |

Both start locked. The research pipeline to unlock a medicine: Identify Threat (20 ticks) → Develop Medicine (40 ticks) → medicine unlocked. Clinical Trial (25 ticks) makes it safe to deploy without adverse effect risk.

At tick ~100 (earliest medicine availability), infections are ~170K. Antiviral-A's 100K doses can treat ~59% of infected — a meaningful intervention. Broad-Spectrum at 500K doses can treat all infected at that stage.

## Selection Bounds

```
BrowseMedicines  → unlocked_medicines.len() - 1
SelectRegion     → regions.len() - 1
SelectTarget     → 2 * target_diseases.len() - 1
ConfirmDeploy    → 0 (single confirmation)
```

BrowseMedicines filters to unlocked medicines only. The selection index maps into this filtered list.
