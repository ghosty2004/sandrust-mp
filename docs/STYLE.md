# Code style

The code is read by people who were not part of the reverse-engineering work. A comment must make sense to
someone who knows Rust and GTA modding but has never seen our task board or measurement logs.

## Comments
1. **Say what the code does and why**, in plain English. Measurement history ("measured in run X", "the
   original sent … 4.2 ms later", "TRACE-6 showed …") belongs in `docs/phase3/*-notes.md`; the code keeps
   the rule and, when it is not obvious, one short reason.
2. **No internal ids** in code: no task / milestone / decision ids (M5, M8a, TRACE-6, FEAT-B, DIFF2-D1,
   MR-5.9, U-KEXP, CRASH-1, NET6-P3, …), no "CSV row 51", no "UR §5" / "UCA §101" shorthands. When a
   reader needs the background, point to the document by path and section title:
   `(docs/re/subsystems/vehicles.md, "Trailers")`.
3. **samp.dll addresses**: at most one per item, in that item's doc comment, as
   `Mirrors samp.dll 0x100A3BF0.` (build A, the 2021 build; add `B 0x…` only where the builds differ in a way
   the code depends on). Chains of addresses, call graphs and constants' origins go to the module's table in
   `docs/re/addresses/<module>.md` (columns: our item, samp.dll A, samp.dll B, what it is, spec section).
4. **GTA addresses and offsets** are named constants; the constant's doc comment says what it is (one line).
   Do not repeat the number in prose elsewhere.
5. **Module header** (`//!`): 2–8 lines — what the module is responsible for, its main entry points, and
   the spec it implements (doc path). No lists of every address the module touches.
6. **No stale notes**: no "not implemented yet" for things that exist, no commented-out code. A `TODO` only
   for an open item that is also on `docs/ROADMAP.md`, phrased as what is missing.
7. **Safety comments** (`// SAFETY:`) stay: one line saying why the access is valid.
8. Tests: a test name says the behaviour; a comment only where the expected value needs a source.

## Code
- `cargo fmt` (rustfmt.toml: width 120) and `cargo clippy` clean on host and `i686-pc-windows-gnu`.
- Files: aim for under ~1,000 lines; split by responsibility, not by size alone.
- Functions: aim for under ~100 lines; an RPC dispatcher is a `match` that calls one handler per arm.
- Raw game-memory access goes through the typed helpers in `patch.rs` / `game.rs` (`rd`, `wr`, offsets as
  named constants); `unsafe` blocks are small and carry a `SAFETY` line.
