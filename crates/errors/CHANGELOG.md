# tectonic_errors 0.1.0 (2021-01-15)

Initial release. A new crate providing a generic boxed error type for Tectonic.

We need a boxed error type because we have a bunch of optional dependencies, and
we can't abstract around their errors without boxing them.

Strongly derived from [Cranko](https://github.com/pkgw/cranko).