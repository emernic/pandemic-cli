---
name: crisis-events
description: "MANDATORY when creating, editing, or reviewing crisis events. Read this BEFORE touching any crisis event code. Do not skip. Do not skim. Crisis events are the most important user-facing content in the game and there is no linter, no test, and no automated quality gate. You are the only thing standing between the player and tedious, incoherent, thematically wrong garbage."
---

# Crisis Events

## Why This Skill Exists

Crisis events are the single most important user-facing content in the game. They pause the game, demand the player's full attention, and force a decision. Every crisis event that fires is a promise to the player: "this is worth stopping what you're doing for." If the event is boring, meaningless, mechanically broken, or thematically wrong, you have lied to the player and wasted their time. There is no recovery from that. They will start auto-resolving crises, disengaging from the content, and the game dies.

There is no linter for crisis events. There is no test that catches "this option is obviously always correct" or "this description uses language from the wrong century." There is no CI check for "does this event justify pausing the game?" **You are the quality gate. If you do a bad job, nobody catches it.** The last time someone reviewed crisis events without this skill, they rubber-stamped dozens of broken, boring, incoherent events as "fine" and "good" and "no issue needed." It took the project owner seven increasingly furious interventions to force a real review. Do not repeat that failure.

**Before you write or edit a single line of crisis event code, read this entire document. Then read `docs/setting.md`. Then read `docs/framing-guide.md`. Then read `docs/naming-style.md`. Then read the `/flavor-text` skill. Then come back here and re-read the checklist at the bottom.** If you skip any of these, you will produce wrong-world, wrong-tone, mechanically broken content. You will not notice because it will feel right to you. It is not right. Read the documents.

---

## The Bar

Every crisis event must clear ALL of these. Not most. ALL.

### 1. It must justify pausing the game

The game is real-time. When a crisis fires, everything stops. The player was in the middle of managing a pandemic. They were watching infection numbers, planning research, juggling policies. You interrupted all of that. The event must be worth the interruption.

**Events that do NOT justify pausing the game:**
- "Pay ¥200 or lose a small thing." That's a toll booth, not a crisis.
- "1-3 staff are out for a few days." That's Tuesday. Nobody cares.
- "A number goes up or a different number goes down." If the only consequence is shuffling approval/cooperation by single-digit percentages, the event is tiddlywinks.
- Any event where the player reads the options, picks the obvious one, and moves on without thinking. If the player doesn't have to think, don't make them stop.

**The test:** Would a human player groan when this event pops up for the third time? If yes, cut it.

### 2. It must create a real decision

A real decision means: the correct answer depends on the player's situation. Different game states produce different correct answers. If one option is always right, or one option is always wrong, the event is broken.

**The most common failure mode is the "tax event."** A tax event says: "Pay money or lose a thing." The paid option is obviously correct whenever you have money. The only question is "are you broke?" That is not a decision. That is a wallet check. Tax events waste the player's time.

**The second most common failure mode is the dominated option.** Option A and Option B are both free. Option A is strictly worse than Option B in every scenario. No rational player picks A. The event has fewer real options than it displays. Examples from this codebase's history:

- Lab Accident: "Evacuate lab" (guaranteed loss) vs "Leave it" (30% chance of keeping research, same worst case). Evacuate is strictly dominated. Dead option.
- Quarantine Riots: "Wait it out" (free, keep quarantine, -3% approval) vs "Hire security" (-15% approval, 2 personnel). Wait dominates in every scenario.
- Healthcare Worker Collapse: "Push through" (lose N personnel) vs "Ignore warnings" (lose N/2 personnel, same outcome). Push through is strictly dominated.
- Treatment Noncompliance: "Enforce compliance" (-10% approval, -15 cooperation, possible follow-up crisis) vs "Accept noncompliance" (-5% approval, 10% infection bump). Enforce is dramatically worse by every measure.

**Before shipping any crisis event, check every pair of free options and ask: "Is there ANY scenario where a rational player picks the more expensive one?"** If no, you have a dead option. Fix it or remove it.

**The third failure mode is the "trap option."** An option that sounds strong and decisive ("Enforce compliance!", "Push through!") but is mechanically much worse than a passive-sounding option ("Accept noncompliance", "Ignore warnings"). This punishes players for making the choice that feels right and rewards passivity. It's bad design.

### 3. It must involve a named actor

This game has named governors with personalities. Named board members with corporate affiliations. Named corporations with directors and sectors. Named chairmen. These people have relationships with the player that the player has been managing for the entire game.

**A crisis event with no named actor is a crisis event with no stakes.** "A cold chain failed." Who cares? "Kestrel Freight's cold chain failed and Dir. Mensah is blaming your quarantine policies" connects to a relationship the player is invested in.

Events that say "the regional administration," "a pharmaceutical consortium," "an organized resistance group," "desperate people," or "a corrupt official" are wasting the game's character system. There are named people in this game. Use them.

