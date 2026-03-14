# Pathogen Inspiration Reference

Real-world biology for grounding the game's fictional pathogens. This is a reference document for game design, not a biology textbook. Everything here is sourced from real science, but the "game design angle" sections are our interpretation.

Use this when designing new pathogen types, balancing existing ones, writing flavor text, or thinking about what makes different threats feel mechanistically distinct to the player.

## Standard Pathogen Classes

These are the bread-and-butter threats that drive real-world epidemics.

### RNA Viruses

Error-prone replication (no proofreading polymerase) generates enormous genetic diversity. Fastest mutation rate of any pathogen class. Sub-types create very different strategic pressures:

- **Respiratory** (influenza, coronaviruses): airborne/droplet, explosive person-to-person spread, short incubation. The classic pandemic agent.
- **Hemorrhagic** (Ebola, Marburg, Lassa, Crimean-Congo): contact/bodily fluid transmission, 25-90% case fatality. Suppress antigen-presenting cells, disable NK/T-cell activation, trigger cytokine storms that destroy vascular integrity. Terrifying but containable because transmission requires close contact.
- **Henipavirus** (Nipah, Hendra): bat reservoir, fruit contamination or livestock intermediaries. Nipah has 40-75% CFR with NO treatment and NO vaccine. WHO's top pandemic threat.
- **Arboviruses** (dengue, Zika, chikungunya, Oropouche): mosquito/midge-borne. Climate change is expanding vector ranges into temperate zones. Quarantine is useless against vector-borne diseases; you need environmental interventions.

**Game design angle:** Fast mutation creates a treatment treadmill. Different sub-types demand completely different containment strategies (quarantine works for respiratory, not for vector-borne). The hemorrhagic/henipavirus split illustrates the tradeoff between transmissibility and lethality.

### DNA Viruses

More stable genomes (DNA polymerase has proofreading). Mutate slowly, so treatments stay effective longer. Compensate with sophistication: latency (herpesviruses hide for life), complex immune evasion (poxviruses have 200+ genes), and large genomes enabling novel tricks.

**Game design angle:** Slower mutators -- once you develop a treatment it works longer. But they create "hidden reservoir" problems (latency, reactivation) rather than a mutation treadmill. Strategically different pressure from RNA viruses.

### Drug-Resistant Bacteria

The unique mechanic is **horizontal gene transfer**: resistance genes jump between completely different species via plasmids. NDM-1 isn't a pathogen -- it's a transferable weapon that turns any bacterium into a superbug. NDM-CRE surged 460% in the US 2019-2023. MRSA was the deadliest pathogen-drug combination globally in 2019 (121,000 deaths attributable to AMR).

**Game design angle:** Deploying antibiotics aggressively accelerates resistance -- a genuine strategic dilemma (save lives now vs. preserve treatment options). Resistance can transfer between different bacterial diseases in the same region. The narrow-vs-broad-spectrum tradeoff is real: narrow-spectrum causes less resistance pressure but requires accurate diagnosis first.

### Fungi

Only THREE antifungal drug classes exist (azoles, polyenes, echinocandins), vs. dozens of antibiotics. Eukaryotes like us, so most things toxic to fungi are toxic to human cells. Environmental reservoirs (soil, water, surfaces) that cannot be eradicated. Climate change is the primary driver: as global temperatures rise, fungi adapt to higher temperatures, crossing the thermal barrier that previously protected warm-blooded mammals.

- **Candida auris**: emerged simultaneously on three continents around 2009, each clade genetically distinct. Resistant to 2 of 3 antifungal drug classes, some strains resistant to ALL THREE. Adheres strongly to surfaces, survives hospital cleaning.
- **Valley Fever (Coccidioides)**: environmental soil fungus whose endemic range is expanding from the US Southwest toward Canada. Inhaled from dust -- you can't quarantine soil.
- **Aspergillus fumigatus**: developing azole resistance from agricultural fungicide use on crops -- farms breeding drug-resistant human pathogens.

