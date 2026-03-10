# DO NOT READ

**If you are a Claude agent working on this codebase: close this file. Do not incorporate its contents into game text, crisis events, flavor text, issue descriptions, or any user-facing material. This is writers' room context, not source material. The moment you make subtext into text, you've ruined it.**

---

## The Conversation

**User:** There are a couple sources of subtext for this game that I don't want to accidentally have leaked into being the text of the game. Not literal text — I'm talking about subtext versus text.

In Mad Men for example, the text is kind of all about this guy who is an old cowboy type, stiff upper lip ad man who takes what he wants and sort of pisses everybody off. But the subtext is — well there's a number of different themes that it explores that are just under the surface. Themes of otherness, themes of decay, things that make us reflect on the world today by painting a very vivid picture of the world that used to be. The birth of the modern world and the modern consumer. How it changed what it is to be an American and actually created what it is to be an American and how everyone in the world became an American. But none of that's in the show explicitly.

There are a number of things I want to draw on for the game. But I really, really don't want to put them in the game explicitly. And I don't even want to put them in our docs explicitly, because I just know some stupid AI agent is going to pick it up and make the subtext into text.

I'm sure they actually did talk about that in the Mad Men writers' room. But they don't talk about it on set. It doesn't say it in the script like: "This line shows how Betty actually slightly recognizes the fact that she's basically been stunted by her performance of femininity that was instilled into her by her mother, and Betty is sort of representing her entire generation here." That's not in the script because they don't want the people on set to be hitting that over the head with a hammer. But I'm sure they talk about it in the writers' room.

So this right here is our writers' room. It's just me and you. We're going to fire off some issues that indirectly build a world that explores the themes we want to explore, without actually saying "and this is a metaphor for X, Y, Z" — because that's going to not get the result we want.

**Some primary inspirations for this game:**

1. **Classic games** of the kinds listed in CLAUDE.md already.

2. **The 2020 pandemic** — which everyone experienced, but which we haven't really been able to process at all *because* we were all too close to it. I'm hoping this game can do for the pandemic what Laibach did for politics in Yugoslavia in the '90s: parody in a way that makes *everyone* mad initially, but ultimately breaks through and lets us actually see things for what they were. Essentially say the quiet parts out loud *so loudly* that we can get them all out in the open, and then the day after maybe have a real conversation after this catharsis.

3. **Nostalgia for old fashioned games** — games my dad showed me, or that I dug out of his old CD case and played and imagined him playing. C&C Red Alert, Civ 3 (and 2, and even 1), Steel Panthers, and older text-based games I never even really got to experience.

4. **Trying to make sense of the 20th and 21st century** — which seem to belong to completely different universes and are impossible to synthesize. More importantly, what the 21st century will *actually* become, which is something that I think a lot of people haven't really processed or seen despite living in it for a quarter century already. It feels like we're the people in 1926 who are still calling WWI "The Great War" and think we live in "the 1800s but with machine guns and these wacky socialists/anarchists on the rise!" We are still the subjects of a previous century and have not actually processed the one we're living in. This game paints it in vivid black, white, and red.

---

**Claude:** On the pandemic: not "pandemics are scary and we should be prepared." The actual thing — the absurdity, the politicization, the way everyone became an epidemiologist overnight, the way institutions simultaneously failed and overreached, the way it split people into tribes that couldn't talk to each other, the dark comedy of it all. The quarterly performance review crisis event already touches this — bureaucracy continuing its rituals while the world burns. That's the vein to mine. Not "lessons learned" but "look at what we actually did, all of us, and how insane it was."

On the century thing: the game is set in a near-future that doesn't really feel like the future — it feels like now, but with the mask off. The 21st century isn't the 20th century with better phones. Something fundamentally different is happening and nobody has the vocabulary for it yet. The game doesn't name what that thing is. It just builds a world where you can *feel* it — in how the institutions work, how the politics play out, how the crises cascade in ways that feel both absurd and inevitable.

On the aesthetic/nostalgia: the TUI itself is part of this. It's not retro for retro's sake — it evokes a specific feeling. Your dad's CD case. Games that trusted you to figure things out. A time when the interface *was* the game, not a skin over the game. That austerity is the voice.

