//! Textures', palettes', fonts' and sounds' native data
//! (`UTexture::Serialize`, `UPalette::Serialize` in `Engine/Src/UnTex.cpp`,
//! `UFont::Serialize` in `Engine/Src/UnFont.cpp`, and `USound::Serialize`
//! in `Engine/Src/UnAudio.cpp`).

use crate::archive::{Archive, FString, LazyArray, NameRef, Serialize, native_struct};
use crate::math::Color;
use crate::{Error, ObjectRef, Package, Result};

/// The first version in which every texture has one array of mipmaps.
/// Before it, a texture with `bHasComp` has two (`OLDbHasComp`).
pub const TEXTURE_MIPS_VERSION: u16 = 84;

/// A mipmap (`FMipmap`, `Engine/Inc/UnTex.h`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mipmap {
    /// The pixels, in the texture's `Format`.
    pub data: LazyArray<u8>,
    pub u_size: i32,
    pub v_size: i32,
    /// The base-2 logarithm of `u_size`.
    pub u_bits: u8,
    /// The base-2 logarithm of `v_size`.
    pub v_bits: u8,
}

impl Serialize for Mipmap {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        self.data.serialize(ar)?;
        ar.i32(&mut self.u_size, "mipmap")?;
        ar.i32(&mut self.v_size, "mipmap")?;
        ar.u8(&mut self.u_bits, "mipmap")?;
        ar.u8(&mut self.v_bits, "mipmap")
    }
}

/// A texture's native data, after its tagged properties: its mipmaps,
/// largest first (`SerializeMips`). From version
/// [`TEXTURE_MIPS_VERSION`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Texture {
    pub mips: Vec<Mipmap>,
}

impl Serialize for Texture {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        ar.array_of(&mut self.mips, "texture's mipmaps")
    }
}

impl Package {
    /// Export `export`'s data from `offset` (where its tagged properties
    /// end) as a texture's. Returns it and where it ends: a subclass of
    /// `Texture` may have more after it.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidData`] if the package is older than
    /// [`TEXTURE_MIPS_VERSION`], or the data is malformed.
    ///
    /// # Panics
    ///
    /// If `export` is out of range.
    pub fn texture(&self, export: usize, offset: usize) -> Result<(Texture, usize)> {
        if self.summary.version() < TEXTURE_MIPS_VERSION {
            return Err(Error::InvalidData {
                offset,
                reason: format!(
                    "textures before version {} aren't read",
                    TEXTURE_MIPS_VERSION
                ),
            });
        }
        self.read_native(export, offset)
    }
}

/// A sound's native data, after its tagged properties.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sound {
    /// The format of `data`, such as `WAV`.
    pub file_type: NameRef,
    /// From licensee version 2.
    pub likelihood: Option<f32>,
    /// The sound file.
    pub data: LazyArray<u8>,
}

impl Serialize for Sound {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        ar.name(&mut self.file_type, "sound")?;
        if ar.licensee_version() >= 2 {
            ar.f32(self.likelihood.get_or_insert(0.0), "sound")?;
        }
        self.data.serialize(ar)
    }
}

/// The licensee version from which a sound group stores its sounds, rather
/// than the package they're in.
pub const SOUND_GROUP_SOUNDS_LICENSEE_VERSION: u16 = 0x1B;

/// A sound group's native data (`USoundGroup::Serialize`), after its tagged
/// properties. It doesn't start with a sound's: the engine skips
/// `USound::Serialize`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoundGroup {
    /// Before licensee version [`SOUND_GROUP_SOUNDS_LICENSEE_VERSION`]: the
    /// package its sounds are in.
    pub package: Option<FString>,
    /// From licensee version [`SOUND_GROUP_SOUNDS_LICENSEE_VERSION`]: its
    /// sounds.
    pub sounds: Option<Vec<ObjectRef>>,
}

impl Serialize for SoundGroup {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        if ar.licensee_version() < SOUND_GROUP_SOUNDS_LICENSEE_VERSION {
            self.package
                .get_or_insert_with(FString::default)
                .serialize(ar)
        } else {
            ar.array_of(
                self.sounds.get_or_insert_with(Vec::new),
                "sound group's sounds",
            )
        }
    }
}

native_struct! {
    /// A palette's native data (`UPalette::Serialize`), after its tagged
    /// properties. Before version 66, the engine sets every alpha to 255
    /// when it loads them.
    pub struct Palette {
        pub colors: Vec<Color>,
    }
}