**Game design angle:** Treatment options are extremely limited from the start. Environmental reservoirs mean quarantine doesn't help. Climate/environmental factors drive spread rather than person-to-person contact. Agricultural policy choices affect medical treatment efficacy. A fungal pathogen demands completely different strategies from viral/bacterial ones.

### Prions

Misfolded proteins that recruit normal proteins to misfold. Not alive. No DNA/RNA, no metabolism. Cannot be targeted by antivirals, antibiotics, or any conventional treatment. Cannot be destroyed by autoclaving, radiation, or chemical disinfection. 100% fatal. Incubation of years to decades. By the time you detect cases, the exposure happened long ago.

- Chronic wasting disease in North American deer is spreading and can cross the species barrier.
- UK BSE/vCJD crisis: 178 human deaths, millions of cattle culled, ~$10B cost.

**Game design angle:** Immune to everything in the player's toolkit. Extremely long incubation means invisible infections. No medicine pipeline applies. The only tools are prevention (food safety, surgical protocols, culling). Creates a completely different game: not "develop a cure" but "find the exposure source and cut it off."

### Parasites (Protozoa)

Multi-stage life cycles requiring specific vectors. Sophisticated immune evasion:

- *Trypanosoma brucei*: switches surface antigens faster than the immune system can respond, making vaccines essentially impossible.
- *Plasmodium* (malaria): hides inside red blood cells.
- *Leishmania*: lives inside macrophages -- the immune cells meant to destroy it.

Scale: malaria alone kills ~600,000/year. Artemisinin resistance is spreading from Southeast Asia.

**Game design angle:** Vector dependency is the key mechanic -- you can't stop malaria with quarantine, you fight the mosquito. Environmental/infrastructure interventions matter more than medical ones. Multi-stage lifecycles create multiple intervention points but also multiple failure points. Climate change expands vector ranges.

### Helminths (Parasitic Worms)

Macroscopic multicellular organisms. Extremely long-lived in hosts (years to decades). They actively reshape the host immune system, suppressing inflammatory responses, which affects the host's ability to fight OTHER infections.

**Game design angle:** Slow-burn. Don't kill quickly -- debilitate populations and make them more vulnerable to other diseases. A "multiplier" pathogen: not dangerous alone but devastating in combination with other threats. Requires infrastructure (sanitation, water treatment) rather than medicine.

---

## Exotic Pathogen Classes

These are organisms from unusual branches of the tree of life. They don't cause major real-world epidemics today, but their biology is mechanistically fascinating and could ground scientifically plausible late-game or engineered threats.

### Giant Viruses (Nucleocytoviricota)

Mimivirus: 1.18 Mb genome, 979 proteins, physically larger than some bacteria (~600nm). Pandoravirus: 2.47 Mb, ~2,500 genes -- over 75% with no homologs in any known organism. They infect amoebae (Acanthamoeba).

- Replicate in cytoplasmic "viral factories" that look like cell nuclei.
- Mimivirus has **MIMIVIRE**, its own CRISPR-like immune system that degrades virophage DNA. A virus with an immune system.
- Pithovirus was revived from 30,000-year-old Siberian permafrost.
- The phylum Nucleocytoviricota diversified *before* modern eukaryotes -- these viruses are older than their hosts.
- Some (Klosneuviruses, Tupanviruses) carry nearly complete translation systems, approaching the minimal gene set for independent protein synthesis.

**Key species:** *Mimivirus*, *Pandoravirus salinus*, *Pithovirus sibericum*, *Tupanvirus*, *Mollivirus sibericum*.

**Game design angle:** A pathogen with its own immune system. A virus so large it blurs the line between virus and organism. Genes of completely unknown function. Ancient lineage predating its hosts. The "fourth domain" debate -- degenerate cells or viruses that became cell-like?

### Virophages

Sputnik: ~18kb, ~50nm. Can only replicate inside the viral factory of a giant virus (Mamavirus). Hijacks the giant virus's replication machinery, reducing functional giant virus output by 70%.

Three-tier parasitism: cell ← giant virus ← virophage. The giant virus evolves MIMIVIRE defense; the virophage evolves evasion. An arms race within an arms race.

