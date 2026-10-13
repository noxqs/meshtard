# SketchUp VFF container (2021+): what we know

Research notes towards writing SketchUp 2021+ `.skp` files. No public spec or writer
exists; OpenSKP (MIT, commit `359d183`) and Meshtard only read VFF. Tags:
`[code]` = from our reader (`crates/mt-skp/src/vff.rs`, `tlv.rs`, ported from OpenSKP),
`[observed]` = seen in the corpus files, `[guess]` = unverified.

**Status (2026-10-13): research parked by the user (low priority).** Only the container
survey below was done. No VFF writer code exists yet; the round trip (step 2) and
templated new models (step 3) are not started. See "Where this stopped".

Corpus VFF files (`tests/corpus/openskp`): `SU_File` (25.0.575, 16,708 B),
`Untitled` (25.0.575, 415,694 B), `style_watermark` (25.0.575, 15,636 B),
`coedge_orientation_regression` (26.1.194, 5.5 MB). Survey scripts were throwaway
Python (zipfile + struct); nothing needs installing.

## 1. File header (69 bytes in every corpus file) [observed]

| Offset | Bytes | Meaning |
|---|---|---|
| 0 | `FF FE FF 0E` | MFC CString marker (`FF FE FF`) + length 14 UTF-16 chars [code: `valid_header`] |
| 4 | `"SketchUp Model"` UTF-16LE | 28 bytes |
| 32 | `FF FE FF 0A` | CString marker, length 10 |
| 36 | `"{25.0.575}"` UTF-16LE | version `{major.minor.build}`; major <= 20 means legacy MFC body [code: `is_legacy`] |
| 56 | `"VFF"` ASCII | container magic |
| 59 | `08 00 01 00 11 00` | identical in all four files [observed]; three u16 (8, 1, 17)? meaning unknown [guess: format/schema versions] |
| 65 | 4 bytes | varies per file: `3F71527C` (SU_File and style_watermark), `E8EA7DD3` (Untitled), `6A32CE28` (coedge) |
| 69 | `PK 03 04` | the ZIP starts here; the reader scans for it in the first 4 KB [code: `zip_offset`] |

The 4 bytes at 65: **not** CRC-32 (either byte order) of the ZIP, the header, header+ZIP,
or any single ZIP entry [observed, tested]. `SU_File` and `style_watermark` share the
value and have byte-identical `model.dat`/`meta.dat` but differently packed ZIPs, so the
value follows the model content, not the container bytes [observed]. Candidates [guess]:
a hash over `model.dat` with another algorithm (Adler-32, CRC-32C, xxHash, FNV), a hash
of the model GUID (see meta.dat `f`), or a random save id. **Open question; must be
resolved before writing**, unless SketchUp ignores it (test: flip it in a copy and open
in SketchUp, see section 6).

## 2. The ZIP [observed]

- Ends exactly at the EOCD (no trailing bytes, no archive comment). Offsets in the
  central directory are relative to the ZIP start (offset 69), not the file start.
- Entry order (SketchUp-saved files): `meta/model_thumbnail.png`,
  `meta/preview_thumbnail.png`, `meta/meta.dat`, then `materials/<folder>/...` (texture
  image before its `material.xml` when present, `thumbnail.jpg` after), layer
  pseudo-materials `materials/Layer_<name>/material.xml`, then
  `styles/[<style>]/style.xml` (and `[<style>]_1` working copy), style images, then
  `model.dat` (largest) and, if any, root-level `watermarks/*.png`.
- Every entry: method 8 (deflate). Local header version-needed 45, central "made by"
  0x0000 (MS-DOS, version 0), version-needed 20. Timestamps zeroed: time `0x0000`,
  date `0x0020` (1980-01-00, an invalid DOS day) - deterministic.
- General-purpose flags: `0x0` for already-compressed images (PNG/JPG), `0x6`
  (deflate "super fast" option bits) for text/binary entries. No data descriptors,
  no encryption.
- Local headers carry a 20-byte extra field `01 00 10 00` + 16 zero bytes (a Zip64
  extended-information record with zeroed sizes). Central headers have no extra field.
  Internal/external attributes 0. No directory entries.
- Deflate stream: produced by SketchUp's own zlib-like compressor at a fast level;
  recompressing with zlib will very likely not give the same bytes [guess], so byte
  identity of a rewrite needs the original compressed streams copied verbatim when an
  entry is unchanged.