/// The licensee version from which a font stores its characters and
/// textures, rather than pages of them.
pub const FONT_CHARACTERS_LICENSEE_VERSION: u16 = 0x1D;

native_struct! {
    /// A character of a font (`FFontCharacter`, `Engine/Inc/UnTex.h`): where
    /// it is in its texture.
    pub struct FontCharacter {
        pub start_u: i32,
        pub start_v: i32,
        pub u_size: i32,
        pub v_size: i32,
        /// Which of the font's textures it's in.
        pub texture_index: u8,
    }
}

native_struct! {
    /// A character of a font page (`FOLDFontCharacter`), before licensee
    /// version [`FONT_CHARACTERS_LICENSEE_VERSION`].
    pub struct OldFontCharacter {
        pub start_u: i32,
        pub start_v: i32,
        pub u_size: i32,
        pub v_size: i32,
    }
}

native_struct! {
    /// A font page (`FOLDFontPage`): a texture and its characters.
    pub struct OldFontPage {
        pub texture: ObjectRef,
        pub characters: Vec<OldFontCharacter>,
    }
}

native_struct! {
    /// A pair of a `TMap<TCHAR,TCHAR>`: a character and the one it's drawn
    /// as. A `TCHAR` is a UTF-16 code unit.
    pub struct CharRemap {
        pub from: u16,
        pub to: u16,
    }
}

/// A font's native data (`UFont::Serialize`), after its tagged properties.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Font {
    /// Before licensee version [`FONT_CHARACTERS_LICENSEE_VERSION`]: its
    /// pages, and then `characters_per_page`.
    pub pages: Option<Vec<OldFontPage>>,
    pub characters_per_page: Option<i32>,
    /// From licensee version [`FONT_CHARACTERS_LICENSEE_VERSION`]: its
    /// characters, and the textures they're in.
    pub characters: Option<Vec<FontCharacter>>,
    pub textures: Option<Vec<ObjectRef>>,
    pub kerning: i32,
    /// From version 69: characters drawn as others, and whether there are
    /// any.
    pub char_remap: Option<Vec<CharRemap>>,
    pub is_remapped: Option<i32>,
}

impl Serialize for Font {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        if ar.licensee_version() < FONT_CHARACTERS_LICENSEE_VERSION {
            self.pages.get_or_insert_with(Vec::new).serialize(ar)?;
            self.characters_per_page.get_or_insert(0).serialize(ar)?;
        } else {
            self.characters.get_or_insert_with(Vec::new).serialize(ar)?;
            self.textures.get_or_insert_with(Vec::new).serialize(ar)?;
        }
        self.kerning.serialize(ar)?;
        if ar.version() >= 69 {
            self.char_remap.get_or_insert_with(Vec::new).serialize(ar)?;
            self.is_remapped.get_or_insert(0).serialize(ar)?;
        }
        Ok(())
    }
}

native_struct! {
    /// A procedural sound's native data (`UProceduralSound::Serialize`),
    /// after its tagged properties. Like a sound group, it doesn't start
    /// with a sound's.
    pub struct ProceduralSound {
        /// `USound::Likelihood`.
        pub likelihood: f32,
        pub base_sound: ObjectRef,
        pub pitch_modification: f32,
        pub volume_modification: f32,
        pub pitch_variance: f32,
        pub volume_variance: f32,
    }
}

native_struct! {
    /// A spark of a fire texture (`FSpark`, `Fire/Inc/UnFractal.h`).
    pub struct Spark {
        pub spark_type: u8,
        pub heat: u8,
        pub x: u8,
        pub y: u8,
        pub byte_a: u8,
        pub byte_b: u8,
        pub byte_c: u8,
        pub byte_d: u8,
    }
}

/// A fire texture's native data (`UFireTexture::Serialize`,
/// `Fire/Src/UnFractal.cpp`), after its tagged properties: a texture's,
/// then its sparks. Its mipmaps are empty, as a procedural texture's are.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FireTexture {
    pub texture: Texture,
    pub sparks: Vec<Spark>,
}

impl Serialize for FireTexture {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        self.texture.serialize(ar)?;
        self.sparks.serialize(ar)
    }
}

native_struct! {
    /// A prefab's native data (`UPrefab::Serialize`,
    /// `Editor/Src/UnPrefab.cpp`), after its tagged properties: actors as
    /// T3D text.
    pub struct Prefab {
        pub file_type: NameRef,
        pub t3d_text: FString,
    }
}
