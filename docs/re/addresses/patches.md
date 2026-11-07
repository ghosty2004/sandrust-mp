# patches.rs — samp.dll addresses

Start-up patch table (`crates/samp-client/src/patches.rs`). A = 2021 build, B = 2019 build. Every entry's
samp.dll write site (both builds) is in docs/re/data/gta-patches-r4-annotated.csv (columns `samp_site_A`,
`samp_site_B`) at the entry's patch table row; the rows below are the ones the code used to cite inline.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `TABLE` entry, patch table row 179 | 0x100AB291 | — | init-main write: NOP `CStreaming::DeleteRwObjectsAfterDeath` call in `CGameLogic::SortOutStreamingAndMemory` | gta-patches.md, "Full table" |
| `TABLE` entry, patch table row 199 | 0x100AB53D | — | init-main write: `jmp +0x77` over the BMX branch of `CFire::ProcessFire` | gta-patches.md, "Full table" |
| `TABLE` entry, patch table row 203 | 0x100AB5FF | — | init-main write: NOP a `CSprite2d::DrawRect` in `CMenuManager::PrintMap` | gta-patches.md, "Full table" |
| `TABLE` entry, patch table row 212 | 0x100AB74C | — | init-main write: NOP `Interior_c::Exit` call in `InteriorGroup_c::Exit` | gta-patches.md, "Full table" |
| `TABLE` entry, patch table row 245 | 0x100ABA04 | — | init-main write: `CBirds::CreateNumberOfBirds` LOS test also checks objects | gta-patches.md, "Full table" |
