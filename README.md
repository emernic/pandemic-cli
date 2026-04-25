# Pandemic CLI

Inverse Plague Inc. — a real-time-with-pause strategy game where you defend humanity against concurrent disease outbreaks. Terminal UI built in Rust with [ratatui](https://github.com/ratatui/ratatui).

You run the N.W.H.O. (New World Health Organization), managing six global regions as diseases emerge, mutate, and spread. The game is unwinnable — diseases will eventually overwhelm you. A good run lasts about 40 days.

```
 RUNNING   Day: 12.4  POL: 38%  Funds: ¥2,140 (+¥312/day)  Personnel: 14  Infected: ~8.2M ▲  Dead: 1.4M
 Field: ▶ Ehrlichia-Delta [Trial]  │  Applied: ▶ Anti-Staph IV [Manufacture]  │  Basic: ▶ Rapid Sequencing
 ──────────────────────────────────────────────────────────────────────────────────────────────────────
 ╔════════════════════════╗   ┌────────────────────────┐   ┌────────────────────────┐
 ║North America       CRIT║   │Europe              HIGH│   │Asia                 MOD│
 ║Inf~ 4.1M  Dead 890K    ║   │Inf~ 2.3M  Dead 340K   │   │Inf~ 1.2M  Dead 95K    │
 ║████████████████▓▓▓▓░░░░║───│██████████████████▓▓░░░│───│██████████████████████▓░│
 ╚════════════════════════╝   └────────────────────────┘   └────────────────────────┘
```

## Quick Play

Running this one line in your terminal will download, install, and run the game in one shot:
```
command -v cargo >/dev/null || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y; . "$HOME/.cargo/env" 2>/dev/null; cargo install --locked --git https://github.com/emernic/pandemic-cli && pandemic-cli
```

Note:
- Only works on Mac or Linux
- Only run if you trust me ;)

## Build

```bash
cargo build && cargo run
```

Requires Rust (stable). Single binary, runs in any terminal.
