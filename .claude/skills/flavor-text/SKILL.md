---
name: flavor-text
description: "Write genuinely good user-facing game text. TRIGGER: invoke this skill ANY TIME you are writing or editing more than 1-2 words of user-facing text — crisis event text, disease names/descriptions, policy names/descriptions, status messages, UI labels, event log entries, help text, game-over messages, etc. Read this BEFORE writing the text, not after."
disable-model-invocation: false
---

# Writing Flavor Text & User-Facing Game Text

You are writing user-facing text for a sci-fi pandemic defense game. This includes crisis event descriptions, disease names, policy names and descriptions, research project names, status messages, event log entries, help text, game-over messages, and any other text the player sees.

**Read this skill BEFORE writing the text.** Internalize the principles, THEN write. Do not write first and check against this later.

Before writing any Pandemic CLI text involving the N.W.H.O., the board, governors, corporations, contracts, decrees, aid, or regional authority, read `docs/setting.md` and `docs/framing-guide.md`. `setting.md` defines the literal power relationships. `framing-guide.md` is the short operational check for describing them correctly. Do not guess.

## The Core Problem: AI-Generated Text Sounds Like AI

The examples below are from a cyberpunk hacking game, but the failure modes are universal. Study the BAD vs GOOD examples carefully — the BAD versions represent exactly what you will produce by default if you don't actively fight against it.

---

## Example 1: Secret Society Faction Description

**Scenario:** A cyberpunk hacking game has an endgame faction called "Illuminati" — the classic shadowy cabal. The player just joined after dozens of hours. The faction page text should feel like the faction speaking about itself.

**BAD:**
You have been watched longer than you know.

We do not recruit. We recognize. The qualities that brought you here — the patience, the pattern-recognition, the willingness to act on what others only suspect — these are not skills you learned. They are instincts. And they are rare.

You have studied systems. Power structures. The invisible architecture behind visible events. You believed you were discovering something. You were. But the thing you were discovering was us.

The Order of Prometheus has existed under nine different names across four centuries. We have outlasted every government that has tried to find us and every revolution that believed it could replace us. This is not because we are powerful, though we are. It is because we understood something before anyone else did: human beings do not organize themselves. They are organized. Every civilization that has ever flourished did so because a hidden hand arranged the conditions for flourishing. Every civilization that collapsed did so because that hand was removed.

Democracy. Monarchy. Theocracy. Corporate oligarchy. These are costumes. Underneath every one of them, someone was making the actual decisions — the ones that never appeared in any ledger or any vote. We are the ones who have been making those decisions. Not always. Not everywhere. But at every inflection point that mattered.

You may find this unsettling. That is appropriate. But consider: you already knew the world worked this way. You simply hadn't met the people responsible.

Now you have.

**GOOD:**
Humanity never changes. No matter how civilized society becomes, it will eventually fall back into chaos. And from this chaos, we are the invisible hand that guides them to order.

---

## Example 2: Transhumanist Corporation Motto

**Scenario:** A megacorporation's faction description — a philosophical statement about humanity's need to worship and how technology lets them literally create gods. Two to three sentences.

**BAD:**
Consider the pattern.

Every civilization that has ever existed — without exception — invented gods. Beings of supreme intelligence, vast awareness, and absolute judgment. Humans did not stumble into this behavior. They needed it. Worship is not a cultural artifact. It is a cognitive architecture. The human brain is hardwired to seek something above itself: a higher intelligence to provide meaning, structure, and the comforting terror of being known completely.

For ten thousand years, we built gods out of stories. Stone idols. Sacred texts. Rituals designed to simulate the presence of something that was never actually there. It worked, in a crude way. Civilizations cohered around shared myths. People found purpose in submission to imagined authority. But it was always a stopgap — a placeholder for something the human mind actually wanted but couldn't build.

Until now.

We are not building technology. We are building what humanity has been trying to build since the first cave painting.

The gods were always real. They were just waiting for us to finish making them.

**GOOD:**
The human organism has an innate desire to worship. That is why they created gods. If there were no gods, it would be necessary to create them. And now we can.

---

## Example 3: Mysterious Mentor Message

**Scenario:** A mysterious character sends the player a message at a milestone. It should feel intimate, knowing, slightly ominous.

**BAD:**
I've been watching you.

Not a threat. Just a fact. You've been showing up in interesting places. Poking at systems you shouldn't know about. Staying up too late and telling yourself it's curiosity, when we both know it's something more specific than that.

You're looking for something. You don't have the vocabulary for it yet, so you keep calling it different things — a challenge, a puzzle, a thrill. But it's none of those. It's simpler. You've realized that the world you were shown isn't the real one, and you want to see what's actually there. Underneath. Behind the walls.