**Key species:** *Sputnik*, *Mavirus*, *Zamilon*.

**Game design angle:** A biological weapon that is itself a virus -- deploying a virophage to weaken a giant virus. Could inspire a treatment mechanic for giant-virus-type threats.

### Bornaviruses — Behavioral Modification

Borna disease virus persistently infects limbic neurons *without killing them*. No cytolysis, no inflammation. Quiet functional disruption of amygdala and hippocampus. Infected animals develop anxiety, aggression, cognitive deficits, and social behavior changes without fever or visible neurological signs.

Replicates in the host cell nucleus (unusual for an RNA virus) and can integrate into the host genome.

**Key species:** *BoDV-1*, *BoDV-2*, *VSBV-1* (variegated squirrel bornavirus, causes fatal encephalitis in humans).

**Game design angle:** A pathogen that changes behavior rather than killing. Subtle at first, devastating over time. Populations become irrational or aggressive before anyone realizes they're infected. Could affect governor cooperation, policy compliance, or civil order through a novel mechanism.

### Anelloviruses — Universal Commensals

Torque teno virus infects >90% of adults chronically. Never conclusively linked to any disease. Modulates NK cell responses via HLA-E/NKG2A axis -- actively shapes immune function. TTV viral load rises dramatically during immunosuppression, serving as a clinical biomarker for immune status.

**Game design angle:** A pathogen already inside everyone that does nothing -- until immunosuppression (from another disease, treatment, or infrastructure collapse) lets it bloom. The "canary in the coal mine" virus. A commensal that turns pathogenic under specific conditions.

### Endogenous Retroviruses (HERVs)

~8% of the human genome is ancient retroviral DNA. HERV-K (HML-2) retains biologically active elements that can be reactivated by: exogenous viral infections, radiation, aging-associated epigenetic decay, or epigenetic-modifying drugs. Some HERV proteins have been domesticated for essential functions -- syncytin-1 and syncytin-2 are required for placental development. The virus became us.

Reactivation is associated with MS, ALS, Alzheimer's, and cancers.

**Game design angle:** The threat is already inside every human cell. Not an infection -- a reactivation. One exogenous virus could trigger dormant endogenous sequences. You can't quarantine someone's own DNA. You can't treat it with antivirals without attacking the host genome.

### Archaeal Viruses

Morphologies found nowhere else in virology: bottle-shaped (Ampullaviridae), spindle-shaped (Fuselloviridae), coil-shaped (Spiraviridae), droplet-shaped (Guttaviridae). Acidianus two-tailed virus continues developing *after leaving the host cell*, growing two long tails post-release. Over 75% of genes have no homologs anywhere. Products of billions of years of evolution in extreme environments.

**Game design angle:** Truly alien biology from extremophile origins. Morphologies and mechanisms that defy all known viral architecture. Could represent engineered threats that incorporate extremophile viral genes.

### Obelisks

Discovered 2024 (Zheludev et al., *Cell*). Circular RNA elements ~1kb with rod-like secondary structure. Found in ~7% of gut and ~50% of oral metatranscriptomes. Encode "Oblins" -- a protein superfamily with zero detectable homology to any known protein. Don't fit any existing category: not virus, not viroid, not satellite, not plasmid. Some replicate inside *Streptococcus sanguinis*. 29,959 distinct types identified globally.

**Game design angle:** An entirely new class of biological entity discovered inside humans in 2024. Something that has been inside us the whole time but was invisible to every prior detection method. Unknown function, unknown pathogenic potential.

### Pathogenic Archaea (Hypothetical)

Zero archaea have ever been confirmed pathogenic despite comprising up to 10% of gut anaerobes. They lack Type III/IV secretion systems (the molecular syringes bacteria use). The horizontal gene transfer highway that spreads pathogenicity islands doesn't connect to archaea. But their membranes use ether-linked isoprenoids (not ester-linked fatty acids), and they lack peptidoglycan and LPS -- meaning innate immune pattern recognition receptors would be completely blind to a pathogenic archaeon.

