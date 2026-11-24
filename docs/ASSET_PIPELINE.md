# Asset pipeline (design)

A design for exporting objects' native data to usable formats (textures to
DDS and PNG, sounds to WAV, meshes and animations to glTF, ...), and later for
importing those formats back into package data. **Little here is
implemented yet**: the fallback, `usagi pkg export`, writes every object's
tagged properties as T3D and its native data, unread, as `.bin` (see
[CLI.md](CLI.md#usagi-pkg-export-paths---output-dir)); the first native
data types (step 1, below) read textures and sounds, which `usagi pkg diff`
compares; sounds export as WAV, through all three layers: `Sound`, the
`Audio` model and `WavWriter`, in `usagi-io`; and textures export as DDS,
PNG and TGA, through `Texture` and `Palette`, the `Image` model, and
`DdsWriter`, `PngWriter` and `TgaWriter`. Going the other way, so far
only within `usagi-io`, `TgaReader` reads a TGA file into an `Image`,
and `Image::compress` compresses one to DXT as UnrealEd's
`UTexture::Compress` does. The native data formats known so far are in
[PACKAGE_FORMAT.md](PACKAGE_FORMAT.md#native-data).

## Goals

- Export any kind of object, not just textures, without each new kind
  meaning changes all over the code.
- No pairing of every class with every file format: N readers and M writers
  should be N + M pieces of code, not N × M.
- Anything without a reader still exports, as `.t3d` and `.bin`.
- The same pieces work the other way round: a file format read into package
  data.

## Layers

```
 package bytes ──► [1. native data] ──► [2. adapter] ──► asset model ──► [3. writer] ──► file
               ◄──                  ◄──              ◄──             ◄── [3. reader]  ◄──
                          ▲
                [0. object context]
```

### 0. Object context (`usagi-pkg`)

What every reader needs, from one place:

- The object's class chain, most derived first.
- Its properties, typed (`props.get::<u8>("Format")`), **with class
  defaults**: a property equal to its default isn't stored (a texture
  without `Format` is `TEXF_P8`).
- A resolver for the objects it refers to (a texture's `Palette`, a mesh's
  materials), in the same package or another one.
- The package's version and licensee version.

Class chains and defaults come from the game's `.u` files, not from a table
in usagi (`Packages`, in `usagi-pkg`, does this now: it finds packages by the
game's `Paths`, resolves imports, finds properties' declarations through the
class chain, and reads tagged properties by them, including classes'
defaults): a package only names its classes (`Engine.Texture`), and the
superclass and `defaultproperties` are in the package that declares the
class. This needs:

- Reading a class export's data: its `UStruct`, `UState` and `UClass`
  fields, then its default properties (tagged properties, after the script).
  Done.
- Finding a class's package from the game directory. Done, from the game's
  `System/Default.ini` (and a mod's).
- Not done: a property's value isn't yet combined with its class's default
  when it isn't stored. The defaults are a chain too: a class's are only
  those that differ from its superclass's.

### 1. Native data (`usagi-pkg`)

Types that mirror the engine's C++ `Serialize` functions one for one:
`FMipmap`, `TLazyArray`, `FColor`, and later meshes' and sounds' structures.
Each is serialized by a single function that both reads and writes, as the
engine's `FArchive` and `operator<<` do, so reading and writing can't
disagree. `TLazyArray`'s absolute `SeekPos` is handled once, in its own
type, and fixed up on writing.

Done so far, by hand, in `usagi-pkg`: `LazyArray<u8>` (in `src/archive.rs`,
which works out `SeekPos` from `Archive::file_offset` when writing),
`Mipmap`, `Texture`, `Palette` and `Sound` (in `src/media.rs`). Textures before version
84 aren't read yet, and there's no registry by class yet.

**We intend to generate these functions with a derive macro, in a
proc-macro crate of its own**, once a few have been written by hand (texture,
palette, sound and a mesh) and the pattern is clear. What matters most is
that it's bidirectional, so one declaration gives both reading and writing,
and self-documenting, so the declaration *is* the layout:

```rust
#[derive(Serialize)]
struct Mipmap {
    data: LazyArray<u8>,   // SeekPos, then the array
    u_size: i32,
    v_size: i32,
    u_bits: u8,
    v_bits: u8,
}
```

It would also give, from the same declaration:

- Round-trip tests (read, write, compare) over every object in a corpus.
- A visitor over the fields, for dumps of any native data (names, offsets,
  values) and error messages with field paths (`Mips[3].Data: ...`).
- Version differences as attributes (`#[since(84)]`, `#[licensee(...)]`)
  rather than conditions at each call site.

Layouts that don't fit (the two mipmap arrays before version 84, sizes that
depend on a property) implement the same trait by hand. Until the macro
exists, every type is written by hand against a bidirectional `Archive`
trait, in the shape the macro will generate, so that moving to it changes
nothing built on top.

Readers are registered by class path (`Engine.Texture`). Lookup walks the
class chain to the nearest registered class: a subclass's data starts with
its parent's (`Super::Serialize`), so a `Cubemap` gets its `Texture` part
read and only the rest stays unread.

### 2. Adapters

Small functions per class between native data and a game-independent asset
model: `Texture` ↔ `Image`, `Sound` ↔ `Audio`, `StaticMesh` and
`SkeletalMesh` ↔ `Mesh`, `MeshAnimation` ↔ `Animation`. The models are few:
`Image` (format, mipmaps, palette), `Audio`, `Mesh`, `Animation`.

### 3. Writers and readers of file formats (`usagi-io`)

Each declares the asset model it takes and the format it writes: `Image` →
DDS, PNG, TGA; `Audio` → WAV; `Mesh` → glTF, PSK; `Animation` → glTF, PSA.
A writer may refuse an asset it can't hold without loss (DDS and a `P8`
image). Readers of formats (the `Reader` trait), for importing, go the
other way.

Done so far: `TgaReader`, a standard TGA reader rather than a copy of
UnrealEd's importer, whose quirks (ignoring the descriptor's origin,
16-bit read as greyscale) cause more trouble than they're worth:
colour-mapped, true-colour and greyscale images, uncompressed or
run-length encoded, at every bit depth the format defines, following
the descriptor's origin. 8-bit greyscale is read as `L8` and the rest as
`RGBA8`.