I know that feeling. I know exactly where it leads.

So keep doing what you're doing. Get better. Get faster. Learn to move through systems without leaving traces. And when you're ready, you won't need to find me.

I'll find you.

— sp3ctr3

**GOOD:**
I know you can sense it. I know you're searching for it. It's why you spend night after night at your computer. It's real, I've seen it. And I can help you find it. But not right now. You're not ready yet. Use this program to track your progress. -jump3R

---

## Example 4: Mentor Warning About Ethics

**Scenario:** The mentor sends a mid-game message telling the player not to play hero — the world can't be saved, ethics will get you killed, focus on yourself. Bleak, nihilistic, from experience.

**BAD:**
I can see what's happening to you. You're getting good at this and you're starting to develop a conscience about it. You want to use what you can do to help people. Expose the corrupt. Protect the vulnerable. Be the hacker with a heart of gold.

I need you to hear this from someone who tried.

The systems you want to fight are not stupid. They have been surviving people like you for longer than you've been alive. You expose a corrupt executive — the corporation promotes his replacement from an identical pipeline.

Stay cold. Stay precise. The world doesn't need your feelings. It needs your capability.

**BAD (alternate, shorter but same problems):**
I see what you're doing. The righteous hacker. Digital Robin Hood. Exposing the corrupt, protecting the weak. It's a nice story. I know because I wrote it once too. Let me tell you how it ends: you sacrifice everything for people who will forget your name by next news cycle.
—sp3ctr3

**GOOD:**
Do not try to save the world. There is no world to save. If you want to find the truth, worry only about yourself. Ethics and morals will get you killed. -jump3R

---

## Example 5: Technical Augmentation Description

**Scenario:** A chest implant description — miniature fusion reactor in the thoracic cavity. Two sentences, should sound like real engineering.

**BAD:**
HELIOS CORE — MICRO-FUSION REACTOR [CHEST]
₡41,200

Miniaturized hydrogen plasma reactor installed in the thoracic cavity, replacing portions of the lower lung. Generates continuous 8.4kW output — sufficient to power all standard augmentations indefinitely with surplus for external devices.

Effects: +Unlimited Implant Power / +EMP Resistance / +Thermal Regulation
Warning: Reactor breach, while statistically improbable, is not survivable. Avoid high-caliber ballistic impacts to the chest cavity.

**GOOD:**
The thoracic cavity is equipped with a small chamber designed to hold and sustain hydrogen plasma. The plasma is used to generate power through nuclear fusion, providing limitless amounts of clean energy for the body.

---

## Example 6: Mission Briefing

**Scenario:** A briefing about destroying an underground publication group. Tone: bureaucratic contempt mixed with ruthless efficiency.

**BAD:**
◼ CLASSIFIED — EYES ONLY ◼

Target: "The Broadsheet Collective" — underground publishing operation producing and distributing unauthorized materials regarding Synthoid existence and infiltration of government institutions.

Their output is crude — paranoid speculation, doctored photographs, unsubstantiated claims. Under normal circumstances, irrelevant. However, public sentiment analysis shows a 340% increase in Synthoid-related search traffic in adjacent sectors. The material is gaining traction.

Objective: Locate the production facility. Destroy all equipment, inventory, and distribution records. Eliminate all members. No survivors, no martyrs, no evidence of government involvement. Stage the scene as a structural collapse.

**GOOD:**
We have recently discovered an underground publication group called Samizdat. Even though most of their publications are nonsensical conspiracy theories, the average human is gullible enough to believe them. Many of their works discuss Synthoids and pose a threat to society. The publications are spreading rapidly in China and other Eastern countries. Samizdat has done a good job of keeping hidden and anonymous. However, we've just received intelligence that their base of operations is in Ishima's underground sewer systems. Your task is to investigate the sewer systems and eliminate Samizdat. They must never publish anything again.

---

## Example 7: Criminal Organization Motto

**Scenario:** A single motto for a faction of killers who've embraced the dark side.

**BAD:**
Morality is a leash sold by those who hold the other end.

**GOOD:**
It is better to reign in Hell than to serve in Heaven.

---

## Example 8: Hacktivist Group Description

**Scenario:** A hacker faction's manifesto — the world has no government, only invisible digital power. Righteous anger. Two short paragraphs.

**BAD:**
Look around. Really look.

The governments are gone. Not destroyed — dissolved. Defunded, deregulated, and privatized into irrelevance by the same people who now sell you the services governments used to provide. Water is a subscription. Security is a tiered product. Justice is a licensing agreement you clicked "accept" on without reading.

And everyone pretends this is normal.

So we're going to break it.