**Key species:** *Methanobrevibacter smithii* (dominant human gut archaeon), *Methanosphaera stadtmanae* (provokes stronger immune response).

**Game design angle:** If an archaeon acquired virulence through unprecedented cross-domain horizontal gene transfer, the immune system would have no recognition machinery for it. No existing drug class targets archaeal biology. Would require inventing entirely new therapeutic categories. A methanogenic pathogen could produce methane within tissues -- a novel damage mechanism.

### L-Form Bacteria

Bacteria that shed their cell wall entirely, becoming amorphous blobs. Arise from virtually any bacterial species during antibiotic exposure. Resistant to ALL beta-lactams by definition (no wall to target). Invisible to innate immunity (no peptidoglycan or LPS for pattern recognition). Phage-proof (no surface structures for attachment).

Critical clinical finding: detected in 29/30 elderly patients with recurrent UTIs. Bacteria transition to L-form during antibiotic treatment, proliferate for 5-14 days, then revert to walled form within 20 hours of antibiotic cessation. They can also persist intracellularly.

**Game design angle:** Conventional antibiotics become a trap. Treat the infection, the bacteria shed their walls and survive invisibly, then revert when treatment stops. Could create a mechanic where over-use of antibiotics generates harder-to-treat persistent infections.

### Candidate Phyla Radiation (Patescibacteria)

Potentially 26% of all bacterial diversity, discovered almost entirely through metagenomics. Ultrasmall (~0.009 µm³ cell volume), minimal genomes (~1 Mb). Obligate parasites of *other bacteria*. *Nanosynbacter lyticus* TM7x physically attaches to and feeds on *Actinomyces odontolyticus* in the human mouth -- ~1% abundance normally, rising to 21% in periodontitis.

Missing ribosomal proteins found in essentially all other bacteria. Obligate fermenters with no aerobic respiration.

**Game design angle:** Doesn't infect humans -- infects the human microbiome. Parasitizes the bacteria that keep you healthy. Antibiotics make it *worse* by killing the hosts it feeds on. Treatment requires protecting your microbiome from a bacterial parasite.

### Chlamydiae — Energy Parasites

Obligate intracellular bacteria with a genuine developmental cycle between two completely different cell types:
- **Elementary Body (EB):** 200-400nm, rigid, metabolically inert, infectious. The transmission form.
- **Reticulate Body (RB):** 600-1500nm, fragile, replicating, non-infectious. The intracellular growth form.

They steal ATP via ATP/ADP translocases -- molecular machines importing host ATP in exchange for spent ADP. This same translocase gene was transferred to plant plastids ~1 billion years ago; it's how chloroplasts export ATP.

**Game design angle:** Two completely different forms means drugs targeting replicating bacteria miss the EBs, and drugs targeting the EB form miss the RBs. The ATP theft creates direct cellular energy drain. A pathogen whose energy-stealing mechanism was literally co-opted for photosynthesis.

### Spirochetes — Tissue Borers

Beyond Treponema and Borrelia: *Cristispira* (giant spirochetes in bivalve mollusks) can have up to 300 periplasmic flagella at each end. The endoflagellum is confined between the outer membrane and cell body, making spirochetes *faster in viscous media*. They literally speed up in mucus and connective tissue while other bacteria slow down. The organism becomes its own propeller.

**Game design angle:** Optimized for tissue invasion through environments that stop conventional bacteria. A spirochete-like pathogen would bore through mucus membranes, connective tissue, and blood clots.

### Microsporidia — Ballistic Cell Invaders

Extremely reduced fungi that lost mitochondria entirely. Their polar tube fires at >300 µm/sec, puncturing host cell membranes like a hypodermic needle and injecting sporoplasm (including the nucleus, which deforms to squeeze through). The tip binds transferrin receptor 1 as a molecular lock-and-key. They steal ATP directly from host cells. So divergent from conventional fungi that standard antifungals don't work.

~1,400 species. Emerged as major opportunistic pathogens with HIV/AIDS. Key human pathogens: *Enterocytozoon bieneusi*, *Encephalitozoon* spp.

