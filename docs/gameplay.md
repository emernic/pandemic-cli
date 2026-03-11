# Gameplay

## Concept

You run a global health defense agency in a world that has already seen what biological threats can do. This is not the first pandemic. It's not even the second. The institutions you work through were built in response to earlier crises — crises that reshaped the world order. You are the current director of the N.W.H.O., replacing someone who was removed for inaction.

Real-time with pause. TUI. Deterministic simulation via seeded RNG.

**The game is unwinnable.** There is no victory screen. This is a survival challenge — the goal is to last as long as possible and save as many lives as you can. Without intervention, the game ends within 10-30 days (median ~15, never past 65). A competent player survives around 40 days. Surviving past 100 days should be essentially impossible. The question is never *if* you lose, but *when* and *how many* you save before you do.

## The Arc

The game has an arc that the player discovers by playing, not by being told.

**Early game (days 1-10)** feels like pandemic response. A disease appears. You research it, identify it, develop treatments. You set policies — quarantines, screening, travel bans. You manage governors. You deploy medicines. The systems are familiar. This is what public health looks like.

**Mid game (days 10-30)** introduces pressure from every direction simultaneously. Multiple diseases. Governors defying your directives. Funding sources with agendas. Infrastructure degrading. Crisis events that chain into worse crises based on your earlier choices. The player is no longer just fighting disease — they're fighting the systems they operate through.

**Late game (days 30+)** should feel different from the early game in ways the player can't quite articulate. The threats are more precise. The tools are more powerful but feel less like medicine. The decisions are bigger but the options are narrower. The game never comments on this shift. It just happens.

## Core Systems

### Diseases

Multiple concurrent diseases, procedurally generated. Each has:
- **Pathogen type**: RNA Virus, DNA Virus, Bacterium, Fungus, Prion — each with different mutation rates and treatment requirements
- **Transmission vector**: Airborne, Waterborne, Contact — each countered by different policies
- **Parameters**: infectivity, lethality, recovery rate, cross-region spread rate
- **Mutation**: diseases mutate over time, drifting away from your medicine calibrations. Genomic sequencing slows this. The arms race between your medicines and their mutations is a core tension.

New diseases emerge throughout the game. The emergence rate scales with time.

### Research Pipeline

Three independent tracks run simultaneously:

1. **Field Research** — boots on the ground. Identify unknown threats, run clinical trials, perform genomic sequencing, train personnel. Multiple projects can run in parallel, gated by available personnel.
2. **Applied Research** — lab work. Develop medicines, manufacture doses. One project at a time.
3. **Basic Research** — the tech tree. Unlock new capabilities: Targeted Drug Design, Monoclonal Antibodies, Phage Therapy, Rapid Sequencing, Vaccine Platform, Resistance Surveillance, Combination Therapy, Pathogen Suppression, Directed Attenuation, Genomic Interdiction.

The pipeline for a single disease: **Unknown Threat → Identify (field) → Develop Medicine (applied) → Clinical Trial (field) → Deploy**. Each step takes time and resources. Skipping steps (deploying untested medicine) is possible but risky.

### Medicines

Medicines have a therapy type (Antiviral, Antibiotic, Antifungal, Broad Spectrum) and a mechanism of action (9 types, from fast/cheap to slow/expensive/durable). Efficacy depends on:
- Therapy-pathogen match
- Mechanism effectiveness
- Strain calibration (drift from mutation reduces efficacy — re-trial to recalibrate)
- Drug resistance (builds per mechanism with repeated deployment)
- Whether the medicine has been clinically tested against this disease

Deployment options: **Treat** (reduce infected population) or **Vaccinate** (build immunity in susceptible population). Doses are finite and must be manufactured.

### Policies

Regional policies, toggled per-region. Each costs funding and personnel:
- **Travel Ban** — blocks 90% cross-region spread but halves regional income
- **Quarantine** — halves infection rate within region
- **Hospital Surge** — halves lethality, boosts medicine efficacy
- **Border Controls** — lighter version of travel ban (50% cross-region reduction)
- **Water Sanitation** — halves waterborne disease spread
- **Disease Screening** — tiered (Basic → Antigen → Mass Rapid), affects disease detection visibility
- **Martial Law** — heavy-handed control, reduces collapse threshold, high cost
- **Field Hospital / Medical Center** — permanent infrastructure upgrade (two tiers: Field Hospital reduces lethality, Medical Center adds medicine efficacy bonus)
- **Nuclear Option** — last resort for collapsed regions

Policy costs scale with regional traits and governor cooperation.

### Emergency Decrees

Irreversible global decisions unlocked as the crisis worsens. Each one gives the player new tools, but the tools get heavier.

- **Conscript Researchers** (500K+ infected) — more personnel, permanently reduced income
- **Authorize Human Trials** (50M+ dead or 2+ critical regions) — faster clinical trials, risk of adverse events
- **Suspend Regional Authority** (50M+ dead or 2+ critical regions) — neutralize all governors, no defiance or cooperation
- **Sacrifice Region** (region collapsed or 500M+ dead) — abandon a region to boost income from the rest
- **Fortify Region** (region collapsed or 500M+ dead) — restore one region's infrastructure, penalize all others
- **Emergency Countermeasure** (3+ regions collapsed or 2B+ dead) — reduce all disease infectivity and spread, at the cost of immediate civilian casualties

These are one-way doors. The game doesn't tell you whether they were worth it.

### Governors

Each region has a governor with a name, personality, and loyalty score.

**Personalities**: Buffoon, Blowhard, Recluse, Hardliner, Operative, Mobster. Each has a unique bargain cost, loyalty pattern, and defiance behavior. A Blowhard hates restrictions but is cheap to buy off. A Hardliner is angry about both restrictions and suffering. An Operative demands permanent income cuts. A Mobster's price doubles every time you pay.

