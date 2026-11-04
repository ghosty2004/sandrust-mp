# 0.3.7-R4 build diff: fv0.3.7.4 (2021, "A") vs fv0.3.7.3 (2019, "B")

A = `0.3.7-R4_fv0.3.7.4_2021-05-07/samp.dll` (sha256 0b0e409c…9088, PDB GUID 0684e97b-…, age 1)
B = `0.3.7-R4_fv0.3.7.3_2019-11-21/samp.dll` (sha256 15db80c5…12ab, PDB GUID 6e329a76-…, age 3)

REA evidence: full pseudocode exports A `ev_effc083a09f1a2cc16801e5554196309c5e712e82f429aa52b6883bedc717efa`, B `ev_51b698f5e4081f48bfa501de67cae66637707dc376a77092e934db90df5fe724`; S-box reads A `ev_27f4ca56…`, B `ev_a6bbaa7b…`. Snapshots `re/snapshots/R4_2021.snapshot`, `re/snapshots/R4_2019.snapshot`.

## Method

Both exports were compared by normalising pseudocode (all `FUN_/DAT_/LAB_` names, image addresses and stack-local names replaced by placeholders, numeric constants kept) and pairing functions inside a ±0x3000 address window. For the pairs that still differed, the ordered sequence of bitstream primitive calls with their bit widths was compared (primitive mapping A→B: bitstream primitives shifted by −0x50, the 1-bit reader 0x10008420→0x10008410). Data constants were read from both files at the same VA.

Whole-binary result: 5261/5262 decompiled functions in A/B; 5009 are identical after normalisation (multiset match). No string literal present in one build is missing from the other. B has one more function than A (not identified).

## Functions involved in handshake.md §1–6

| role | A | B | result |
|---|---|---|---|
| netgame constructor (host/port/nick/password, cipher key seed) | 0x1000b930 | 0x1000b920 | same behaviour; struct-offset differences only (see below) |
| connect tick (3000 ms, "Connecting to…") | 0x10008940 | 0x10008930 | same; struct offsets only |
| app packet dispatch switch | 0x1000b260 | 0x1000b250 | same case set and handlers; struct offsets only |
| auth-challenge handler (0x0c) | 0x1000a100 | 0x1000a0f0 | same bitstream sequence; struct offsets only |
| auth response computation | 0x100576f0 | 0x100576a0 | identical (normalised) |
| hex renderer | 0x100576b0 | 0x10057660 | identical |
| join on 0x22 (RPC 25 build) | 0x1000ad80 | 0x1000ad70 | same field order/widths, same 4057 and "0.3.7-R4", same serial call args (0x00c9236c, 1001); RPC ID cell 0x100e5b88 = 25 in both; local-player struct offsets differ |
| client serial (gpci) | 0x1005ffa0 | 0x1005ff50 | identical |
| SHA-1 init / block | 0x10063ae0 / 0x10063b70 | 0x10063aa0 / 0x10063b30 | identical |
| connection-rejected RPC 130 | 0x10010540 | 0x10010520 | same; struct offsets only |
| InitGame RPC 139 | 0x10010660 | 0x10010640 | same 28-field read sequence; local-player-ID store offset differs |
| server-closed (0x20) / connection-lost (0x21) | 0x10008de0 / 0x1000acf0 | 0x10008dd0 / 0x1000ace0 | same bitstream sequence |
| sync-packet handlers 0xc8,0xcb,0xce,0xcf,0xd0,0xd1,0xd2,0xd3 | 0x1000aa10, 0x10009c40, 0x10009b40, 0x1000a740, 0x10009f10, 0x10009a40, 0x10009e20, 0x10009d30 | 0x1000aa00, 0x10009c30, 0x10009b30, 0x1000a730, 0x10009f00, 0x10009a30, 0x10009e10, 0x10009d20 | same bitstream sequence (contents not analysed) |
| cipher checksum / substitute / encode / key set | 0x1001f630 / 0x1001f660 / 0x1001f6b0 / 0x1001f710 | 0x1001f5e0 / 0x1001f610 / 0x1001f660 / 0x1001f6c0 | identical; key global 0x10118b74 and S-box 0x100fe500 at the same VAs; S-box bytes identical |
| sendto / recvfrom wrappers | 0x10053ab0 / 0x10053a00 | 0x10053a60 / 0x100539b0 | identical |
| ProcessNetworkPacket (unconnected handling, cookie store) | 0x1003f440 | 0x1003f3f0 | identical; cookie global 0x10118e20, 0x6969 at 0x100fe7e0 in both |
| RakPeer update (request loop, internal msgs, ping, keep-alive) | 0x10041450 | 0x10041400 | identical |
| connect-request record | 0x1003e6a0 | 0x1003e650 | identical |
| RakPeer ctor (MTU 576) / SetMTU (≤1492) | 0x1003de50 / 0x10038860 | 0x1003de00 / 0x10038810 | identical |
| ping send | 0x1003f1a0 | 0x1003f150 | identical |
| reliability: datagram parse / build / message write / message read / split | 0x10049250 / 0x10048070 / 0x10045530 / 0x10045650 / 0x10045800 | 0x10049200 / 0x10048020 / 0x100454e0 / 0x10045600 / 0x100457b0 | identical |
| ack range write / read | 0x10046f40 / 0x10047d20 | 0x10046ef0 / 0x10047cd0 | identical |
| reliability update / layer init (reliability width 4) | 0x10049ae0 / 0x10048e10 | 0x10049a90 / 0x10048dc0 | identical |
| compressed write / read | 0x10020130 / 0x1001fa70 | 0x100200e0 / 0x1001fa20 | identical |
| RPC send / RPC receive / packet getter | 0x1003ecb0 / 0x1003ade0 / 0x10040fd0 | 0x1003ec60 / 0x1003ad90 / 0x10040f80 | identical |
| RPC registration (core / script) | 0x10012450 / 0x1001e130 | 0x10012420 / 0x1001e0e0 | identical ID values and order (141 entries); handler-by-handler bitstream sequences identical (struct offsets differ in 70 handlers) |
| client-sent RPC call sites | 34 sites (see rpc-ids.md) | 34 sites | identical ID set, priorities and reliabilities |

**Conclusion: no wire-protocol difference was found between the two R4 builds** for anything in handshake.md §1–6. One Rust implementation covers both.

## Notable non-protocol differences

* The netgame object layout changed: in A the RakClient interface pointer is at offset 0; in B it is at +0x2c (B code reads `[this+0x2c]` where A reads `[this]`). Host string and other fields keep the same offsets.
* The player-pool / local-player layout changed: local player pointer at pool +4 (A) vs +8 (B); local name std::string at +0x0a (A) vs +0x12 (B); local player ID at +4 (A) vs +0xc (B). These account for most of the ~250 non-identical functions.
* B addresses are lower than A's by an offset that grows in steps over the start of .text (−0x10 around 0x10008000, −0x20/−0x30 around 0x10010000–0x10012000, −0x50 from about 0x1001e000 onward), consistent with small code-size changes in the netgame/player code; the RakNet part is a uniform −0x50.
