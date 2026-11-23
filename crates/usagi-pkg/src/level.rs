//! Levels' and terrain's native data: `ULevelBase` and `ULevel`
//! (`Engine/Inc/UnLevel.h`, `Engine/Src/UnLevel.cpp`), and `ATerrainInfo`,
//! `UTerrainSector` and `UTerrainPrimitive` (`Engine/Inc/UnTerrain.h`,
//! `Engine/Src/UnTerrain.cpp`).

use crate::archive::{Archive, FString, Serialize, native_struct};
use crate::math::{BoundingBox, Color, Coords, Vector};
use crate::mesh::{Primitive, at_least};
use crate::model::db_array;
use crate::{ObjectRef, Result};

native_struct! {
    /// `FURL`: where a level was loaded from.
    pub struct Url {
        pub protocol: FString,
        pub host: FString,
        pub map: FString,
        pub portal: FString,
        pub op: Vec<FString>,
        pub port: i32,
        pub valid: i32,
    }
}

/// The first version whose levels have the layout that's read. Before it,
/// a level has its old reach specs, which the engine drops.
pub const LEVEL_VERSION: u16 = 98;

/// The number of text buffers a level has (`NUM_LEVEL_TEXT_BLOCKS`).
pub const LEVEL_TEXT_BLOCKS: usize = 16;

native_struct! {
    /// A pair of a `TMap<FString,FString>`.
    pub struct StringPair {
        pub key: FString,
        pub value: FString,
    }
}

/// A level's native data (`ULevelBase::Serialize` and `ULevel::Serialize`),
/// after its tagged properties, from version [`LEVEL_VERSION`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Level {
    /// The size the actor list had in memory, stored after the number of
    /// actors. The engine writes the number again.
    pub actors_max: i32,
    pub actors: Vec<ObjectRef>,
    pub url: Url,
    pub model: ObjectRef,
    /// `TimeSeconds`.
    pub approx_time: f32,
    pub first_deleted: ObjectRef,
    pub text_blocks: [ObjectRef; LEVEL_TEXT_BLOCKS],
    pub travel_info: Vec<StringPair>,
}

impl Serialize for Level {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        at_least(ar, LEVEL_VERSION, "levels")?;
        db_array(ar, &mut self.actors_max, &mut self.actors, "actors")?;
        self.url.serialize(ar)?;
        self.model.serialize(ar)?;
        self.approx_time.serialize(ar)?;
        self.first_deleted.serialize(ar)?;
        self.text_blocks.serialize(ar)?;
        self.travel_info.serialize(ar)
    }
}

/// The first version whose terrain has the layout that's read, with vertex
/// colours. The engine converts older terrain as it loads it.
pub const TERRAIN_VERSION: u16 = 117;

native_struct! {
    /// `FTerrainNormalPair`: the normals of a quad's two triangles.
    pub struct TerrainNormalPair {
        pub normal1: Vector,
        pub normal2: Vector,
    }
}

/// A terrain's native data (`ATerrainInfo::Serialize`), after its tagged
/// properties, from version [`TERRAIN_VERSION`]. A `TerrainInfo` is an
/// actor; actors have no native data of their own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerrainInfo {
    pub sectors: Vec<ObjectRef>,
    pub vertices: Vec<Vector>,
    pub sectors_x: i32,
    pub sectors_y: i32,
    pub face_normals: Vec<TerrainNormalPair>,
    pub to_world: Coords,
    pub to_heightmap: Coords,
    pub heightmap_x: i32,
    pub heightmap_y: i32,
    pub vertex_colors: Vec<Color>,
}

impl Serialize for TerrainInfo {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        at_least(ar, TERRAIN_VERSION, "terrain")?;
        self.sectors.serialize(ar)?;
        self.vertices.serialize(ar)?;
        self.sectors_x.serialize(ar)?;
        self.sectors_y.serialize(ar)?;
        self.face_normals.serialize(ar)?;
        self.to_world.serialize(ar)?;
        self.to_heightmap.serialize(ar)?;
        self.heightmap_x.serialize(ar)?;
        self.heightmap_y.serialize(ar)?;
        self.vertex_colors.serialize(ar)
    }
}

native_struct! {
    /// `FTerrainSectorLightInfo`: a light that lights a terrain sector, and
    /// which of its vertices it reaches.
    pub struct TerrainSectorLightInfo {
        pub light_actor: ObjectRef,
        pub visibility_bitmap: Vec<u8>,
    }
}

/// A terrain sector's native data (`UTerrainSector::Serialize`), after its
/// tagged properties, from version [`TERRAIN_VERSION`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerrainSector {
    pub info: ObjectRef,
    pub quads_x: i32,
    pub quads_y: i32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub bounding_box: BoundingBox,
    pub light_infos: Vec<TerrainSectorLightInfo>,
}

impl Serialize for TerrainSector {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        at_least(ar, TERRAIN_VERSION, "terrain sectors")?;
        self.info.serialize(ar)?;
        self.quads_x.serialize(ar)?;
        self.quads_y.serialize(ar)?;
        self.offset_x.serialize(ar)?;
        self.offset_y.serialize(ar)?;
        self.bounding_box.serialize(ar)?;
        self.light_infos.serialize(ar)
    }
}

native_struct! {
    /// A terrain's collision primitive (`UTerrainPrimitive::Serialize`),
    /// after its tagged properties.
    pub struct TerrainPrimitive {
        pub primitive: Primitive,
        pub info: ObjectRef,
    }
}
