# vehpatches.rs — samp.dll addresses

Spec: docs/re/subsystems/gta-patches.md, "Vehicles". The samp.dll site of every byte patch (A and B) is in the
`samp_site_A` / `samp_site_B` columns of `docs/re/data/gta-patches-r4-annotated.csv`, keyed by each entry's
`csv_row`; they are not repeated here.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `VEHICLE_COLOURS` | 0x101169C8 (.data) | 0x101169C8 (byte-identical) | samp's 256-entry vehicle palette, copied at init to 0x1026AE50 | gta-patches 4.5 |
| `apply_colour_table` | 0x100AADDB … 0x100AAE5B | 0x100AADAB … 0x100AAE2B | the seven operand writes redirecting `ms_vehicleColourTable` | gta-patches 4.5 |
