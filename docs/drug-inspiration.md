# Drug & Treatment Inspiration Reference

Real-world therapeutic and vaccine classes for grounding the game's treatment systems. This is a reference document for game design -- sourced from real science, with game design interpretation.

Use this when designing treatment mechanics, balancing therapy types, writing flavor text, or thinking about what makes different treatment approaches feel strategically distinct.

## Antiviral Drug Classes

Each class works on a genuinely different mechanism with different tradeoffs.

### Nucleoside/Nucleotide Analogs (Polymerase Inhibitors)

**Mechanism:** Fake building blocks that get incorporated into viral DNA/RNA by the viral polymerase, causing chain termination or lethal mutagenesis.

**Tradeoffs:** Cheap to manufacture. Moderate-high resistance risk (single point mutations in the polymerase can confer resistance). Can affect host cell DNA polymerases (especially mitochondrial), causing toxicity. Good in cocktails, poor as monotherapy.

**Examples:** Acyclovir (herpes), Remdesivir (SARS-CoV-2), Sofosbuvir (HCV, "cured" hepatitis C), Molnupiravir (lethal mutagenesis approach), Tenofovir (HIV/HBV).

### Protease Inhibitors

**Mechanism:** Block the viral protease that cleaves polyprotein precursors into functional viral proteins. Without cleavage, the virus produces non-functional proteins. Designed to mimic the protease-substrate transition state.

**Tradeoffs:** Slow initial development (structure-based drug design). Moderate resistance risk. Metabolic side effects. Excellent in combinations (backbone of HIV HAART). Moderate to high cost.

**Examples:** Nirmatrelvir/Paxlovid (SARS-CoV-2), Ritonavir/Lopinavir (HIV).

### Neuraminidase Inhibitors

**Mechanism:** Block the neuraminidase on influenza virus surfaces. Neuraminidase normally releases newly assembled virions from the cell -- blocking it traps new viruses on the cell surface.

**Tradeoffs:** Must be given within 48 hours of symptom onset. Narrow therapeutic window. Influenza-specific. Low-moderate resistance. Generally mild side effects.

**Examples:** Oseltamivir (Tamiflu), Zanamivir (Relenza).

### Entry/Fusion Inhibitors

**Mechanism:** Block virus attachment to or entry into host cells. Some prevent receptor binding; others prevent membrane fusion after docking. Acts before the virus ever gets inside.

**Tradeoffs:** Slow to develop (requires detailed structural knowledge). Moderate resistance risk (viral surface proteins mutate rapidly). No intracellular delivery needed.

**Examples:** Maraviroc (HIV CCR5 antagonist), Enfuvirtide (HIV fusion inhibitor), Bulevirtide (HBV entry inhibitor).

### Integrase Inhibitors

**Mechanism:** Block the integrase enzyme retroviruses use to insert their DNA into the host genome. Prevents viral DNA from being stitched into host chromosomes.

**Tradeoffs:** Retrovirus-specific. Newest generation (dolutegravir, bictegravir) has very high resistance barrier. Excellent tolerability.

**Examples:** Dolutegravir, Bictegravir, Raltegravir.

### Capsid Inhibitors

**Mechanism:** Bind to viral capsid proteins, either stabilizing the capsid so it can't disassemble (preventing genome release) or destabilizing it prematurely.

**Tradeoffs:** Novel mechanism, effective against strains resistant to other classes. Variable resistance risk -- lenacapavir (HIV) has high barrier; amantadine (influenza M2 blocker) has near-universal resistance.

**Examples:** Lenacapavir (HIV, given every 6 months -- long-acting), Amantadine (influenza, now largely obsolete).

### Interferons (Host-Directed Antivirals)

**Mechanism:** Don't attack the virus. Bind cell surface receptors and activate JAK-STAT pathway, inducing hundreds of "interferon-stimulated genes" that create an antiviral state: degrading viral RNA, blocking translation, inhibiting assembly. Also activate NK cells and macrophages.

**Tradeoffs:** Low resistance risk (hard to mutate around a host immune response). Severe side effects (flu-like symptoms, depression, cytopenias). Expensive (biologics). Broad-spectrum.

