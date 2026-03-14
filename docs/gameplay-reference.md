# Gameplay Impressions

This document is a time-bound gameplay overview based on direct playtesting of one specific version of the game.

It is descriptive, not prescriptive.

It is not a design manifesto, not a balance bible, and not a statement of how the game should continue to work. It is meant to answer questions like:

- What is a normal run trying to do?
- What does the player usually have by day 5, day 10, or day 20?
- What counts as a cheap cost, a meaningful cost, or a huge cost?
- When does a new crisis or policy meaningfully matter versus bounce off the existing economy?

This snapshot is grounded in:

- manual snapshot play on the current branch state
- one representative manual run with normal containment and research (`seed 11`)
- one manual run focused on board and ledger interaction (`seed 17`)
- the built-in `competent_play_extends_survival` benchmark across seeds `0-19`

## Read This First

Treat everything in this document as gameplay impressions from a particular point in time.

The game changes frequently. Systems get redesigned, priorities shift, and previously central mechanics can become secondary or disappear entirely. Because of that:

- do not read this as a list of directives for how the game must behave
- do not assume the quantitative ranges here remain valid after major system changes
- do not use the level of detail here as evidence that every listed pattern is intentional or desirable

The detail is here to help new developers and agents absorb the overall feel of a run, not to lock the design in place.

When major systems change, this document should be annotated, not silently trusted. The annotation should say what changed and which parts of this document are no longer safe to rely on. Good examples:

- `Note: written before the board-budget rework. The budget pacing and meeting examples below should not be trusted without re-testing.`
- `Note: written before the ledger redesign. The ledger section below describes the older buy/sell and board-relationship behavior.`
- `Note: approval thresholds changed after this was written. The policy-unlock pacing in the early-game sections is probably stale.`

If enough core systems drift, the right fix is a new playtesting pass and a fresh rewrite, not trying to preserve this document as timeless truth.

## High-Level Shape

Across the runs I played, the game started as a focused outbreak-management game and then drifted into a broader systems-management collapse game.

The broad arc that showed up in those runs was:

1. A single hotspot appears and you scramble to identify it.
2. Chairman approval rises as the crisis becomes undeniable, unlocking stronger policies.
3. The first pathogen gets identified and the player starts the targeted-medicine pipeline.
4. Additional pathogens appear before the first one is truly solved.
5. The run becomes about triage: one anchor region, several secondary fires, board politics, contracts, and staffing shortages.
6. Later still, healthcare collapse, governor problems, board budget cuts, and region failure matter more than any single disease.

The game is unwinnable. In the current benchmark, competent automated play survived `48.3d` to `72.4d`, with most tested seeds landing in the `50-60` day range.

## Core Player Loop

In the manual runs I played, competent play repeatedly collapsed onto the same five questions:

1. Which region is the real anchor problem right now?
2. Which research project is most urgent: identification, study/trial, medicine development, or a tech unlock?
3. Is staffing or money the actual bottleneck this minute?
4. Can I afford to appease a board member or governor, or do I need to spend directly on containment?
5. Which new problem can be deferred, and which one becomes catastrophic if ignored for 2-3 days?

The game did not stay centered on one system for long. In the same 5-day span, I was often:

- turning on containment in the opening hotspot
- accepting or refusing contracts
- reacting to a governor crisis
- waiting on identification
- starting targeted-drug tech
- watching the board budget get cut at the first meeting
- noticing another pathogen has already appeared

## What The Main Systems Do In Practice

### Chairman Approval

Chairman approval is both a political meter and a policy gate.

- Early game starts around `10%`.
- In the runs I played, it rose quickly during the first week if the outbreak was clearly worsening and the board had not yet turned hostile.
- Important policy thresholds felt strong in play because they unlocked tools the player was already waiting for:
  - around `30%`: lighter political actions start to open
  - around `40-45%`: the first heavy containment options become realistic
  - around `60%+`: later authoritarian tools are in sight

