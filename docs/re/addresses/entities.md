# entities.rs — samp.dll addresses

Pickups, actors, checkpoints, gang zones, map icons and 3D labels (`crates/samp-client/src/entities.rs`).
A = 2021 build, B = 2019 build. Spec: docs/re/subsystems/world-entities.md.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `IS_LINE_OF_SIGHT_CLEAR` | 0x100EA618 | — | script-command descriptor (0x06BD, "ffffffiiiii") | world-entities.md, "36 PlayerShowTextLabel" |
| `LOS_CAMERA_POS` | 0x101039C8 | — | pointer to GTA 0xB6F32C; the label LOS test reads the vector at +0xC | world-entities.md, "36 PlayerShowTextLabel" |
| `Entities::on_rpc` (pickups) | 0x10013270 | — | pickup create | world-entities.md, "95 PlayerCreatePickup" |
| `Entities::on_rpc` (pickups) | 0x10013320 | — | pickup destroy | world-entities.md, "63 PlayerDestroyPickup" |
| `Entities::on_rpc` (actors) | 0x10001900 | — | actor pool: create | world-entities.md, "171 ShowActorForPlayer" |
| `Entities::on_rpc` (actors) | 0x1009C310 | — | actor wrapper (ped creation) | world-entities.md, "171 ShowActorForPlayer" |
| `Entities::on_rpc` (checkpoints) | 0x100A1E20 | — | normal checkpoint set | world-entities.md, "107 SetCheckpoint" |
| `Entities::on_rpc` (checkpoints) | 0x100A2140 | — | race checkpoint set | world-entities.md, "38 SetRaceCheckpoint" |
| `Entities::on_rpc` (gang zones) | 0x10002180 | — | gang-zone pool | world-entities.md, "Gang zones" |
| `Entities::on_rpc` (map icons) | 0x1000A300 | — | RPC 56 handler | world-entities.md, "56 SetPlayerMapIcon" |
| `Entities::on_rpc` (map icons) | 0x100A0DD0 | — | blip creation called by the RPC 56 handler | world-entities.md, "56 SetPlayerMapIcon" |
| `Entities::on_rpc` (3D labels) | 0x100011D0 | — | label pool: store (colour codes kept, `~k~` expansion) | world-entities.md, "36 PlayerShowTextLabel" |
| `Entities::on_rpc` (3D labels) | 0x100012E0 | — | label pool: delete | world-entities.md, "58 PlayerHideTextLabel" |
| `Entities::on_rpc` (3D labels) | 0x100B6140 | — | GTA key-name (`~k~`) expansion helper | world-entities.md, "36 PlayerShowTextLabel" |
| `Entities::delete_actor` | 0x1009C460 | — | actor wrapper destructor (scalar deleting destructor of the ped) | world-entities.md, "172 HideActorForPlayer" |
| `Entities::tick` (checkpoints), `add_checkpoint_blip` | 0x100A1F60 | — | per-frame checkpoint test and blip | world-entities.md, "Checkpoints" |
| `Entities::tick` (pickup poll) | 0x10013520 | — | pickup poll every 7 frames (type 14) | world-entities.md, "131 OnPlayerPickUpPickup" |
| `Entities::labels_tick` | 0x10001350 | — | 3D label placement | world-entities.md, "36 PlayerShowTextLabel" |
| `Entities::clear_pickups` | 0x10013410 | — | pickup pool deletion | world-entities.md, "Pickups" |
| `Entities::clear_actors` | 0x100018E0 | — | actor pool deletion | world-entities.md, "Actors" |
| `set_actor_health` | 0x1009C630 | — | actor health (≤ 0 → EXPLODE_CHAR_HEAD) | world-entities.md, "178 SetActorHealthForPlayer" |