**Examples:** Pegylated interferon alfa-2a/2b (HBV, formerly HCV).

**Game design angle across all antiviral classes:** Each class targets a different step of the viral lifecycle: entry → uncoating → replication → protein processing → assembly → release. A game system could model these as different intervention points with different speed/resistance/side-effect tradeoffs. Cocktails (combining classes) are dramatically more effective than monotherapy because the virus must simultaneously mutate at multiple sites to escape.

---

## Antibiotic Classes

### Beta-Lactams (Cell Wall Synthesis)

**Mechanism:** Bind penicillin-binding proteins, prevent peptidoglycan cross-linking. Weakened wall can't resist osmotic pressure; cell lyses. Bactericidal.

**Sub-types:** Penicillins → cephalosporins (5 generations) → carbapenems → monobactams. Each step broader spectrum.

**Tradeoffs:** Cheapest antibiotics. Most used. Most resisted. Beta-lactamase enzymes (ESBLs, carbapenemases) hydrolyze the beta-lactam ring. Countered with beta-lactamase inhibitors (clavulanic acid, tazobactam). Allergic reactions are the main side effect concern.

**Examples:** Amoxicillin, Ceftriaxone, Meropenem, Piperacillin-tazobactam.

### Aminoglycosides (Ribosome - 30S)

**Mechanism:** Bind irreversibly to 30S ribosomal subunit, causing mRNA misreading. Bacterium produces nonfunctional proteins. Bactericidal. Require oxygen for uptake (useless against anaerobes).

**Tradeoffs:** Irreversible hearing loss (ototoxicity) and kidney damage (nephrotoxicity). Synergistic with beta-lactams (cell wall disruption enhances uptake). Cheap. Requires therapeutic drug monitoring.

**Examples:** Gentamicin, Tobramycin, Amikacin, Streptomycin.

### Fluoroquinolones (DNA Gyrase)

**Mechanism:** Inhibit DNA gyrase and topoisomerase IV, essential for DNA replication. Prevent DNA supercoiling and chromosome segregation. Bactericidal.

**Tradeoffs:** Excellent oral bioavailability, broad spectrum, good tissue penetration. Rising resistance from overuse. FDA black box warning: tendon rupture, peripheral neuropathy, QT prolongation. Cheap (generics).

**Examples:** Ciprofloxacin, Levofloxacin, Moxifloxacin.

### Macrolides (Ribosome - 50S)

**Mechanism:** Bind 50S ribosomal subunit, block peptidyl transferase. Primarily bacteriostatic.

**Tradeoffs:** Good intracellular penetration (effective against atypical pathogens: Mycoplasma, Chlamydia, Legionella). Anti-inflammatory properties. GI upset, QT prolongation. Cheap.

**Examples:** Azithromycin ("Z-pack"), Erythromycin, Clarithromycin.

### Glycopeptides (Cell Wall - Different Site)

**Mechanism:** Bind D-Ala-D-Ala terminus of peptidoglycan precursors, physically blocking enzymes from accessing the substrate. Too large to penetrate gram-negative outer membranes (gram-positive only).

**Tradeoffs:** Last-line for MRSA. VRE modifies the target (D-Ala-D-Lac), reducing binding 1000-fold. Nephrotoxicity. "Red Man Syndrome" from rapid infusion. IV only for systemic use.

**Examples:** Vancomycin, Teicoplanin, Dalbavancin (long-acting, single dose covers 2 weeks).

### Polymyxins (Membrane Disruptors)

**Mechanism:** Disrupt bacterial cell membrane like a detergent. Last-resort for extensively drug-resistant gram-negatives.

**Tradeoffs:** Significant nephrotoxicity. Reserved for pan-resistant infections where nothing else works. The "break glass in case of emergency" antibiotic.

**Examples:** Colistin, Polymyxin B.

### Rifamycins (RNA Polymerase)

**Mechanism:** Block bacterial RNA polymerase. Cornerstone of TB treatment.

**Tradeoffs:** Resistance develops rapidly as monotherapy (always combine). Massive drug interactions (CYP450 inducer -- affects metabolism of many other drugs). Turns bodily fluids orange.