In practice, the early runs often felt like waiting for approval to cross the next relevant threshold.

### Board Budget

> **Note (2026-03-13):** Budget formula changed. Visible infections now boost the board allocation, so the monotonic cuts described below are stale for mid-to-late game.

Board budget is separate from approval.

- In the runs I checked, it started as a fixed daily allocation, around `¥390-406/day`.
- It did not move smoothly with the outbreak from tick to tick.
- It changed at board meetings.

Observed example:

- `seed 11` started at `¥406/day`
- first board meeting at day `6.0` cut it to `¥362/day`
- second board meeting at day `13.5` cut it again to `¥306/day`

Because of that, anything that changes fixed budget is not just flavor text. In the sampled runs it changed the whole funding baseline the player was planning around.

### Contracts

Contracts mattered very early and often became a major share of income.

In practice:

- they started appearing around day `1`
- I usually accepted them when they matched the plan I was already on
- a few stacked contracts contributed roughly `+200` to `+300+ / day`
- in the representative run, they offset repeated board-budget cuts enough to carry a large share of the midgame economy

Observed values:

- early contract offers around `+82/day`, `+100/day`, `+157/day`
- by day `12-13` in `seed 11`, contracts were worth `+322/day`, roughly the same scale as the board budget itself

That makes contract conditions a real design lever. A contract that looks small in text can still matter if it forbids a policy the player would otherwise use by default.

### Research

Research felt like the real spine of the sampled runs.

The sequence that showed up most often was:

1. `Identify` the opening pathogen
2. once identified, use the new knowledge to unlock targeted capabilities
3. `Study` the pathogen to push knowledge to deployment-ready levels
4. develop a first targeted drug
5. run a trial before relying on it
6. keep identifying new pathogens as they appear

The main practical constraint was usually personnel, not funding.

Observed pattern:

- day `0`: starting a field project plus one containment policy often left about `13-18` free personnel
- by day `5-10`, free personnel in my runs often fell to `3-8` because containment, research, and crisis operations all drew from the same pool
- this is why new-pathogen crises started to hurt even when cash was still available

### Medicines

Medicines played in three recognizable phases in the runs I checked:

1. `Broad-Spectrum` is the emergency opening tool.
2. targeted drugs become available after the first identify/basic-research milestones.
3. later medicines become part of a rotation of trial, deploy, manufacture, and re-trial.

Practical notes from play:

- Broad-Spectrum did a lot of work early.
- Without enough screening or visibility, some doses were wasted.
- In the representative competent run, targeted-medicine development became part of the core plan by about day `10`.
- Once the midgame economy was online, trial costs and manufacturing costs were meaningful but no longer dominant.

Current observed targeted-drug examples:

- `¥300` / `2 personnel` / `2.0 days` for a fast, resistance-prone targeted antiviral
- `¥500` / `3 personnel` / `3.3 days` for a more balanced alternative
- `¥900` / `5 personnel` / `6.0 days` for a slower, more durable line

### Policies

Policies were not managed uniformly across the map. In the runs I played, they concentrated on one anchor region first.

What usually happened in those runs:

- a cheap cross-region measure goes on the opening hotspot early
- once approval rises enough, vector-matched containment goes on the main hotspot
- screening gets added where visibility or dose efficiency matters
- the player does not usually try to lock down every region equally

In observed runs:

- early `Border Controls` was a common first move because it is cheap and available early
- `Quarantine` became the main escalation once chairman approval crossed the mid-40s
- `Basic Screening` was often used to improve visibility and reduce wasted medicine

The basic pattern was:

- early spread control
- then vector-specific containment
- then later infrastructure and medicine support if the region is still the anchor disaster

### Governors

Governors mattered as friction, not just text flavor.

Even before outright defiance, they created:

- emergency spending demands
- priority demands
- corruption / medical-expense / evacuation crises
- cooperation drift that makes policies less reliable

