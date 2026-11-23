# Package writer (design)

A design for writing package files: changing objects' properties
(`usagi pkg set`, below), and later importing assets (see
[ASSET_PIPELINE.md](ASSET_PIPELINE.md)). The binary format is in
[PACKAGE_FORMAT.md](PACKAGE_FORMAT.md).

**Status.** Implemented: a reader and writer for the native data of every
class in UT2004's and Red Orchestra's packages (the [census](#class-census)),
the full rewrite ([way B](#b-a-full-rewrite-canonical-layout-implemented),
`Package::write_with` and `Packages::write` in `src/write.rs`), and the
census command, `usagi pkg classes`, whose `--round-trip` is the
[round-trip check](#round-tripping-is-the-test). Every object and every
package in a Red Orchestra install with Darkest Hour's development tree
(1,691 packages) and a UT2004 install (840) writes back byte for byte. Not
yet implemented: changing anything (`pkg set`, writing tagged properties,
adding names and imports). Appending
([way A](#a-appending-first-superseded)) won't be: see
[Decisions](#decisions).

## Why a writer is needed

The first user is setting properties on many objects at once:

```sh
usagi pkg set Textures/UI.utx "Texture'UI.HUD.*'" LODSet=LODSET_Interface
usagi pkg set Textures/*.utx --class Texture LODSet=LODSET_Interface --dry-run
usagi pkg set UI.utx "UI.HUD.*" --reset LODSet          # back to the class default
usagi pkg set UI.utx "UI.HUD.*" LODSet=LODSET_Interface -o patched/
```

Values are written in T3D, as `pkg obj` prints them, so anything copied from
its output can be pasted back.

Hardly any edit leaves an export the same size:

- The engine only saves properties that differ from the class's default
  ([Tagged properties](PACKAGE_FORMAT.md#tagged-properties)). A texture
  whose `LODSet` is still the default has no `LODSet` tag, so setting it
  adds one (a name index, the info byte, the value), and `--reset` removes one.
- Strings, arrays and structs change length with their values.
- A new tag may need names the package doesn't have yet (`LODSet`, a
  `NameProperty`'s value), so the **name table** grows. An object reference
  may need a new **import** (and an import of its package).

When an export changes size, every later export's `SerialOffset` moves, and
so does everything in the file after it. A new name grows the name table,
and that table comes *before* the export data, so every export moves. So
`pkg set` needs to write packages, not patch them.

## The obstacle: absolute offsets in export data

The container only places things through the summary's offsets and each
export's `SerialOffset`/`SerialSize`, and both are easy to rewrite. But some
**export data contains absolute file offsets**: `TLazyArray`'s `SeekPos`
([Absolute offsets inside export data](PACKAGE_FORMAT.md#absolute-offsets-inside-export-data)).
If an export holding one moves without being re-serialized at its new
offset, the engine skips to the wrong place and reads garbage. These
classes have one, from the UT2004 source (`TLazyArray<` in `Engine/Inc`):

| Class | Lazy arrays | Header |
|---|---|---|
| `Texture` (and subclasses) | each mipmap's `DataArray` | `UnTex.h` |
| `Sound` | its data (`FSoundData`) | `UnAudio.h` |
| `Model` | lightmap textures' `Data[2]` | `UnModel.h` |
| `StaticMesh` | `RawTriangles` | `UnStaticMesh.h` |
| `SkeletalMesh` (and its LOD models) | `Influences`, `Wedges`, `Faces`, `Points`, `RawVerts`, `RawNormals`, `RawWedges`, `RawFaces`, `RawInfluences`, `RawCollapseWedges`, `RawFaceLevel` | `UnSkeletalMesh.h` |

usagi now reads all of these (`LazyArray` in `src/archive.rs` works
`SeekPos` out from where the data will be, for any type of item), and fire
textures, which are textures too. A licensee's own classes may have lazy
arrays as well. So an export can only be moved if usagi either knows its
class has no absolute offsets, or can read it and write it again at the new
offset: `NativeClass::has_absolute_offsets` says which.

## Two ways to write

### A. Appending (first; superseded)

*Not implemented: with every class read, way B works for every package, and
it doesn't leave dead bytes or a layout the engine has never written.*

Leave every export that hasn't changed where it is, byte for byte, and put
whatever changes after the export data:

```text
before                          after
Summary                         Summary            (rewritten: new counts and offsets)
Name table                      old name table     (dead: nothing points to it)
Export data                     Export data        (unchanged exports, same offsets)
Import table                    old copies of changed exports (dead)
Export table                    changed exports' data, at new offsets
                                Name table         (old names, then new ones appended)
                                Import table       (old imports, then new ones appended)
                                Export table       (new offsets and sizes for changed exports)
```

- Only **changed** exports are written again, and only they need a reader
  for their native data: a texture's mipmaps get a new `SeekPos` from
  `Saver::at(new_offset)`. An export with native data whose class usagi
  can't read is refused, unless the class is known not to hold absolute
  offsets (see the census).
- Names, imports and exports keep their indices. New ones are **appended**,
  so every object reference and name index in unchanged data stays right.
  This is what a conformal save does to indices
  ([GUID and generations](PACKAGE_FORMAT.md#guid-and-generations)).
- The cost is dead bytes: the old name table, the old tables and the old
  copies of changed exports. For `LODSet` on a texture package, that's a
  few bytes per texture in a new copy of each one's mipmaps, which roughly
  doubles the texture data that's edited. `--compact` (way B) gets rid of it.
- The file no longer has the canonical layout
  ([File layout](PACKAGE_FORMAT.md#file-layout)). The engine only ever
  seeks by offset, so this should load. **That needs checking in the game
  and in UnrealEd before relying on it**, along with what other tools
  (UEViewer, UTPT) make of it.

This works with nothing more than the readers that exist today (textures,
sounds, palettes, script), so it can come first.

### B. A full rewrite (canonical layout; implemented)

Write the package as `UObject::SavePackage` does (`Core/Src/UnObj.cpp`):
summary, names, every export's data in table order, imports, exports, then
the summary again with the real offsets. Every export after the first
change moves, so **every class in the package that holds absolute offsets
needs a reader and a writer**, and every class whose native data usagi
can't read has to be known to hold none. This is the reason for the
[class census](#class-census).

Unlike the engine, usagi keeps the order of the names, imports and exports
rather than sorting them again (the engine sorts names by use and exports by
class), so indices don't change and unchanged export data can be copied
rather than re-serialized, unless it holds absolute offsets.

## Round-tripping is the test

Both ways rest on the same check: **read and write with no changes gives
the same bytes.**

- Per export: read its native data and write it again with
  `Saver::at(its own offset)`. The bytes must be identical. This runs over
  every object in a corpus, class by class, and is how each new reader is
  accepted.
- Per package (way B): write the whole package with no edits. The output
  must equal the input, byte for byte, for every package in the corpus.
  UT2004's packages all have the canonical layout, so this should hold,
  unless the engine pads or aligns something we don't know about yet, which
  is exactly what we want this check to find.
- After an edit: read the result back, check that every export still reads,
  and check that the edited properties read back as set.

`usagi pkg classes --round-trip` runs all of these but the last (there's no
editing yet) over a corpus. For each object, the native data is also written
at another offset, read back from there and written where it was, which must
give the original bytes again: the check that moving an object works, since
a package written with no changes moves nothing. Every object and package in
both installs passes.

## Class census

The plan is to write a reader and writer for each native class we meet
**before** we need it. That makes the round-trip check cover the whole
corpus, and shows up odd cases (licensee changes, old versions, padding)
early rather than in the middle of a feature.

Only classes whose C++ overrides `Serialize` have native data of their own;
a script class (an actor placed in a map, a `Shader`) has its native parent's
layout. The classes that override it in the UT2004 v3369 source and are
saved in packages (leaving out the transient ones: linkers, engines,
viewports, net drivers, factories, exporters):

- **Core:** `UObject`, `UField`, `UStruct`, `UState`, `UClass`,
  `UFunction`, `UConst`, `UEnum`, `UTextBuffer`, `UPackage`, and the
  property classes (`UProperty` and its subclasses).
- **Materials and sound:** `UMaterial`, `UTexture`, `UPalette`, `UFont`,
  `UMaterialSequence`, `UFireTexture` (Fire), `USound`, `USoundGroup`,
  `UProceduralSound`.
- **Geometry:** `UModel`, `UPolys`, `UPrimitive`, `UStaticMesh`,
  `UStaticMeshInstance`, `UMesh`, `ULodMesh`, `UVertMesh`, `USkeletalMesh`,
  `UMeshAnimation`, `UAnimation`, `UKMeshProps`, `UConvexVolume`,
  `UFluidSurfacePrimitive`, the vertex streams (`UVertexStreamBase`,
  `...COLOR`, `...PosNormTex`, `...UV`, `...VECTOR`), `UVertexBuffer`,
  `UIndexBuffer`, `USkinVertexBuffer`, `URenderResource`.
- **Levels:** `ULevelBase`, `ULevel`, `AActor`, `ATerrainInfo`,
  `UTerrainSector`, `UTerrainPrimitive`, `UPrefab`.

Licensees add their own: the census has to be redone for each game, and an
object whose class has native data but no registered reader is the thing to
look for.

### Finding the classes in a corpus

`pkg list --imports --exports --format json` gives each export's
`class_index`, which is an import (negative) or an export (positive) with
its full `path`. A script to count objects and bytes by class:

```sh
cd <game root>
find . -iname '*.u' -o -iname '*.utx' -o -iname '*.usx' -o -iname '*.ukx' \
       -o -iname '*.uax' -o -iname '*.umx' -o -iname '*.rom' -o -iname '*.ut2' |
while read -r p; do usagi pkg list --imports --exports --no-summary --format json "$p"; done |
jq -r '. as $p | .exports[] |
  (if .class_index == 0 then "Core.Class"
   elif .class_index < 0 then $p.imports[-.class_index - 1].path
   else $p.exports[.class_index - 1].path end) + "\t" + (.serial_size | tostring)' |
awk -F'\t' '{n[$1]++; s[$1]+=$2} END {for (c in n) print n[c] "\t" s[c] "\t" c}' |
sort -t$'\t' -k2 -nr
```

This is worth having as a command (see [CLI](#cli)), since it should also
say how many objects of each class have native data (`pkg obj` already
works out `native_size`) and whether usagi reads it.

`usagi pkg classes` does this, and says which native class stores each
class's data and, with `--round-trip`, how many objects of each write back
the same.

### Red Orchestra with Darkest Hour

One run over a Red Orchestra install with Darkest Hour's development tree
(1,692 package files, all version 128, licensee 29, with Tripwire's tag)
found 1,331 classes. Maps are counted twice where the development tree has
its own copy. The native classes, by bytes:

| Class | Objects | Bytes | In | Reader |
|---|---:|---:|---|---|
| `Engine.Texture` | 25,102 | 13.5 GB | `.utx` `.usx` `.ukx` `.rom` `.u` | yes |
| `Engine.Sound` | 10,471 | 3.9 GB | `.uax` `.usx` `.rom` `.u` | yes |
| `Engine.TerrainInfo` | 595 | 3.8 GB | `.rom` | yes |
| `Engine.StaticMeshInstance` | 2,516,481 | 3.6 GB | `.rom` | yes |
| `Engine.Model` | 396,951 | 2.4 GB | `.rom` `.usx` `.utx` `.ukx` | yes |
| `Engine.StaticMesh` | 19,618 | 2.3 GB | `.usx` `.rom` `.utx` `.u` | yes |
| `Engine.SkeletalMesh` | 1,739 | 1.2 GB | `.ukx` `.utx` | yes |
| `Engine.Polys` | 396,937 | 343 MB | `.rom` `.usx` `.utx` `.ukx` | yes |
| `Engine.MeshAnimation` | 798 | 250 MB | `.ukx` `.utx` | yes |
| `Engine.TerrainSector` | 371,941 | 59 MB | `.rom` | yes |
| `Core.TextBuffer` | 12,536 | 20 MB | `.u` `.rom` `.uax` | yes |
| `Engine.KMeshProps` | 6,418 | 12 MB | all but `.uax` | yes |
| `Engine.Level` | 453 | 7.8 MB | `.rom` | yes |
| `Engine.Font` | 245 | 4.5 MB | `.utx` `.u` | yes |
| `Core.Class`, `Core.Function`, `Core.State`, properties, ... | ~140,000 | 8 MB | `.u` `.rom` `.uax` | yes |
| `Engine.ConvexVolume` | 2,225 | 1.2 MB | `.rom` | yes |
| `Engine.Palette` | 170 | 175 KB | | yes |
| `Engine.MaterialSequence` | 111 | 17 KB | | none (`Material`'s) |
| `Engine.SoundGroup` | 2,280 | 25 KB | `.uax` | yes |
| `Engine.Cubemap` | 58 | 3 KB | | `Texture`'s |
| `Engine.ProceduralSound` | 2 | 46 B | `.uax` | yes |

Most of the other classes are actors in maps (`StaticMeshActor`, 2.5
million; `Brush`, `Light`, `PlayerStart`, ...), which have `AActor`'s
native data (none) and their properties. None of `Fire`'s classes and no
`VertMesh` turned up. With every reader in place, every object of every one
of the 1,331 classes writes back the same, but for 22 tiny objects (two
copies of 11) whose classes are `UnrealEd` editor options that Red
Orchestra's `UnrealEd.u` doesn't have.

### UT2004

The 840 packages of a UT2004 install (OldUnreal's) have the same native
classes, and also 24 vertex meshes (`Engine.VertMesh`, 7.3 MB) and 12 fire
textures (`Fire.FireTexture`), which usagi reads too. Every object and
package writes back the same, with the same exception (one copy of the
`UnrealEd` options).

### Order

*Done: every class in both lists has a reader. Kept for the record.*

By what unblocks the most, then by bytes:

1. **`StaticMesh`, `Model`/`Polys`, `SkeletalMesh`, `MeshAnimation`.** The
   lazy-array classes and nearly all the remaining bytes. With textures
   and sounds, they make way B possible for every texture, sound, static
   mesh and animation package.
2. **`KMeshProps`, `Font`, `MaterialSequence`, `ConvexVolume`, `AActor`,
   `Level`, `TerrainInfo`, `TerrainSector`, `StaticMeshInstance`.** The
   rest of what's in maps.
3. **Anything a licensee adds**, found by the census.

Each one gets a type per C++ structure in `usagi-pkg` (see [Native
data](ASSET_PIPELINE.md#1-native-data-usagi-pkg)), a section in
PACKAGE_FORMAT.md citing the engine source, and a pass of the per-export
round trip over the corpus before it's accepted. Tripwire's licensee
version (29) is the same as UT2004's, but its tag isn't, and its classes
may serialize differently from the UT2004 source. The round trip is what
will tell.

## Decisions

The open questions, settled:

- **Way A or B.** B only. Every class in both corpora has a reader, so every
  package can be rewritten in the engine's own layout, which is known to
  load, with no dead bytes. A package with a class whose native data isn't
  read, or can't be found, can still be written as long as none of those
  objects moves; editing it in a way that moves one is refused.
- **Order of names, imports and exports.** Kept, with new ones appended, so
  every index in unchanged data stays right.
- **Version floors.** The readers implement every version of a layout that
  only adds fields, and refuse the old layouts the engine converts as it
  loads them (static meshes before version 112, BSP models before 110, ...;
  see [PACKAGE_FORMAT.md](PACKAGE_FORMAT.md#which-native-data-an-object-has)).
  Nothing in either install is that old, and the engine never writes those
  layouts.
- **What the engine stamps when saving.** `UStaticMesh` and `ULodMesh` write
  the engine's own `InternalVersion` constant when saving, whatever was
  loaded; usagi writes back what it read, so that unchanged data is
  unchanged.
- **GUID.** Kept by default, with `--new-guid` to opt out, when `pkg set`
  comes.
- **Generations.** An edit that appends names or exports also appends a
  generation (the new counts), as a conformal save does. With the GUID
  kept, that's what lets a client with the old build agree on the old
  generation and keep using the indices both builds share.
- **Defaults, declarations, `.uz2` and safety.** As below: a property set to
  its class's default is dropped; a property whose declaration can't be
  found is refused; a `.uz2` is decompressed, edited and compressed; and a
  file is replaced only after the new one has been written to a temporary
  file and read back.

## Other questions

- **GUID.** Clients and servers compare packages by GUID and generation.
  Keeping the GUID means a changed package can be mistaken for the old one
  online; a new one breaks anything cached or compiled against the old.
  For an edit like `LODSet`, keeping it seems expected, with `--new-guid`
  to opt out. If names or exports are appended, should the edit also add a
  generation, as a conformal save does?
- **Defaults.** Setting a property to its class's default should drop the
  tag, as the engine would on save. This needs the class's defaults, which
  `Packages` reads but doesn't yet combine with an object's own properties
  ([Object context](ASSET_PIPELINE.md#0-object-context-usagi-pkg)).
- **Declarations.** Writing a tag needs its type, size and, for an enum, the
  enum's values, all from the declaration. A property whose declaration
  can't be found is refused rather than guessed.
- **`.uz2`.** Decompress, edit, compress, as `set_package_flags` does.
- **Safety.** Write to a temporary file and rename it, read the result back
  before replacing anything, and never write to a game install that isn't a
  scratch copy.

## CLI

- `usagi pkg set <PATHS>... [OBJECTS]... NAME=VALUE...`, with `--class`,
  `--reset NAME`, `--dry-run`, `-o DIR`, `--compact`, and later
  `--from FILE`. `FILE` would be the TOML or JSON that `pkg obj` and
  `pkg export --properties` write, with only the fields to change.
- `usagi pkg classes <PATHS>...` (implemented; see
  [CLI.md](CLI.md#usagi-pkg-classes-paths)): objects and bytes per class,
  the native class that stores their data and whether usagi reads it; with
  `--round-trip`, how many of each class write back identically, and which
  packages don't.