**Examples:** Rifampin, Rifabutin.

### The Narrow vs. Broad Spectrum Tradeoff

This is a real and important strategic tension:

- **Narrow-spectrum:** Less collateral damage to microbiome. Less selection pressure for resistance. But requires accurate pathogen identification before use (takes time -- you can't use what you can't aim).
- **Broad-spectrum:** Can be used empirically before identification. But promotes resistance, can cause secondary infections (C. diff colitis, fungal overgrowth from wiping out competing bacteria).

**Game design angle:** The narrow/broad tradeoff maps directly to a gameplay tension between speed (deploy broadly now) and sustainability (deploy narrowly to preserve long-term effectiveness). Over-use of broad-spectrum drives resistance across all bacterial threats simultaneously.

---

## Antifungal Drug Classes

### Azoles (Ergosterol Synthesis)

**Mechanism:** Block conversion of lanosterol to ergosterol. Without ergosterol, fungal membrane becomes leaky. Primarily fungistatic (stops growth, doesn't kill).

**Tradeoffs:** Oral formulations available (unlike most antifungals). Drug interactions (CYP450 inhibition). Rising resistance, partly from agricultural fungicide use breeding resistant human pathogens.

**Examples:** Fluconazole (cheap), Voriconazole, Posaconazole, Isavuconazole (newer, expensive).

### Polyenes (Membrane Disruptors)

**Mechanism:** Bind directly to ergosterol in fungal membrane, forming pores that leak intracellular contents. Fungicidal. Broadest-spectrum antifungals.

**Tradeoffs:** Very rare resistance (hard to mutate away ergosterol). Severe nephrotoxicity -- amphotericin B is nicknamed "amphoterrible." Lipid formulations reduce toxicity but are very expensive.

**Examples:** Amphotericin B (IV), Nystatin (topical only).

### Echinocandins (Cell Wall - Glucan Synthesis)

**Mechanism:** Block beta-(1,3)-glucan synthase. Human cells have no cell wall, so selectivity is excellent. Fungicidal against Candida, fungistatic against Aspergillus.

**Tradeoffs:** Best safety profile of systemic antifungals. Historically IV-only (oral formulations now emerging). No activity against Mucorales (mucormycosis). Expensive. Emerging resistance via FKS gene mutations.

**Examples:** Caspofungin, Micafungin, Anidulafungin.

**Game design angle:** Only three drug classes for all fungi. Each with significant limitations. Resistance to one class often means there are only two options left; pan-resistant fungi (like some Candida auris strains) have exhausted all three. This scarcity of options is the core tension of antifungal therapy.

---

## Antiparasitic Drug Classes

### Antimalarials

- **Chloroquine:** Prevents heme detoxification in the parasite's food vacuole. Near-universal resistance in P. falciparum now.
- **Artemisinins (ACTs):** Generate free radicals via reaction with iron, causing oxidative damage. Fastest-acting antimalarials. Must combine to prevent resistance. Partial resistance spreading from Southeast Asia.
- **Atovaquone-Proguanil:** Inhibit mitochondrial electron transport and folate synthesis. Prophylaxis.

### Antihelminthics

- **Benzimidazoles** (Albendazole, Mebendazole): Block microtubule polymerization. Worms can't absorb glucose and starve.
- **Ivermectin:** Paralyzes worms via glutamate-gated chloride channels.
- **Praziquantel:** Calcium influx causes spastic paralysis in schistosomes/tapeworms.

### Antiprotozoals

- **Metronidazole/Tinidazole:** Reduced inside anaerobic organisms, generating toxic free radicals. Effective against Giardia, Entamoeba, Trichomonas.
- **Nitazoxanide:** Broad-spectrum antiparasitic with antiviral activity. Inhibits pyruvate:ferredoxin oxidoreductase.

---

## Vaccine Platforms

Each platform has genuinely different speed, durability, safety, and manufacturing tradeoffs.

### mRNA

Synthetic mRNA encoding target antigen. Host cells produce the protein, triggering immune response. mRNA degrades within days.

- **Speed:** Fastest. Moderna designed mRNA-1273 in 2 days. Sequence-to-candidate in weeks.
- **Manufacturing:** Cell-free enzymatic synthesis. Highly scalable. ~7 days production.
- **Cold chain:** Requires -20C to -70C (improving).
- **Durability:** May wane faster. Boosters needed.
- **Adaptability:** Update for new variants by changing the sequence.

### Viral Vector

Harmless virus (adenovirus, MVA, VSV) carries genetic material encoding the antigen into host cells.

- **Speed:** Moderate (need to engineer the vector).
- **Durability:** Strong, long-lasting. Sometimes single-dose.
- **Limitation:** Pre-existing immunity to the vector reduces efficacy. Solved by rare serotypes (ChAdOx1 uses chimpanzee adenovirus).
- **Examples:** Oxford-AstraZeneca, J&J, rVSV-ZEBOV (Ebola).

### Protein Subunit

Purified viral/bacterial proteins with adjuvant.

- **Speed:** Slow (identify, produce, purify protein; optimize adjuvant).
- **Safety:** Excellent. No live components. Suitable for immunocompromised.
- **Examples:** Novavax, Hepatitis B vaccine, HPV vaccine (Gardasil).

### Inactivated (Killed)

Whole pathogen killed chemically or by heat.

- **Speed:** Slow (must culture actual pathogen, then inactivate). Requires BSL-3/4 facilities.
- **Durability:** Weaker response. Multiple doses/boosters needed.
- **Examples:** Sinovac (CoronaVac), Polio (Salk), most injectable flu vaccines.

### Live Attenuated

Weakened live pathogen that replicates but can't cause disease.

- **Speed:** Very slow (attenuation can take years).
- **Durability:** Strongest, most durable immunity. Often lifelong with 1-2 doses.
- **Safety:** Cannot give to immunocompromised. Extremely rare reversion to virulence.
- **Examples:** MMR, oral polio, yellow fever, BCG (tuberculosis).

**Game design angle:** Speed-durability tradeoff. mRNA is fast but wanes; live attenuated is slow but lifelong. Manufacturing complexity varies enormously. The choice between platforms is a real strategic decision about time horizon vs. resource investment.

---

## Monoclonal Antibodies

Laboratory-engineered antibodies targeting specific pathogen epitopes. Provide immediate but temporary passive immunity.

**Tradeoffs:** Immediate protection (unlike vaccines). Very expensive ($1K-$6K/infusion). HIGH resistance risk -- single mutations escape binding (demonstrated dramatically with COVID Omicron variants rendering most mAbs useless). Complex biologics manufacturing. Weeks-to-months duration.

**Game design angle:** The "emergency button" treatment. Fast, expensive, and a single viral mutation can make it worthless overnight.

---

## Bacteriophage Therapy

Viruses that naturally infect and kill specific bacteria. Self-amplifying at the infection site. 77% clinical improvement in a 100-patient study.

**Tradeoffs:** Ultra-narrow spectrum -- must match phage to patient's exact bacterial strain. Bacteria that evolve phage resistance often *lose* their antibiotic resistance (evolutionary tradeoff). No FDA-approved product. Non-standard manufacturing. Phages co-evolve with bacteria.

**Game design angle:** The anti-broad-spectrum. Exquisitely targeted but useless against the wrong strain. The resistance tradeoff (phage resistance → antibiotic sensitivity) creates interesting strategic interplay with conventional antibiotics.

---

## Non-Drug Interventions

### Convalescent Plasma

Plasma from recovered patients containing polyclonal antibodies. Immediate passive immunity. Variable quality. First tool for novel outbreaks before anything else exists. Rapidly outpaced by monoclonal antibodies but doesn't require months of development.

### ECMO (Extracorporeal Membrane Oxygenation)

Mechanical lung bypass. Does not treat infection; buys time for immune system or drugs to work. Extremely expensive, requires specialized centers, limited availability.

**Game design angle:** Pure supportive care. Reduces lethality but doesn't affect the disease. Resource-intensive. The "keep them alive long enough for the real treatment to work" option.

---

## Emerging Therapeutic Technologies

These represent genuinely new strategic axes, not just "better versions" of existing approaches.

### CRISPR Antivirals (Cas13)

Programmable RNA-targeting enzymes that find and destroy viral RNA inside cells. Six guide RNAs could target >90% of all coronaviruses. Retarget to a new virus in days by redesigning the guide RNA.

**Status:** Preclinical for antivirals. EBT-101 (HIV, Cas9-based) failed Phase 1/2a in May 2025. No Cas13 antiviral in human trials yet. Delivery to infected cells in vivo is the critical unsolved problem.

**Game design angle:** Fast to design, slow to deliver. The gap between "we know exactly what RNA to cut" and "we can get the scissors into the right cells" is the core tension.

### Broad-Spectrum Antiviral Platforms

Host-targeting compounds that activate innate antiviral defenses. Pegylated interferon lambda showed 51% reduction in COVID hospitalization (Phase 3, NEJM). Targets the host, not the virus -- resistance is essentially impossible through viral mutation.

**Game design angle:** Pre-developed before you know what's coming. Breadth trades off against potency. The "always ready but never optimal" option.

### Nanobodies (Single-Domain Antibodies)

1/10th the size of conventional antibodies. Access hidden epitopes conventional antibodies physically can't reach. Survive heat, nebulization, oral delivery. Produced in bacteria (cheap vs. mammalian cell culture for mAbs). Can be multimerized -- link 2-3 targeting different epitopes to prevent escape mutations.

**Game design angle:** Cheaper, more robust, harder-to-escape version of monoclonal antibodies. The "next generation" of passive immunotherapy with genuinely different properties.

### Host-Directed Therapy

Modulate the host immune response rather than targeting the pathogen. Boost autophagy, enhance antimicrobial peptide production, reverse pathogen-induced immune suppression. Many candidates are cheap repurposed drugs (metformin, statins). Resistance is theoretically impossible.

**Status:** Multiple candidates in clinical trials for TB. ImmunoSep trial (276 patients, 6 countries) tested precision immunotherapy for sepsis.

**Game design angle:** Pathogen resistance is impossible, but immune modulation is double-edged -- wrong timing (enhancing inflammation during already-hyperactive early sepsis) can be fatal. Requires diagnosing immune state first.

### Antimicrobial Peptides

Short positively charged peptides that disrupt bacterial membranes. Low resistance risk (hard to evolve around membrane disruption). Also immunomodulatory. But degraded by proteases in vivo, and activity drops in physiological conditions. No systemic AMP has passed clinical trials.

**Game design angle:** The "theoretically perfect" antibiotic that doesn't work in practice yet. Low resistance, broad spectrum, but stability and delivery are unsolved.

### Checkpoint Inhibitors for Infection

Cancer immunotherapy repurposed. Many chronic infections and sepsis cause "immune exhaustion" (T cells upregulate PD-1 and become functionally paralyzed). Anti-PD-1 reactivates them.

**Game design angle:** Must diagnose immune state first. Stimulating an already-hyperactive immune system during early sepsis is fatal; stimulating an exhausted system during late sepsis saves lives. The diagnostic requirement creates a "knowledge gate" before treatment.

---

## Key Strategic Dimensions (Summary)

These are the axes along which real treatments genuinely differ, and which could create distinct strategic decisions in gameplay:

| Dimension | Range |
|-----------|-------|
| **Development speed** | mRNA vaccines (days) ... live attenuated vaccines (years) |
| **Spectrum** | Phage therapy (single strain) ... interferons (any virus) |
| **Resistance risk** | Antimicrobial peptides/polyenes (very low) ... monoclonal antibodies (very high) |
| **Cost** | Doxycycline (pennies) ... ECMO/mAbs (thousands per day) |
| **Durability** | Convalescent plasma (days) ... live attenuated vaccine (lifetime) |
| **Side effect severity** | Echinocandins (minimal) ... amphotericin B/interferons (severe) |
| **Manufacturing complexity** | Small molecule pills ... personalized phage cocktails |
| **Target** | Host-directed (interferons, HDT) ... pathogen-directed (most drugs) |
| **Mechanism of failure** | Resistance mutations ... immune escape ... toxicity ... delivery failure |
