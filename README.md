# Does it JSON?

Simple crate to validate that a type's serialization via `serde` matches the
JSON schema from `schemars`.

This is particularly useful when hand-rolling (rather than deriving)
`serde::Serialize` and/or `schemars::JsonSchema`--it can be easy to
accidentally have divergence between the two.

```rust
let item = MyType::create_somehow();
does_it_json::validate(&item).unwrap();
```

For best results, apply to a variety of instantiations of your type.
