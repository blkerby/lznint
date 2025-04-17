# lznint

A library for compressing and decompressing data using the LZ-based compression format used by Nintendo in Super Metroid.

## Example

```rust
let input = [0x1, 0x2, 0x3, 0x4, 0x1, 0x2, 0x3, 0x4];

let compressed = lznint::compress(&input);
println!("{:x?}", compressed);  // [63, 1, c3, 4, ff]

let decompressed = lznint::decompress(&compressed).expect("Decompressino failed");
assert_eq!(&decompressed, &input);
```