`Image::compress` makes DXT1, DXT3 and DXT5 images from `P8` and `RGBA8`
ones, with box-filtered mipmaps down to 1×1, as `UTexture::Compress`
asks NVIDIA's nvDXTlib for: DXT1 without alpha (`kDXT1`), allowing
three-colour blocks, without dithering. nvDXTlib is only in the engine's
source as a compiled library, so the encoder is usagi's own (a
principal-axis fit refined by least squares, exact for solid colours),
and its blocks aren't bit for bit UnrealEd's. `Compress`'s border and
fade options, which `TEXTURE IMPORT` sets for modulated and additive
textures, aren't supported.

They're in a crate of their own, `usagi-io`, so that `usagi-pkg` stays a
reader of the package format without image or audio dependencies.

## CLI

`usagi pkg export --native <FORMAT>` picks the writer, with a default for
each kind of asset (DDS for DXT textures, PNG for other images, ...).
`--raw` writes `.bin` as well. Objects without a reader or a writer are
written as `.t3d` and `.bin`, as now.

Done so far, with the flag named `--native` rather than `--format`, since
`--properties[=t3d|json|toml]` chooses the properties' format: `--native
auto` (the default) uses the default writer for each kind (WAV for sounds,
DDS for DXT textures and TGA for other textures, or PNG for 16-bit
greyscale ones, which TGA can't hold), `--native tga`, `png` and `dds`
choose textures' writer, `--native bin` turns writers off, and
`--native none` writes no native data; `--raw` works as above, and
`--mips` writes each mipmap of an image in a file of its own. Properties
aren't written unless asked for. Which class has a writer is still a check
in `pkg export` (`Engine.Sound` and `Engine.Texture` exactly), not yet the
registry by class chain that step 1 describes.

A texture's `Format`, `Palette` and `bMasked` are taken as 0 (`TEXF_P8`,
`None`, `False`) when they aren't stored, since `Engine.Texture` and its
superclasses don't give them defaults: class defaults aren't combined yet
(see layer 0).

## Open questions

- When to move this onto a persistent runtime (see
  [WISHLIST.md](WISHLIST.md#a-persistent-runtime)), since resolving
  references and class defaults means reading many packages.