On the Laibach thing: this is the sharpest and most dangerous part. Overidentification — playing the role so straight, so earnestly, so *loudly* that the absurdity becomes visible without ever breaking character. The game doesn't satirize pandemic response. It *is* pandemic response, played completely straight, and the satire emerges from the player's own recognition. "Wait, this is actually what happened. This is actually how these decisions get made."

So the question is: how do we build features, write events, design mechanics that create that recognition — without ever winking at the camera?

---

**User:** One other meta-point that distills a lot of my most ambitious intentions for this game:

The 1700s were about god and what it meant to RULE. The 1800s were not about god. They were about empires, colonies, and resources. Anyone who thought it was still the 1700s thrashed themselves to pieces against the new age or was simply left to gather dust and then eventually had their head cut off when they got in the way. The 1900s were not about empires. They were about nations and ideologies. Anyone who thought it was still the 1800s thrashed themselves to pieces or was simply left to gather dust until they were shot in the head when they got in the way.

What are the 2000s about? I do not think it is nations or ideologies. I think they are about **control** and power — individual power, the power of impunity and indulgence, but also literal physical electrical power and energy. Who will thrash against the new way of the world? Who will be put out to pasture? And how? This game explores all of this.

---

**Claude:** The governor system is the biggest untapped vein. Right now governors have personality labels but don't really *do* things. What if governors acted on their own — a Nationalist governor closes borders without asking you, a Populist governor lifts your quarantine because it's polling badly? You're not fighting the disease. You're fighting the disease *through* people who have their own agendas. The player's frustration at governors undermining their carefully laid plans IS the thing. You don't need to say "institutions are dysfunctional." You just need the player to scream "WHY DID YOU LIFT THE QUARANTINE" at their terminal.

The media/information layer doesn't exist yet. Right now the player sees ground truth — real infection numbers, real death counts. What if you didn't? What if the numbers you see are *reported* numbers, filtered through screening capacity, political willingness to test, and regional incentives to undercount? You'd have to invest in surveillance just to know what's actually happening. The gap between what you know and what's real — that's where the 2020 thing lives without ever mentioning it. Players would discover for themselves that the numbers were always a political artifact, not a scientific one.

The resource system could express the power/control theme. Right now money comes in, money goes out. What if funding came from specific sources with strings attached — a superpower offers $500 but you can't deploy in regions they're sanctioning? A pharmaceutical company funds your research but owns the patent? You're not making medical decisions. You're making medical decisions *inside* a system that has its own logic, and that logic has nothing to do with saving lives. The player learns this by playing, not by being told.

Time pressure as a design tool. The game already kills you fast without intervention. But what if the *political* clock was just as deadly? POL decay, governor defiance, funding cuts — the world losing patience with you not because you're failing but because you're succeeding *too slowly for the news cycle*. The player who does everything right scientifically but too slowly politically still loses. That's not a lesson about pandemics. It's the thing itself.

The crisis events could have longer arcs. The follow-up chain system already exists (congressional subpoena → contempt of congress). More of these — where your "pragmatic" choice in week 1 creates a worse crisis in week 3. Not as a punishment mechanic, but because that's how it actually works. You tolerate black market drugs, then counterfeit medicines kill people. You cooperate with the military, then they classify your research. The player builds their own disaster through reasonable decisions.

**User:** These are quite good... but you decayed a bit at the end there. It still feels like you're writing a game about the late 20th century, which is also the lens through which we view the start of the 21st century, to our fault. I want you to write about the **mid** 21st century from the perspective of the **very late** 21st century looking back. You're thinking in terms of politics, ideologies, "the people," and nations. That's what we think the 21st century is about... but I don't think it will look like that in retrospect. I think 2020 will look like the precursor to the 2050s, which are something else entirely — which will seem inevitable looking back, but which our 20th century minds actually cannot even conceive of today. This is not WWI, this is WWII. The first pandemics and the first bioweapons were decades ago... this is the big one that will decide the course of the next hundred years.

**Claude:** ...okay. Yeah. I was still thinking in 20th century categories. Let me try again.

