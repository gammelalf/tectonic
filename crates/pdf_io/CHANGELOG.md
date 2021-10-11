# tectonic_pdf_io 0.1.2 (2021-10-11)

- Fix an incorrect warning issued when reading EXIF data (#822, @korrat)


# tectonic_pdf_io 0.1.1 (2021-07-03)

- Fix the build on Rust 1.46, needed for the conda-forge macOS build (which is
  stuck on this version of Rust for the time being)
- Fixes for complaints from Clippy 1.53.0


# tectonic_pdf_io 0.1.0 (2021-06-03)

This crate contains the vast majority of the C/C++ code from `xdvipdfmx`. It
provides I/O services to both the `tectonic_engine_xdvipdfmx` and
`tectonic_engine_xetex` crates through its C/C++ APIs. It does not provide a
Rust API.

This crate deals with general graphics I/O, not just PDF files, but the majority
of its code is PDF-centric.