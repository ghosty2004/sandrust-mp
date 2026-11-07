# filehook.rs — samp.dll addresses

samp.saa redirection, Windows side (`crates/samp-client/src/filehook.rs`). A = 2021 build, B = 2019 build;
build A has this code 0x40 bytes later than B (docs/re/samp-saa.md, introduction). The per-hook addresses
are in `saafs.md`.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `init` (hook install) | — | 0x10062C60 | samp inline-hooks the six kernel32 exports with 5-byte jumps; we patch gta_sa.exe's import table instead | samp-saa.md, "8. Redirection layer" |
| `create_file_a` | — | 0x10062950 | in-place trailing-space cut of the caller's path | samp-saa.md, "7.2 By path" |
