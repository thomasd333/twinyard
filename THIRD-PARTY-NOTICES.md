# Third-party notices

twinyard includes code from the projects below (Rust crates linked into twinyard.exe).
Most are MIT / Apache-2.0; the ones with extra terms are listed here.

## UnRAR (RAR archive reading)

twinyard reads RAR archives with the UnRAR library by Alexander Roshal (version 7.1,
source code compiled in through the `unrar_sys` crate). twinyard only extracts RAR
archives; it never creates or modifies them.

> UnRAR source code may be used in any software to handle RAR archives without
> limitations free of charge, but cannot be used to develop RAR (WinRAR) compatible
> archiver and to re-create RAR compression algorithm, which is proprietary.
> Distribution of modified UnRAR source code in separate form or as a part of other
> software is permitted, provided that full text of this paragraph, starting from
> "UnRAR source code" words, is included in license, or in documentation if license
> is not available, and in source code comments of resulting package.

All copyrights to RAR and the utility UnRAR are exclusively owned by the author,
Alexander Roshal. UnRAR is distributed "as is", with no warranty of any kind.

## 7z reading — sevenz-rust2

Apache-2.0, https://github.com/hasenbanck/sevenz-rust2

## ZIP — zip

MIT, https://github.com/zip-rs/zip2