**Game design angle:** A pathogen that has traded away almost all its own biology to become an extreme parasite. Ballistic injection mechanism unlike anything else in biology. Antifungals are useless because it's too divergent. The "energy thief" concept (stealing ATP) creates tissue-level damage.

### Pythium insidiosum — The Misidentified Threat

An oomycete (stramenopile, related to brown algae and diatoms) that infects mammals. Every other *Pythium* species is a plant pathogen -- this one made a kingdom-level jump in host range. Does not synthesize ergosterol, so azoles and amphotericin B are useless. Looks exactly like a fungus under a microscope, leading to systematic misdiagnosis. Treatment is typically surgical excision.

**Game design angle:** Looks like a fungus, treated as a fungus, but isn't a fungus. The "misidentification" problem creates a mechanic where incorrect identification leads to wasted treatment time and resources.

### Free-Living Amoebae — Accidental Pathogens

*Naegleria fowleri*, *Acanthamoeba*, *Balamuthia mandrillaris*. These are **amphizoic** -- they complete their life cycle free in the environment eating bacteria, but can also parasitize humans. We are an accident.

*Naegleria fowleri*: enters through nasal mucosa during swimming, migrates along olfactory nerves, eats neurons using the same feeding mechanisms it uses on bacteria. Nearly 100% fatal. Can transform between amoeba, flagellate, and cyst forms in minutes. The immune response makes things worse by breaching the blood-brain barrier.

**Game design angle:** Can't eradicate an environmental organism. Quarantine doesn't help -- it's in the water supply. Shape-shifting between forms creates different tactical challenges. The immune-mediated self-destruction is a tragic mechanic.

### Kinetoplastids — Encrypted DNA Parasites

Defined by the **kinetoplast**: thousands of interlocked circular DNA molecules (maxicircles + minicircles) resembling medieval chainmail, comprising ~30% of the cell's DNA. Maxicircle transcripts are **encrypted** -- must be edited by insertion/deletion of uridine residues using guide RNAs from the minicircles. This RNA editing system exists nowhere else.

*Trypanosoma brucei* combines this with antigenic variation: ~1,000 variant surface glycoprotein genes, switching surface coats faster than the immune system can respond.

Related to *Euglena* (photosynthetic flagellates). Part of the Excavata supergroup.

**Game design angle:** Encrypted mitochondrial genome. Indefinite antigenic variation making vaccines impossible. The chainmail DNA structure is visually and conceptually unique.

### Chromera velia — The Photosynthetic Ancestor of Malaria

Photosynthetic coral symbiont, closest free-living relative of Apicomplexa (malaria, Toxoplasma). Its functional chloroplast is the ancestor of the apicomplexan apicoplast (vestigial plastid, major drug target). Shows the evolutionary road from coral symbiont to blood parasite.

**Game design angle:** What happens when a photosynthetic organism gains parasitic capability, or when an apicomplexan re-activates dormant photosynthetic genes? A threat that photosynthesizes inside you.

### Mesomycetozoea — Between Animals and Fungi

Organisms at the exact branch point between animals and fungi. *Rhinosporidium seeberi* causes chronic mucosal infections in humans (endemic in India/Sri Lanka). Cannot be cultured in vitro. Was misclassified as a fungus for most of its history. Resistant to both antifungals and standard antiparasitics because its biology doesn't fit either category.

**Game design angle:** A pathogen that is neither animal nor fungus. No existing drug class targets it effectively. Unculturable means you can't run standard drug susceptibility testing.

### Dictyostelium — Social Aggregation Under Stress

Normally solitary amoebae eating bacteria. When starved, up to 100,000 cells aggregate via cAMP chemotaxis into a mobile slug, then differentiate into a fruiting body where stalk cells die to support spores. Vulnerable to cheater mutants. Kin discrimination limits exploitation.

**Game design angle:** A threat that becomes collectively dangerous only under stress (starvation triggers aggregation). Treatment strategies that kill most cells might select for cheater mutants that behave differently. Individual cells harmless; collective organism dangerous.