**The only exceptions** are pure system events (BoardMeeting, NewPathogenDetected, LoanOffer) where the content is mechanically generated and the "actor" is the game system itself.

### 4. Stakes must be real and proportionate

**Tiddlywinks costs that do NOT justify a game-pausing crisis:**
- 1-2 personnel for a single-digit number of days
- ¥80-200 at any stage of the game
- 3-5% approval swings
- Small cooperation adjustments (5-10 points on a 0-100 scale)

These are background noise. They do not justify interrupting the player. If your event's biggest consequence is "2 staff are busy for 3 days," the event should not exist.

**Real stakes look like:**
- Permanent personnel loss of 3+ staff
- Policy removal or forced activation
- Region collapse or uncollapse
- 10%+ approval swings
- Lasting mechanical effects (skim rate changes, ongoing funding drains, medicine efficacy penalties)
- Chain consequences that escalate based on the player's choice

### 5. Description claims must be backed by mechanics

If the description says "civilians are abandoning health protocols," then health protocol compliance must actually change in the game state. If the description says "policy enforcement has stalled," then policy enforcement must actually stall.

**This is one of the most insidious failure modes.** The description creates atmosphere and tension by describing real operational problems. But the mechanics are just approval shuffling. The player reads "reporting systems are failing, noncompliance is rising" and expects something to happen in their game. Nothing does. Approval changes by 5%. The description lied.

Events where the description describes consequences that the mechanics don't deliver include (from this codebase's history):
- Communications Failure: "Reporting systems returning inconsistent data, noncompliance rising." Mechanics: approval changes. No reporting or compliance effects.


**Rule: If you can't or won't implement the mechanical effect, don't describe it.** Write a description that matches what actually happens.

### 6. It must fit the game's world

Read `docs/setting.md` before writing any crisis event. The short version:

**The year is 2050.** Not 2020. Not the 20th century. Institutions are gone or irrelevant. Power is concentrated in individuals who control energy, technology, and physical infrastructure.

**The N.W.H.O. is not a government.** It issues directives, not mandates. It negotiates, not commands. It has no military, no police, no sovereign authority. Its leverage comes from the board's corporate economic weight.

**The board is not a medical ethics committee.** Board members are corporate leaders who care about their bottom line. They want results. They pressure you to skip trials. They would NOT reward you for halting drug deployment on safety grounds. They would NOT punish you for continuing treatment. They WOULD reward commercialization deals. If your crisis event gives +approval for "doing the right thing" and -approval for "selling out," you have the board backwards. The board IS "selling out."

**There are no nation-states, intelligence agencies, congresses, or oversight commissions** in the game's power structure. "Foreign intelligence," "classified data," "congressional hearing," "oversight commission" are all from a world that doesn't exist in this game. Power flows through corporations, board members, and governors.

**"Warlord" is not a 2050 actor.** Someone controlling a collapsed region in 2050 would be a corporate security operation, a governor's network, or a board member's subsidiary. Not a generic warlord.