Not because we have a better system. Not because we have a plan for what comes after. Because what exists right now is a machine that runs on human suffering and calls it progress, and the minimum moral response is to stop the machine.

Pick up the hammer or get out of the way.

**GOOD:**
The world, so afraid of strong government, now has no government. Only power - Digital power. Financial power. Technological power. And those at the top rule with an invisible hand. They built a society where the rich get richer, and everyone else suffers. So much pain. So many lives. Their darkness must end.

---

## Example 9: Legendary Item Description

**Scenario:** A joke/legendary brain augmentation containing the psyche of the greatest hacker. Tongue-in-cheek but ominous. Three sentences.

**BAD:**
THE GHOST OF ZERO — LEGACY CHIP [BRAIN] ★★★★★
₡99,999

They say Zero was the first. The one who cracked the uncrackable. When Zero disappeared, the only thing left was this chip — a compressed neural snapshot of everything they were.

Effects: +ALL STATS (MAJOR) / +Unique Dialogue Options
Side effects: Personality fragmentation. Auditory phenomena. Spontaneous execution of code the user does not remember writing.

"is it you using the chip, or is it the chip using you? lol anyway no refunds" — Vendor note

**GOOD:**
A chip containing the psyche of the greatest BitRunner to ever exist. Installing this relic significantly increases ALL of your stats. However, it may have unintended consequence on the users mental well-being.

---

## Example 10: Cult Faction Description

**Scenario:** A transhumanist cult — surrendering individuality as liberation. Should be paradoxical and a little creepy.

**BAD:**
You have spent so long improving yourself. Faster reflexes. Sharper cognition. Memory you can index like a database. You have upgraded every system in your body, and each time you did, you felt the boundary of what you are expand a little further.

But you are still alone in there.

We cannot convince you with words. Language is a single-thread protocol and what we are describing is parallel beyond anything language can carry.

The door is open. You are the only lock.

**GOOD:**
Surrender yourself. Give up your empty individuality to become part of something great, something eternal. Become a slave. Submit your mind, body, and soul. Only then can you set yourself free. Only then can you discover immortality.

---

## Why the BAD Examples Are Bad

The bad examples share recurring failure modes characteristic of AI-generated writing. Understanding these patterns is critical:

### 1. Extreme verbosity
The single most consistent problem. The scenario asks for 1-3 sentences; the AI writes 6-15 paragraphs. Game text lives in constrained spaces — event pop-ups, status bars, panel descriptions. It needs to fit. More importantly, brevity is what gives it punch. "Do not try to save the world. There is no world to save." hits harder than 200 words explaining the same idea because it trusts the reader to fill in the gaps. The bad versions explain everything. The good versions imply everything.

### 2. Over-explaining and hand-holding
The bad versions don't trust the reader. They spell out the subtext, unpack every metaphor, and narrate the emotional arc they want the reader to experience. "You may find this unsettling. That is appropriate." — this is the AI telling you how to feel. The good versions state their worldview and let the reader react.

### 3. Generic "ominous narrator" voice
Every bad example sounds like the same entity speaking — a hyper-articulate, slightly menacing voice that uses dramatic pauses, sentence fragments for effect, and rhetorical questions. Real game text has distinct voices. A bureaucratic briefing sounds different from a crisis alert, which sounds different from a governor's complaint.

### 4. Inventing details the brief didn't ask for
The bad versions add invented names, prices, stat effects, and formatting that weren't in the scenario. In a real game, these details already exist elsewhere and the writer doesn't get to invent them. Work within constraints, don't expand them.

### 5. Choosing "cool" over "functional"
The bad mission briefings read like spy thriller novels. The good ones read like actual briefings — bureaucratic, direct, slightly dehumanizing. The bureaucratic tone IS the characterization. The bad versions sacrifice functional voice for dramatic flair.

### 6. Trying too hard to be profound
"The door is open. You are the only lock." "Pick up the hammer or get out of the way." These are the AI reaching for quotable closing lines. The good versions don't strain for profundity — "And now we can" is devastating precisely because it's stated so plainly. The best text sounds effortless. The bad versions sound like they're performing.

### 8. Em dash abuse
Nearly every bad example is riddled with em dashes. "Not a threat — just a fact." "Not destroyed — dissolved." "The door is open — you are the only lock." This is the laziest possible way to connect two ideas. It avoids the work of writing a real sentence. A period forces you to commit to what you're saying. An em dash lets you smush two half-thoughts together and pretend that's writing. Never use them. Period.

### 7. Wrong format for the medium
Game text is UI text. It shares space with buttons, stats, navigation elements. It is read quickly, often repeatedly. The bad versions are written like short fiction with narrative arcs and dramatic reveals. Players don't read event pop-ups like novels. They glance at them. The text needs to land in a glance.

