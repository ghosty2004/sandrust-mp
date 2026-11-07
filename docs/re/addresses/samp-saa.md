# samp-saa — samp.dll addresses

Addresses behind `crates/samp-saa`. The spec (docs/re/samp-saa.md) was written from build B; data
constants sit at the same address in both builds, code in this region is 0x40 bytes later in build A
(spec header). A cell marked "B + 0x40" was not checked separately in build A.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `DIRECTORY_KEY` | 0x100E9A2C | 0x100E9A2C | 16-byte directory key, stored XORed with 0xAA | 4.1 Decryption |
| `verify::RSA_MODULUS_LE` | 0x100E9998 | 0x100E9998 | PUBLICKEYBLOB, 148 bytes, stored XORed with 0xAA | 6. Signature |
| `crypto::XTEA_DELTA` | 0x101027B4 | 0x101027B4 | masked XTEA delta 0x0CCF225C | 5. Chained XTEA |
| `crypto::XTEA_ROUNDS` | 0x101027B0 | 0x101027B0 | XTEA cycle count (32) | 5. Chained XTEA |
| `crypto::name_hash` seed | 0x10102790 | 0x10102790 | masked golden-ratio seed of the name hash | 7.1 Name hash |
| `INTEGRITY_TABLE_HASH` | B + 0x40 | 0x10062390 | routine that requests the unnamed entry by hash 0xA848B69A | 8. Redirection layer |
| `KNOWN_ENTRIES` "logo.png" | — | 0x10070C20 | loads logo.png by hash as a D3D texture | 9. Contents of the 0.3.7-R4 archive |
| `KNOWN_ENTRIES` "logo.png" | — | 0x100576A0 | feeds logo.png's bytes into a checksum | 9. Contents of the 0.3.7-R4 archive |
