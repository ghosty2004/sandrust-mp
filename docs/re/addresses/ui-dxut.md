# ui/dxut.rs — DXUT-style GUI layer

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `RM_STATE_BLOCK` | 0x1008CBC0 | | GUI dialog / resource-manager creation path; the `D3DSBT_ALL` state block is created with the GUI sprite's `OnResetDevice` (measured, docs/phase3/ux1-notes.md) | ui-rendering.md, "1.1 UI objects", "4.1 Resource manager and skin" |
