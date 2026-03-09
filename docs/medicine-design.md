# Medicine System Design

## Concept

Medicines are the player's primary tool to fight disease. You select a medicine, deploy it to a region, and choose who receives it: susceptible people (vaccination — prevents infection) or infected people (treatment — cures them).

Medicines start locked and must be developed through the research pipeline: Identify Threat → Develop Medicine → (optional) Clinical Trial → Deploy.

## Data Model

### Medicine

```rust
struct Medicine {
    name: String,
    therapy_type: TherapyType,          // Antiviral, Antibiotic, or BroadSpectrum
    target_diseases: Vec<usize>,        // which diseases this medicine works against
    cost: f64,                          // funding per deployment
    doses: f64,                         // remaining doses (depletes on deployment)
    max_doses: f64,                     // maximum dose capacity (restored by manufacturing)
    unlocked: bool,                     // false until developed via research
    tested_against: Vec<usize>,         // diseases with completed clinical trials
    strain_generations: Vec<u32>,       // strain calibration per target disease
}
```

### TherapyType × PathogenType Efficacy

Effective doses = `doses × therapy_efficacy × strain_efficacy`. Doses deplete with each deployment — the number of people actually treated is subtracted from the medicine's dose supply. When doses reach 0, the medicine cannot be deployed until more are manufactured via a bench research project (ManufactureDoses: 3 personnel, 120 ticks).

| TherapyType / PathogenType | RnaVirus | DnaVirus | Bacterium | Prion |
|---|---|---|---|---|
| Antiviral | 1.0 | 0.8 | 0.1 | 0.0 |
| Antibiotic | 0.1 | 0.1 | 1.0 | 0.0 |
| BroadSpectrum | 0.5 | 0.5 | 0.5 | 0.1 |

### Strain Drift

When a disease mutates (increments `strain_generation`), medicines calibrated to older generations lose efficacy: `-25% per generation behind` (floor 10%). Re-running a Clinical Trial re-calibrates the medicine to the current strain.

### Untested Medicine Risk

Deploying a medicine that hasn't been clinically trialed against the target disease triggers a confirmation dialog. If the player proceeds, there's a 25% chance of adverse effects: 20% of the deployed doses kill instead of helping.

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
3. Compute `effective_doses = doses × therapy_efficacy × strain_efficacy`
4. Compute `actual_doses = min(effective_doses, available_targets)`
5. If `actual_doses == 0`, show message and stay on SelectTarget
6. If untested: require confirmation via ConfirmDeploy step
7. Deduct `medicine.cost` (flat rate regardless of actual doses — creates strategic incentive to deploy when there are enough targets to justify the cost)
7b. Deplete `medicine.doses` by `actual_doses` (doses are a finite resource)
8. If untested: roll for adverse effects (25% chance, 20% of doses cause deaths)
9. Apply: vaccinate adds to `immune` from susceptible pool; treat moves `infected` to `immune`
10. Show deployment feedback message (doses used, region, cost, efficacy note, adverse effects if any)
11. Return to SelectRegion for quick follow-up deployments

## Starting Medicines

| Medicine | TherapyType | Targets | Cost | Doses |
|---|---|---|---|---|
| Antiviral-A | Antiviral | Disease 0 | $200 | 100K |
| Antibiotic-B | Antibiotic | Disease 1 | $150 | 100K |
| Broad-Spectrum | BroadSpectrum | Both | $400 | 200K |

All start locked. Research costs scale by target count:
- **Narrow (1 target):** 3 personnel, 200 ticks
- **Broad (2+ targets):** 10 personnel, 400 ticks
- **Clinical Trial:** 5 personnel, 160 ticks (same for all)

The trade-off: narrow medicines are faster and cheaper to develop (crisis response), while broad-spectrum takes longer but covers everything.

## Selection Bounds

```
BrowseMedicines  → unlocked_medicines.len() - 1
SelectRegion     → regions.len() - 1
SelectTarget     → 2 * target_diseases.len() - 1
ConfirmDeploy    → 0 (single confirmation)
```

BrowseMedicines filters to unlocked medicines only. The selection index maps into this filtered list.
