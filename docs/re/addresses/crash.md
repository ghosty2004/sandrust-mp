# crash.rs — samp.dll addresses

Only build B was read for the crash handler; A addresses are not recorded.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `install` | — | 0x100C5121 | call in `DllMain` (0x100C50E0) that installs the exception filter | docs/phase3/featb-notes.md, "Crash handler of the original" |
| `imp::filter` | — | 0x10060920 | unhandled-exception filter | same |
| `dialog_template`, `CAPTION`, `ID_TEXT`, `ID_CLOSE` | — | 0x10060820 | shows dialog resource 101 (caption, report edit 1000, Close 1001, Report Crash 1002) | same |
| `report` | — | 0x10060560 | report builder (module list off for the dialog) | same |
| `STACK_DUMP` | — | 0x10060596 | the 0x280-byte stack dump size in the report builder | same |
| `report` (stack lines) | — | 0x10060240 | stack dump formatter, four dwords per line | same |
| `report` (module list) | — | 0x100603D0 | module list from a ToolHelp snapshot (upload only in samp) | same |
| (left out) | | | remote-player object fields +0x10A and +0x1E7 printed in "State Information", meaning not identified | featb-notes.md, "Decisions" |