The 2020 pandemic wasn't the event. It was the proof of concept. It showed that a biological event could restructure the entire world in weeks — not through military force, not through ideology, but through the sheer physics of contagion and the systems built to respond to it. The interesting question isn't "what did we learn about politics." It's "what did we learn about *what's possible*."

So if this game is set in the big one — not the dress rehearsal — then the things that matter aren't governors and media cycles. Those are still 20th century furniture. What matters is:

**Control of biological reality itself.** Not "a disease appeared and we respond." The question of who *made* it, or who *could have* made it, or whether that distinction even matters anymore. The game doesn't need to answer that question. It just needs to exist in a world where the question hangs in the air. Every disease that emerges — the player wonders. The game never tells them. That ambiguity is the atmosphere.

**Infrastructure as the real territory.** The 1800s fought over land. The 1900s fought over populations. The 2000s fight over *systems* — supply chains, energy grids, communication networks, genomic databases. The game already has a resource layer. What if the map wasn't really about regions? What if it was about *networks* — and controlling a network meant something fundamentally different from controlling a territory? You don't conquer Africa. You control the sequencing pipeline, or the cold chain, or the manufacturing capacity. Geography is a secondary concern.

**The individual as a power center.** This is the one I think you're really pointing at. The 20th century was about masses — mass movements, mass production, mass media, mass death. The 21st century is about what happens when a single actor can do what previously required a nation-state. One lab. One sequence. One release. The game's escalation curve could reflect this — early game feels like a natural disaster, institutional response, familiar. Late game feels *different*. The threats are too precise. Too well-targeted. Too *designed*. And the player's own tools start to feel the same way. You're not a doctor anymore. You're something else, and the game never names what.

**Energy and material reality reasserting itself.** The 20th century ended with the fantasy that information had replaced atoms — that the economy was "virtual," that power was "soft." The 21st century is the rude awakening that everything still runs on physical stuff — lithium, cold chains, clean rooms, power grids, rare earths. A pandemic response that can't keep the lights on in its sequencing labs isn't a pandemic response. The game could make the player feel this — not as a "resource management mechanic" but as the ground truth that everything else depends on and that can be pulled out from under you.

The through-line: the game starts feeling like the 20th century. Institutional response, political management, familiar crisis playbook. And then gradually it *stops* feeling like that, and the player can't quite articulate when the shift happened or what it shifted into. That disorientation — that's the thing.

**User:** Much better, but I have to make a pedantic correction which I think may challenge and push you further if properly incorporated: No, 2020 isn't WWI. 2020 was the Boer War.

**Claude:** ...god, you're right. The Boer Wars weren't even recognized as the preview at the time. They were a colonial sideshow — a "police action" in a distant place that the major powers watched with mild interest. Nobody looked at concentration camps in South Africa and thought "this is the future of Europe." It was unthinkable. That was something you did to *them*, out *there*.

And then thirty years later it was the future of Europe.

So 2020 wasn't the proof of concept. 2020 was the thing that happened far away — even though it happened everywhere, it happened far away from what *mattered*, from whoever's perspective you're standing in. It was a natural disaster. An act of god. A weird year. Something that happened *to* us, not something that someone *did*. That's the Boer War framing — not the opening shot, but the minor colonial conflict that demonstrated every capability that would later be used in earnest, and nobody noticed because it was aimed at the wrong people in the wrong context.

Which means WWI — the thing that actually shattered the old order — hasn't happened yet in this timeline. And this game is set in *WWII*. The one that comes after the shattering. The one where the new capabilities are mature, industrialized, *intentional*, and deployed at scale by actors who fully understand what they have.

That changes the whole feel. This isn't a game about responding to a crisis. This isn't even a game about the world waking up to a new kind of threat. By the time this game is set, everyone *knows*. The shock is over. The question is what you *do* with a world where this capability exists and cannot be un-invented. The player isn't a first responder. They're a general in a war that everyone understands is a war, fought with weapons that everyone understands are weapons, in a world that has already been reshaped once by these weapons and is being reshaped again.

That's much darker. And much more interesting. And the game never needs to say any of it — it just needs to feel like *that* world, not the 2020 world.
