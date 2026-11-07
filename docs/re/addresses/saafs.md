# saafs.rs — samp.dll addresses

samp.saa fake-file rules (`crates/samp-client/src/saafs.rs`). A = 2021 build, B = 2019 build; the spec was
read from build B, and build A has this code 0x40 bytes later (docs/re/samp-saa.md, introduction), so only
B is listed for code. Data addresses are the same in both builds.

| our item | samp.dll A | samp.dll B | what it is | spec section |
|---|---|---|---|---|
| `FIRST_HANDLE`, `HANDLE_STEP` | 0x10102350 | 0x10102350 | next-fake-handle counter (data) | samp-saa.md, "8. Redirection layer" |
| `FakeFiles::open` | — | 0x100625B0 | allocate a fake-file slot, hand out the next handle | samp-saa.md, "8. Redirection layer" |
| `in_fake_range` | — | 0x10062540 | "is this one of our handles" range test | samp-saa.md, "8. Redirection layer" |
| `MAX_OPEN` | — | 0x10062560 | slot lookup over the 50-entry fake-file table | samp-saa.md, "8. Redirection layer" |
| `FakeFiles::read` | — | 0x10062630 | `ReadFile` hook | samp-saa.md, "8. Redirection layer" |
| `FakeFiles::seek` | — | 0x10062800 | `SetFilePointer` hook | samp-saa.md, "8. Redirection layer" |
| `FakeFiles::size` | — | 0x100627B0 | `GetFileSize` hook | samp-saa.md, "8. Redirection layer" |
| `FakeFiles::close` | — | 0x100628A0 | `CloseHandle` hook | samp-saa.md, "8. Redirection layer" |
| `Redirector::data` (cache) | — | 0x10064F20 | decrypted-entry read; the buffer stays cached after close | samp-saa.md, "Constants" |
| `filehook::get_file_type` | — | 0x100628E0 | `GetFileType` hook (answers 1, disk) | samp-saa.md, "8. Redirection layer" |
| `trim_trailing_space` | — | 0x10062950 | `CreateFileA` path preprocessing (trailing space → NUL) | samp-saa.md, "7.2 By path" |