- `style_watermark.skp` is the same model as `SU_File` re-packed by another ZIP tool
  (version 20, no Zip64 extras, flags 0, real DOS timestamps on the PNG entries) plus
  a watermark image [observed]. Whether SketchUp opens it is not known; if it does,
  SketchUp accepts generic ZIP writers (strong hint that the container layout is lax).

## 3. `meta/meta.dat` [observed]

Flat TLV with **1-byte tag + 1 zero byte** as a 2-byte tag, u32 length, payload
(same framing as `model.dat`). Wrapped in `64 00` (`d`); records seen:
`75` (`u`) version text `25.0.575`; `76` (`v`) u16 = 0x18 (25) / 0x19 (26.1) [guess:
file schema]; `77` u16 = 2 (2025) / 4 (2026); `73` u16 = 1; `74` u16 = 0x11 (also the
last u16 of the header at 63); `66` (`f`) **16-byte GUID**, different per model
(`SU_File` and `style_watermark` equal) [guess: model GUID, a candidate input of the
header hash]; `67` (`g`) a list of `2328`/`2923`/`2A23` records naming the
entries (`meta/model_thumbnail.png`, ...) [guess: the entry manifest, maybe with sizes
or hashes per entry - not decoded yet]; `6D00` units string [code: `read_meta_units`].

## 4. `model.dat` [code + observed]

- TLV: 2-byte tag, u32 LE length, payload. A fixed list of tags are containers
  (children are TLV), everything else is a raw leaf [code: `tlv::CONTAINERS`]. A writer
  that keeps leaf payloads as bytes and re-emits lengths reproduces the stream exactly
  as long as no container contains trailing non-TLV bytes (to verify).
- Top level is one `F401` record. Its first children: `6300` (4 bytes, 0),
  `0A02` = `"meta/meta.dat"`, `0902` ... [observed]: the model refers to other ZIP
  entries by name, so entry names are part of the model data.
- Known top-level records [code]: `F601` root entities (geometry, model attribute
  dictionaries), `F901` > `7017` > `7117` > `7C15` component definitions, `FA01` >
  `BC34` current view camera, `0602` > `7869` style catalog (`7969` list, `7A69`
  current style id, `7B69` working copy, `7C69` modified flag; style ids in
  `DC05` > `DE05`). Entity ids and references: see `geometry.rs`, `pages.rs`.
- Materials and tags are stored twice: as XML entries in the ZIP and as records in
  `model.dat` (`collect_material_ids`, `collect_layers`) [code]; a writer must keep
  both in sync [guess: which one SketchUp trusts].

## 5. Checksums / hashes / GUIDs found so far

| Where | What | Covers |
|---|---|---|
| ZIP CRC-32 per entry | standard | the entry's uncompressed bytes [observed] |
| Header bytes 65..69 | 4 bytes, unknown algorithm | follows the model content (section 1) [observed]; open |
| meta.dat `f` | 16-byte GUID | per model; [guess] model GUID |
| meta.dat `g` | per-entry records | not decoded; may hold sizes/hashes of other entries [guess] |
| model.dat | definition GUIDs (`RawDefinition::guid`), style ids | [code] |

## Where this stopped

Done: header, ZIP layout, meta.dat outline (this file). Not done:

1. Identify the header hash (try Adler-32/CRC-32C/xxHash32/FNV over `model.dat`,
   `meta.dat`, the GUID, and over `F401` payload) and decode meta.dat `g`.
2. Round-trip writer `mt_skp::write::vff`: keep each entry's raw compressed stream and
   header fields, re-emit locals + central directory + EOCD; expected byte-identical for
   unchanged entries. Then re-serialize `model.dat` from the TLV tree and check
   identical inner bytes.
3. Templated new model from `Untitled.skp`; experimental Save As behind
   `MESHTARD_EXPERIMENTAL_VFF=1`.

## 6. Only real SketchUp 2026 can tell (for the trial)

- Does SketchUp open `style_watermark.skp` (generic ZIP packing)? Tells whether the
  ZIP details (Zip64 extras, flags, timestamps, order) matter.
- Does it open a copy of `SU_File.skp` with header bytes 65..69 flipped? With the
  meta.dat GUID changed? Tells whether the unknown hash is checked.
- Does it open a copy whose entries were recompressed with zlib (same content)?