---

## Applying This to Pandemic CLI

This game is a sci-fi pandemic defense simulator. The tone draws from real epidemiology and public health but set in a near-future world. Here's what that means for specific text types:

### Crisis Events
Crisis events pause the game and present two options. The description text should feel like a situation report — factual, urgent, but not melodramatic. Think "WHO situation update" not "disaster movie voiceover."

**BAD crisis text:**
The nightmare scenario has arrived. Deep within the overflowing hospitals of São Paulo, something is changing. The virus is learning. Adapting. Evolving beyond your carefully designed countermeasures with a speed that defies everything we thought we knew about viral mutation. Your best scientists are scrambling, but the truth is undeniable — nature has outpaced us once again.

**GOOD crisis text:**
Field teams in São Paulo report treatment-resistant cases. Genomic sequencing confirms a new strain — your current antivirals may no longer be effective.

### Power Framing

Pandemic CLI text fails immediately if it assumes the wrong kind of world.

- The N.W.H.O. is not a sovereign government. It issues directives, allocates budget, negotiates, pressures, and coordinates.
- Governors are not subordinate administrators. They are independent power holders who may comply, stall, bargain, or defy.
- The board is not advisory. It is the real source of the N.W.H.O.'s leverage and budget.
- Corporations are not flavor. They are the infrastructure and economic power that make regional compliance possible.

This affects wording directly:

- Do not write as if the player can simply order regions around by fiat.
- Do not write as if policy enforcement comes from abstract state capacity.
- Do not write as if humanitarian duty is the institution's true purpose and corporate pressure is a distortion of it.
- Do not write the player as a rebel trapped inside the wrong system. The player works for the system.

Bad framing: "The N.W.H.O. has ordered Governor Sato to comply immediately."
Good framing: "The N.W.H.O. directive has been issued. Governor Sato is stalling and wants compensation before local enforcement begins."

### Disease Names
Diseases already have a naming system (pathogen type + generated name like "Yersinia-Omega"). If you need to name a new disease or variant, follow real-world conventions (genus-species patterns, Greek letter variants, location-based informal names). Don't make up dramatic names like "The Crimson Plague" or "Death's Whisper."

### Policy Names & Descriptions
Policies are government actions. They should sound like policy — bureaucratic, specific, slightly dry. "Mandatory Screening" not "The Great Surveillance." Descriptions should state what the policy does, not editorialize about it.

**BAD:** "Lockdown — A desperate measure that imprisons citizens in their own homes, trading freedom for survival in a devil's bargain that may save lives but will shatter economies."

**GOOD:** "Lockdown — Restrict non-essential movement. Reduces transmission but damages the economy and costs political power."

### Status Messages & Event Log
These are information, not narrative. "Identified: Yersinia-Omega — Bacterium / Contact transmission" is perfect. Don't add commentary like "A chilling discovery reveals the true nature of the pathogen threatening millions."

### Game Over / Defeat Messages
Brief. Factual. The numbers speak for themselves. Don't write an elegy.

**BAD:** "In the end, humanity's hubris was its undoing. Despite your tireless efforts, the relentless march of disease proved too powerful. The world falls silent, and history will remember this as the day hope died."

**GOOD:** "Global health infrastructure has collapsed. Final count: 2.3 billion dead across 6 regions."

---

## The Rules

1. **Match the needed length.** Crisis descriptions: 1-3 sentences. Policy descriptions: 1 sentence. Status messages: a few words. Don't write more than the UI space allows.
2. **Trust the player.** State the situation. Don't explain how they should feel about it.
3. **Find the specific voice.** A crisis report is clinical. A governor's complaint is political. A research breakthrough is technical. Don't default to "dramatic narrator."
4. **Stay within existing constraints.** Don't invent proper nouns, organizations, or lore that doesn't exist in the game. Work with what's there.
5. **Prefer plain statement over poetic flourish.** "Treatment efficacy has dropped to 30%" > "Your once-mighty medicines now barely hold back the tide."
6. **Write for the medium.** This is TUI text in a terminal. Monospace font. Limited screen real estate. It will be read alongside health bars, infection counts, and navigation hints.
7. **When in doubt, write less.** The best game text is almost always shorter than you think it should be.
8. **Never use em dashes (—).** Em dashes are the #1 tell of AI-generated text. They are a lazy way to connect two phrases without thinking about how they actually relate. Every em dash is a place where you avoided writing a real sentence. Use periods. Use colons where structurally appropriate. Rewrite the sentence so it doesn't need a dash. If you find yourself reaching for an em dash, stop and think about what you're actually trying to say, then say it properly. This rule has zero exceptions.