The player is often paying to keep the machine moving, not because the payout is obviously efficient in isolation.

### Ledger

The ledger was optional but not fake.

What it did in practice in the run where I exercised it:

- lets the player buy and sell in `10-share` chunks
- provides a way to take positions in corporations that are also tied to board members
- creates relationship effects when investing in a board member's company

Observed example:

- buying `10` shares of `Volant Industries` at `¥55.7/share` cost `¥557`
- the transaction message explicitly noted: `Chairman Salazar approves`

But the ledger did not override everything else:

- stock performance still dominates corporate satisfaction
- accepting contracts can anger other members at the same time
- a board member can still be only `Wary` after you invest if their company is under pressure or other modifiers are negative

Based on that run, the ledger reads more like a side lever for relationship management and speculation than a core economic system.

## Typical Flow By Stage

### Days 0-2

This was the outbreak-establishment phase in every run I checked.

Typical state in those runs:

- one starting hotspot with a few hundred infected
- total dead still in the tens
- `Funds` around `¥500` at start
- `Chairman approval` around `10%`
- fixed board budget around `¥390-406/day`

Typical player actions:

- turn on Broad-Spectrum auto-deploy
- put a cheap first containment policy on the hotspot
- start `Identify`
- accept an early contract if it supports the existing plan

What mattered most:

- getting the first research project moving immediately
- not overspending before the first contract or first few days of income arrive
- not wasting approval on a policy that is too expensive or misaligned with the actual threat

### Days 3-6

This was the first real decision pivot in the sampled runs.

Typical state from observed runs:

- `Funds` often around `¥1200-2000`
- `Chairman approval` often around `30-50%`
- global infections can still be numerically small, roughly `600` to `5K`
- deaths were often in the `200-1300` range
- first pathogen identification usually finishes here

What changed here:

- the player knows the pathogen class and vector
- stronger policies become available because approval rose
- the first board meeting may happen as early as day `6`

Typical player actions around this stage:

- add `Quarantine` or other vector-matched containment on the anchor region
- start `Targeted Drug Design`
- start `Study`
- react to the first governor and contract crises

Developer calibration from this slice of play:

- `¥150` is a small-but-real cost here
- `¥350` is a moderate cost
- `¥500+` is a serious commitment if the player already has active policies

### Days 7-12

By this stage, the game no longer felt like a single-outbreak response game.

Observed state:

- in the sampled runs, the first board meeting had often already cut the fixed budget
- `Chairman approval` was often in the `60-70%` range
- `Funds` were often around `¥1800-2600`
- global infected was often in the `25K-100K+` range
- deaths were often in the `5K-25K+` range
- second and even third pathogens were already appearing

Typical player experience from these runs:

- the first targeted-medicine tech unlocks
- the player is choosing between:
  - identifying a new pathogen
  - trialing the first drug
  - sequencing or re-trial work
  - training personnel
  - starting infrastructure tech

This is also where personnel shortage became obvious. Money was often still available; free staff often was not.

### Days 13-20

This was the start of recognizable midgame triage in the representative run.

Observed state in the representative run:

- a second board meeting had already cut the fixed budget again
- contracts were worth roughly `+322/day`
- global infected was around `200K-260K`
- deaths were around `50K-70K`
- the anchor region was already `CRIT`
- anchor-region healthcare had fallen into the `mid-60s to high-70s`

What the player is often thinking about now:

- which pathogen gets staffing first
- whether the anchor region is salvageable
- whether to spend on immediate governor/board appeasement or on direct medical action
- whether a new tech like `Resilient Grids` is worth delaying another urgent project

By this stage, event costs around `¥350-500` felt normal, but not free.

### Days 20+

The exact shape varied a lot by seed, but the benchmark and my prior manual play support the following as a rough current-build picture:

- several pathogens are active
- at least one region is visibly failing
- chairman approval can remain high even while the board budget is shrinking
- the player is living on a mix of fixed budget plus contract income
- infrastructure failure, delivery efficiency, and regional politics matter as much as raw infection numbers

