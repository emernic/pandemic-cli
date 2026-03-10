# Pandemic CLI

A real-time-with-pause terminal strategy game. You run a global health defense agency fighting concurrent disease outbreaks across six regions. Diseases will eventually overwhelm you — the question is how long you last and how many you save.

```
cargo run
```

## The Game

Diseases emerge throughout the game — each with its own pathogen type, transmission vector, mutation rate, and drug resistance profile. You research them, develop medicines, and deploy treatments while managing a collapsing budget, defiant regional governors, and corporate patrons who fund your operations with strings attached.

Three research tracks run simultaneously: field research (identify threats, run clinical trials), applied research (develop and manufacture medicines), and basic research (unlock new capabilities). The pipeline for each disease — identify, develop, trial, deploy — takes time you don't have.

Six regions with distinct economies, infrastructure, and leadership. Infrastructure degrades under pressure: healthcare capacity, supply lines, and civil order cascade into each other. A region collapses when deaths cross its threshold. When all six collapse, the game ends.

Funding comes from patrons — a shipping magnate who forbids travel bans, a resort billionaire who won't tolerate quarantine, an insurance CEO who pulls out when deaths get too high. Their money keeps you alive. Their conditions constrain your response.

Crisis events interrupt constantly — supply chain failures, black market drugs, political hearings, staff burnout, governor power grabs. Each demands a choice with no clear right answer.

## Running

```bash
cargo build            # build
cargo run              # interactive mode (Space to pause/resume)
cargo run -- --snapshot  # snapshot mode (for scripted/AI testing)
cargo test             # run tests
```

## Tech

Rust + [ratatui](https://github.com/ratatui/ratatui). Single binary, runs in any terminal. Deterministic simulation via seeded RNG.
