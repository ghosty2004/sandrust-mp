# camtarget.rs — samp.dll addresses

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `CamTarget::tick` | 0x10004700 | 0x100046e0 | per-tick camera target update and RPC 168 sender | player-state.md "168"; session-misc.md |
| `INTERVAL_MS` | — | 0x10004702 | `cmp 100` of the rate limit (strictly more than 100 ms) | player-state.md "168" |
| `ray`, `nearest` | 0x10004440 (spec, build not stated) | 0x10004420 | target computation (ray, line-of-sight shortening, pool searches) | player-state.md "168" |
| object search (`OBJECT_RADIUS`) | — | 0x10012E40 | pool ray test of selection mode, 40 start distance | player-state.md "168" |
| vehicle search (`VEHICLE_RADIUS`) | — | 0x1001EEE0 | vehicle pool search | player-state.md "168" |
| player search (`PED_RADIUS`) | — | 0x10013D50 | player pool search | player-state.md "168" |
| actor search (`PED_RADIUS`) | — | 0x10001770 | actor pool search (uses the players' shortened ray) | player-state.md "168" |
| `VEHICLE_SCALE`, `PED_SCALE` | — | 0x100B65E0 | bounding-sphere / segment line test (scale argument 1.0 / 1.5) | player-state.md "168" |
| `label`, `LABEL_COLOUR` | — | 0x10004830 | `/ctd` painter: axis cross + `CAMERA_TARGET_TYPE_*` label in 0xFFD47B75, outline on, no LOS test | docs/phase3/feata-notes.md |
| `CamTarget::enabled` | | | netgame +0x233 (enable flag set by RPC 170) | player-state.md "170" |
| `CamTarget` | | | local player +0x11F..+0x12A (cached ids and timer) | player-state.md "168" |
| `ray` (GTA, not samp.dll) | gta_sa.exe 0xB6F32C / 0xB6F338 | | `TheCamera.m_aCams[0]` front / source vectors | player-state.md "168" |