**Wrong-world terms to never use:** classified, federal, mandate (as sovereign command), foreign intelligence, misinformation, public trust, international community, civil unrest (say what's actually happening), member states, warlord, vaccine hesitancy, congressional.

### 7. No jokes, no winking, no meta-commentary

The game works through overidentification: playing corporate power so straight, so earnestly, that the absurdity becomes visible without ever breaking character. The game NEVER comments on its own themes.

**The Performance Review works** because it states the bureaucratic agenda (KPI alignment, coffee situation) alongside the death toll and lets the player feel the absurdity. It never says "isn't this absurd?"

**Naming Rights failed** because the rename options were "Karen-7," "BrandSynergy-X," "CEO's Regret." These are punchlines. The game winking at the camera. The moment the game comments on its own themes, the overidentification breaks and the player is watching a satire instead of living in the world.

**If a crisis event makes you chuckle while writing it, it's probably wrong.** The humor should come from the situation, never from the text.

### 8. No em dashes

This is in the `/flavor-text` skill but it bears repeating because crisis events are the most common place they appear. Em dashes are the #1 tell of AI-generated text. Never use them. Not once. Write real sentences.

---

## The Approval Direction Test

This deserves its own section because it was wrong on multiple events and the error pattern is consistent.

Before assigning approval rewards to any crisis option, ask: **"What would the board — a group of corporate leaders whose companies control energy, logistics, mining, data infrastructure, and automation — actually think about this?"**

The board is NOT:
- A medical ethics committee
- A democratic accountability body
- A humanitarian organization
- A group of people who value "doing the right thing"

The board IS:
- Corporate leaders who want their operations protected
- People who evaluate you on results, not process
- People who pressure you to skip trials and deploy faster
- People who would approve of commercialization deals
- People whose satisfaction tracks corporate stock performance

**Common approval-direction errors:**

| Event | Wrong | Right |
|-------|-------|-------|
| Halt drug deployment on safety grounds | +approval | -approval (board wants deployment) |
| Continue deployment despite side effects | -approval | +approval (board wants results) |
| Accept naming rights deal | -approval | +approval (board likes commercialization) |
| Decline naming rights deal | +approval | -approval (board wanted the money) |
| Share research data when demanded | situational | depends on WHO demanded it |

---

## The "Should This Event Exist?" Test

Before creating a new crisis event, or before deciding an existing one is worth keeping, run through this:

1. **Does it justify pausing the game?** If the stakes are small and the decision is obvious, it shouldn't exist.
2. **Does it create a decision where the right answer depends on the player's situation?** If one option is always correct, it shouldn't exist.
3. **Does it involve a named actor from the game's power structure?** If nobody specific is involved, it probably shouldn't exist.
4. **Are all options meaningfully distinct?** If any free option is strictly dominated by another free option, it's broken.
5. **Do the description's claims match the mechanical effects?** If the description describes consequences that don't actually happen in the game state, it's lying to the player.
6. **Does it fit the 2050 corporate-power world?** If it uses 20th-century institutional framing, it doesn't belong.
7. **Is the approval direction correct for the board's actual character?** If the board rewards "doing the right thing," it's backwards.
8. **Is it NOT a tax event?** If the entire decision is "pay money or lose a thing" with an obvious correct answer, it shouldn't exist.
9. **Is it NOT a coin flip?** If the outcome is random with no information for the player to reason about, it shouldn't exist.
10. **Would the player be annoyed to see this event for the third time?** If yes, cut it.

If an event fails ANY of these, do not create it. If an existing event fails any of these, remove it. Do not "fix" or "tweak" a fundamentally broken event. Remove it and, if the concept is worth exploring, build something new from scratch.

---

## Writing the Event Text

After your event passes the existence test, write the text. Read `/flavor-text` and `/humanizer` before writing.

**Description:** 1-3 sentences. State what happened. Name the actor. State the concrete problem. Do not editorialize. Do not tell the player how to feel. Do not use em dashes.

**BAD:** "A devastating cold chain failure has struck your supply network, threatening to destroy precious medical supplies that countless lives depend on."

**GOOD:** "Kestrel Freight reports a refrigeration failure at their distribution hub. 4,200 doses of Antiviral-C are in the affected shipment."

**Option labels:** Short action phrase. Include the cost in the label if there is one. The player should be able to see the trade-off without reading the description.

**Option descriptions:** State what happens mechanically. Do not editorialize. Do not say "this is risky" or "this could have consequences." State the actual consequence.

**BAD option description:** "A bold move that could pay off, but carries significant risk to your standing with the international community."

**GOOD option description:** "Dir. Whitford's supply contracts are suspended. Kestrel Freight retaliates: supply lines -10% in South America."

---

## Reviewing an Existing Event

When reviewing a crisis event (during a commissar cycle, an audit, or a fix-up task), evaluate it against every criterion in "The 'Should This Event Exist?' Test" above. Do them one at a time. Do not batch-evaluate multiple events. Do not say "these five events are all fine" without individually justifying each one.

**The default assumption is that the event is bad.** You must prove it's good, not prove it's bad. The codebase has a long history of crisis events that were "fine" on first glance and turned out to be dominated-option, tiddlywinks-stakes, wrong-world, approval-backwards garbage on closer inspection. Your first instinct will be to say "this is fine." Your first instinct is wrong. Push harder.

**For each event, write out your full reasoning** before making a judgment. Think about:
- Who is involved and do they have a name?
- What does each option actually do mechanically? (Read the resolution code, not just the build code.)
- Is any free option strictly dominated by another free option?
- Is the paid option so cheap that it's always obviously correct?
- Are the stakes big enough to justify pausing the game?
- Does the description match what the mechanics actually do?
- Are the approval rewards going in the right direction for this board?
- Does the framing fit the 2050 world?

---

## Mechanical Checklist for Implementation

When implementing a crisis event in code:

- [ ] At least one option must be free (no `cost`) so the player is never softlocked
- [ ] Use `scaled_cost()` for any monetary cost. NEVER hardcode a flat yen amount. Flat costs become trivial mid-game or bankrupting early-game.
- [ ] Personnel operations of 1-2 staff for 1-5 days are tiddlywinks. If that's your "expensive" option, the event doesn't justify existing.
- [ ] Verify no free option is strictly dominated by another free option
- [ ] Add the crisis type tag to `phase_weight()` with appropriate game-day weighting
- [ ] Add a cooldown tag so the same event type doesn't spam
- [ ] If the event has a follow-up chain, make sure the entry point decision is close enough that both paths actually fire in practice. If one path is obviously correct, the chain content is wasted.
- [ ] Test that the resolution code actually does what the option descriptions claim
- [ ] Run `/flavor-text` and `/humanizer` on all player-visible text