**Loyalty** drifts continuously based on regional conditions, your policy choices, and personality. High loyalty means cheaper policies and cooperation. Low loyalty means defiance — governors can lift your quarantines, block your research, or demand military escalation. You can appease them with money, but you can't control them.

### Crises

Random events that pause the game and demand a decision. Two options, each with trade-offs. ~30 distinct types covering supply disruptions, political pressure, staff burnout, media firestorms, corruption, military interference, and more.

Crises chain: your choices create follow-up crises. Tolerate black market drugs and counterfeit medicines appear. Cooperate with the military and they classify your research. Ignore corruption and it becomes an embezzlement ring. The player builds their own disaster through individually reasonable decisions.

### Resources

Three currencies:
- **Funding** — income degrades as global death toll rises. Spent on policies, research, crisis resolution, deployments.
- **Personnel** — finite workforce. Assigned to research projects and policy enforcement. Lost to attrition, crises, burnout.
- **Political Power (POL)** — public mandate. Drifts toward a severity-based target. Gates emergency decrees. Generates personnel slowly.

### Regions

Six regions: North America, Europe, Asia, South America, Africa, Oceania. Connected in a geographic graph. Each has population, traits (Trade-Dependent, Dense Urban, Island Geography, Low Infrastructure, Strong Public Health, Resilient Population), a governor, active policies, and three infrastructure systems.

### Infrastructure

Each region tracks three infrastructure systems (0–100%):

- **Healthcare Capacity** — degrades as infections grow. Below 50%: lethality doubles. Below 25%: lethality quadruples. Hospital Surge policy and field hospitals slow degradation.
- **Supply Lines** — degrades from deaths and travel bans. Below 50%: policy costs increase 50%. Below 25%: medicine delivery takes twice as long. At 0%: no medicine can be deployed.
- **Civil Order** — degrades from deaths, restrictive policies, and healthcare collapse. At 0%: spread rate increases 50%.

The cascade is what makes this system work. Healthcare fails → more deaths → supply lines fail → can't deliver medicine → MORE deaths → civil order collapses → disease runs unchecked. Each failure accelerates the others. The player must decide which system to shore up first — or whether to abandon the region entirely.

Regions **collapse** when deaths exceed their threshold (typically 45-85% of population). Collapse is permanent — policies clear, refugees flee to neighbors (spreading disease), personnel are lost. When all six regions collapse, the game is over.

### Field Operations

Personnel-based missions that cost time and people, not money. Accessed via the `[O]` Operations panel.

- **Recon Mission** (2 personnel, 1.5 days) — partially identifies an unknown pathogen, adding 25% knowledge. Faster than waiting for field research but ties up personnel.
- **Emergency Response** (3 personnel, 1 day to deploy) — reduces lethality by 25% in a target region for 3 days. Buys time when a region is spiraling.
- **Infrastructure Survey** (2 personnel, 2 days) — repairs the worst infrastructure system in a region by 15%. Cheaper than the ¥200 policy repair but slower and limited to one system.

Operations create a personnel-vs-money trade-off: you can spend funding on policies and repairs, or spend personnel on field ops. With 20 starting personnel also needed for research and hospitals, committing 2-3 to a field op is a real cost.

### Game Over

One loss condition:
1. **All regions collapsed** — immediate defeat

The defeat screen shows duration, death toll, collapse timeline, pathogen report, score, and strategic tips specific to what happened in that run.

## Crisis Event Writing Standards

Crisis events are situation reports. State what happened and let the player respond without editorializing about what it means.

In 2050, authority flows from satisfying individual power-holders, not from managing public perception. Write events that reflect who actually cares about what happened: your board members, patrons, and governors. Read the PatronDemand events in `crisis.rs` before writing anything new. That's the register: named individuals, direct statements, no commentary on what the player should feel.

The following words belong to 2020s institutional and media vocabulary, not 2050 crisis text:

- `"transparency"` / `"go transparent"`: write the actual action ("issue a statement", "acknowledge the data")
- `"misinformation"` / `"disinformation"` / `"infodemic"`: describe what mechanically happens ("compliance dropping", "treatment rates falling", "leak circulating")
- `"civil unrest"`: say what's actually happening ("perimeter breached", "staff evacuating")
- `"member states"`: use corporations or named individuals

Format: descriptions are 1-3 sentences, option labels describe the action, and option descriptions state the cost. Don't use em dashes.

## What the Game Is Not

- **Not Plague Inc.** You're defending, not attacking. But more importantly: Plague Inc. is a puzzle game where you optimize a single disease against a static world. This game is about managing cascading systems that interact in emergent ways.
- **Not a tower defense game.** You don't place defenses and watch. You make decisions under uncertainty with incomplete information, through institutions and people you don't fully control.
- **Not a lesson.** The game doesn't teach you about pandemics. It puts you in a situation and lets you draw your own conclusions from what happens.

## Design Priorities

1. **Interesting decisions under pressure.** Every action should have a meaningful trade-off. If there's a clearly correct choice, the design is wrong.
2. **Emergent complexity from simple systems.** Individual mechanics should be straightforward. The complexity comes from their interactions — mutation pressure drives re-trials which consume research capacity which leaves new threats unidentified which causes governor loyalty to drop which causes policy defiance which accelerates spread.
3. **The game should feel different at day 30 than at day 1.** Not because the rules changed, but because the situation has evolved in ways that create qualitatively different decisions.
4. **Respect the player's intelligence.** Don't explain. Don't editorialize. Show the situation, give the tools, let the player act.
