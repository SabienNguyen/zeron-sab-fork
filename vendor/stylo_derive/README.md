# stylo_derive (patched)

A copy of `stylo_derive` 0.22.0 from crates.io (https://github.com/servo/stylo,
MPL-2.0), used through `[patch.crates-io]` in the workspace manifest.

One change, in `to_css.rs`: the `Ok(())` expressions the `ToCss` derive emits
are written as `Ok::<(), std::fmt::Error>(())`. The derive follows them with
`?`, which leaves the error type to inference. GPUI enables `log`'s
`kv_serde` feature, which brings `serde_fmt` into stylo's dependency graph, and
`serde_fmt` implements `From<serde_fmt::Error> for fmt::Error`. With two
candidate conversions the unannotated form no longer compiles.

Remove this directory and the patch entry once stylo spells the type out
upstream or the HTML preview stops depending on it.