By the deep late game, runs were no longer about "stopping the disease" in a clean sense. They were about:

- preserving one or two important regions
- keeping healthcare or supply lines from snapping
- surviving budget cuts and political crises
- delaying collapse long enough for a few more days of survival

## Quantitative Cost Scale

These are rough practical scales from the observed runs. They are for calibration, not as fixed design targets.

### Small

Usually small enough to take without much thought unless the player is already broke.

- around `¥75-150`
- examples: lab upgrade, some utility actions, personnel training

### Moderate

Usually requires an actual choice, especially before day 10.

- around `¥200-400`
- examples: many crisis payments, trial/re-trial actions, early identification, governor appeasement-style spending

### Large

Usually feels like a real strategic commitment, especially if staffing is also attached.

- around `¥500-900`
- examples: major techs, slower durable medicines, larger political demands

### Very Large

On the current build, this reads like a pivot or a late-game move.

- `¥1000+`
- best reserved for major board/corporate pressure, extraordinary emergency actions, or strong upside

## Practical Resource Ranges

These are rough current-build numbers from a small number of manual runs plus the benchmark. They are the first place I would expect to drift after balance changes.

- Starting funds: about `¥500`
- Starting board budget: about `¥390-406/day`
- Day 5 funds in a competent run: often `¥1700-2000`
- Day 10 funds in a competent run: often `¥2100-2600`
- Contract income once stacked: roughly `+200` to `+320+ / day` in the runs I checked
- Early daily net income after a few policies: often still around `+300` to `+500/day` in the sampled runs

Personnel:

- Start at `20`
- Free personnel can drop under `10` quickly once field research and policy enforcement are active
- In the runs I played, mid-early play felt personnel-constrained before it felt money-constrained

## What Felt Meaningful In The Sampled Runs

- early contracts that align with the current plan
- the first board meeting
- vector-matched containment unlocks
- first targeted-medicine choices
- whether to spend personnel on new identification versus finishing the current disease pipeline
- whether to spend money to stabilize a governor crisis or absorb the political fallout

## What Felt Lower Priority In The Sampled Runs

In the observed runs, these felt secondary rather than central:

- symmetrical management of every region
- speculative expansion of containment before a region becomes a real exporter
- ledger play before the player has spare funds

In practice, the ledger only started to compete with core medical spending once I could spare several hundred yen without obviously compromising the medical plan.

## Guidance For Event And Content Authors

If you are adding a new crisis, contract, policy, or operation, calibrate it against the current run structure. The exact breakpoints will drift, but the following scale was a useful fit for the current build:

- A `¥50` effect is tiny unless it chains into something else.
- A `¥150` effect is noticeable early but can become routine fairly quickly.
- A `¥350` effect is roughly a normal crisis-sized tradeoff on the current build.
- A `¥500-900` effect wants a strong upside or real protection from future damage.
- A daily income effect around `+80` is relevant early.
- A daily income effect around `+150` is strong.
- A daily income effect around `+250+` can materially reshape the run.

Non-money costs matter just as much:

- `2-3 personnel for several days` is often a painful commitment
- an approval hit that locks out a policy threshold can matter more than a flat cash loss
- a board-budget cut can matter more than a one-time payment

The right question is not just "how much does this cost?" It is:

- what day does this usually appear?
- is the player currently money-constrained, personnel-constrained, or approval-constrained?
- does this compete with the anchor-region response, the research pipeline, or both?

## Summary

At the moment, the game is best understood as:

- a survival-management game
- with a strong early research/containment loop
- that quickly turns into staffing triage and political economy management
- while diseases continue to multiply faster than the player can fully solve them

For new developers, the most important intuition is this:

The player is rarely choosing between a good option and a bad option. More often, they are choosing which important problem gets to stay unsolved for a little longer.
