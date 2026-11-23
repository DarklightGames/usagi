//! BSP models' and brushes' polygons' native data: `UModel` and `UPolys`
//! (`Engine/Inc/UnModel.h`, `Engine/Inc/UnObj.h`, `Engine/Src/UnModel.cpp`).

use crate::archive::{Archive, Index, LazyArray, NameRef, Serialize, native_struct};
use crate::math::{BoundingBox, Matrix, Plane, Sphere, Vector};
use crate::mesh::{Primitive, at_least};
use crate::{Error, ObjectRef, Result};

/// The first version whose models have the layout that's read, with their
/// lightmaps in textures. The engine converts older ones as it loads them.
pub const MODEL_VERSION: u16 = 110;

/// The most zones a model can have (`FBspNode::MAX_ZONES`).
pub const MAX_ZONES: usize = 64;

/// The most vertices a polygon can have (`FPoly::MAX_VERTICES`).
pub const MAX_POLY_VERTICES: usize = 16;

/// `FBspNode`, from version [`MODEL_VERSION`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BspNode {
    pub plane: Plane,
    pub zone_mask: u64,
    pub node_flags: u8,
    pub vert_pool: Index,
    pub surf: Index,
    /// Back, front and coplanar.
    pub children: [Index; 3],
    pub collision_bound: Index,
    pub render_bound: Index,
    pub exclusive_sphere_bound: Sphere,
    /// Before version 123.
    pub inclusive_sphere_bound: Option<Sphere>,
    pub zones: [u8; 2],
    pub num_vertices: u8,
    pub leaves: [i32; 2],
    pub section: i32,
    pub first_vertex: i32,
    pub light_map: i32,
}

impl Serialize for BspNode {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        self.plane.serialize(ar)?;
        self.zone_mask.serialize(ar)?;
        self.node_flags.serialize(ar)?;
        self.vert_pool.serialize(ar)?;
        self.surf.serialize(ar)?;
        self.children.serialize(ar)?;
        self.collision_bound.serialize(ar)?;
        self.render_bound.serialize(ar)?;
        self.exclusive_sphere_bound.serialize(ar)?;
        if ar.version() < 123 {
            self.inclusive_sphere_bound
                .get_or_insert_with(Sphere::default)
                .serialize(ar)?;
        }
        self.zones.serialize(ar)?;
        self.num_vertices.serialize(ar)?;
        self.leaves.serialize(ar)?;
        self.section.serialize(ar)?;
        self.first_vertex.serialize(ar)?;
        self.light_map.serialize(ar)
    }
}

native_struct! {
    /// `FBspSurf`, from version [`MODEL_VERSION`].
    pub struct BspSurf {
        pub material: ObjectRef,
        pub poly_flags: u32,
        pub base: Index,
        pub normal: Index,
        pub texture_u: Index,
        pub texture_v: Index,
        pub brush_poly: Index,
        pub actor: ObjectRef,
        pub plane: Plane,
        pub light_map_scale: f32,
    }
}

native_struct! {
    /// `FVert`: a vertex of a node.
    pub struct Vert {
        pub vertex: Index,
        /// The side it shares, or -1.
        pub side: Index,
    }
}

native_struct! {
    /// `FZoneProperties`.
    pub struct ZoneProperties {
        pub zone_actor: ObjectRef,
        pub connectivity: u64,
        pub visibility: u64,
        pub last_render_time: f32,
    }
}

native_struct! {
    /// `FLeaf`.
    pub struct Leaf {
        pub zone: Index,
        pub permeating: Index,
        pub volumetric: Index,
        pub visible_zones: u64,
    }
}

native_struct! {
    /// `FBspVertex`, from version [`MODEL_VERSION`].
    pub struct BspVertex {
        pub position: Vector,
        pub u: f32,
        pub v: f32,
        pub u2: f32,
        pub v2: f32,
        pub normal: Vector,
    }
}

native_struct! {
    /// `FBspVertexStream`.
    pub struct BspVertexStream {
        pub vertices: Vec<BspVertex>,
        pub revision: i32,
    }
}

native_struct! {
    /// `FBspSection`: nodes with the same material.
    pub struct BspSection {
        pub vertices: BspVertexStream,
        pub material: ObjectRef,
        pub num_nodes: i32,
        pub poly_flags: u32,
        pub light_map_texture: i32,
    }
}

native_struct! {
    /// `FLightBitmap`: where a light reaches a lightmap.
    pub struct LightBitmap {
        pub light_actor: ObjectRef,
        pub bits: Vec<u8>,
        pub size_x: i32,
        pub size_y: i32,
        pub stride: i32,
        pub min_x: i32,
        pub min_y: i32,
        pub max_x: i32,
        pub max_y: i32,
    }
}

native_struct! {
    /// `FLightMap`: a surface's lightmap.
    pub struct LightMap {
        pub texture: Index,
        pub surf: Index,
        pub zone: Index,
        pub offset_x: Index,
        pub offset_y: Index,
        pub size_x: Index,
        pub size_y: Index,
        pub world_to_light_map: Matrix,
        pub light_map_base: Vector,
        pub light_map_x: Vector,
        pub light_map_y: Vector,
        pub bitmaps: Vec<LightBitmap>,
        pub level: ObjectRef,
        pub revision: i32,
    }
}

native_struct! {
    /// `FStaticLightMapTexture`: lightmaps' texels, in two mipmaps.
    pub struct StaticLightMapTexture {
        pub data: [LazyArray<u8>; 2],
        pub format: u8,
        pub width: i32,
        pub height: i32,
        pub revision: i32,
    }
}

/// `FLightMapTexture`: lightmaps put together in a texture.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LightMapTexture {
    pub level: ObjectRef,
    pub light_maps: Vec<i32>,
    pub cache_id: u64,
    pub revision: i32,
    /// From version 116.
    pub static_texture: Option<StaticLightMapTexture>,
}

impl Serialize for LightMapTexture {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        self.level.serialize(ar)?;
        self.light_maps.serialize(ar)?;
        self.cache_id.serialize(ar)?;
        self.revision.serialize(ar)?;
        if ar.version() >= 116 {
            self.static_texture
                .get_or_insert_with(StaticLightMapTexture::default)
                .serialize(ar)?;
        }
        Ok(())
    }
}

/// A BSP model's native data (`UModel::Serialize`), after its tagged
/// properties, from version [`MODEL_VERSION`]: a level's geometry, or a
/// brush's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub primitive: Primitive,
    pub vectors: Vec<Vector>,
    pub points: Vec<Vector>,
    pub nodes: Vec<BspNode>,
    pub surfs: Vec<BspSurf>,
    pub verts: Vec<Vert>,
    pub num_shared_sides: i32,
    /// As stored; the number of `zones`, if it isn't negative.
    pub num_zones: i32,
    pub zones: Vec<ZoneProperties>,
    pub polys: ObjectRef,
    pub bounds: Vec<BoundingBox>,
    pub leaf_hulls: Vec<i32>,
    pub leaves: Vec<Leaf>,
    pub lights: Vec<ObjectRef>,
    pub root_outside: i32,
    pub linked: i32,
    pub sections: Vec<BspSection>,
    pub light_maps: Vec<LightMap>,
    pub light_map_textures: Vec<LightMapTexture>,
}

impl Serialize for Model {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        at_least(ar, MODEL_VERSION, "models")?;
        self.primitive.serialize(ar)?;
        self.vectors.serialize(ar)?;
        self.points.serialize(ar)?;
        self.nodes.serialize(ar)?;
        self.surfs.serialize(ar)?;
        self.verts.serialize(ar)?;
        self.num_shared_sides.serialize(ar)?;
        let at = ar.pos();
        self.num_zones.serialize(ar)?;
        if ar.is_loading() {
            let count = self.num_zones.max(0) as usize;
            if count > MAX_ZONES {
                return Err(Error::InvalidData {
                    offset: at,
                    reason: format!("a model has {} zones", self.num_zones),
                });
            }
            self.zones = vec![ZoneProperties::default(); count];
        }
        for zone in &mut self.zones {
            zone.serialize(ar)?;
        }
        self.polys.serialize(ar)?;
        self.bounds.serialize(ar)?;
        self.leaf_hulls.serialize(ar)?;
        self.leaves.serialize(ar)?;
        self.lights.serialize(ar)?;
        self.root_outside.serialize(ar)?;
        self.linked.serialize(ar)?;
        self.sections.serialize(ar)?;
        self.light_maps.serialize(ar)?;
        self.light_map_textures.serialize(ar)
    }
}

/// `FPoly`: a polygon of a brush.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Poly {
    pub base: Vector,
    pub normal: Vector,
    pub texture_u: Vector,
    pub texture_v: Vector,
    /// Stored after the number of them.
    pub vertices: Vec<Vector>,
    pub poly_flags: u32,
    pub actor: ObjectRef,
    pub material: ObjectRef,
    pub item_name: NameRef,
    pub link: Index,
    pub brush_poly: Index,
    /// Before version 78: `PanU` and `PanV`.
    pub pan: Option<[u16; 2]>,
    /// From version 106.
    pub light_map_scale: Option<f32>,
}

impl Serialize for Poly {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        let at = ar.pos();
        let mut count = Index(i32::try_from(self.vertices.len()).expect("a few vertices"));
        count.serialize(ar)?;
        if ar.is_loading() {
            let count = count.0.max(0) as usize;
            if count > MAX_POLY_VERTICES {
                return Err(Error::InvalidData {
                    offset: at,
                    reason: format!("a polygon has {} vertices", count),
                });
            }
            self.vertices = vec![Vector::default(); count];
        }
        self.base.serialize(ar)?;
        self.normal.serialize(ar)?;
        self.texture_u.serialize(ar)?;
        self.texture_v.serialize(ar)?;
        for vertex in &mut self.vertices {
            vertex.serialize(ar)?;
        }
        self.poly_flags.serialize(ar)?;
        self.actor.serialize(ar)?;
        self.material.serialize(ar)?;
        self.item_name.serialize(ar)?;
        self.link.serialize(ar)?;
        self.brush_poly.serialize(ar)?;
        if ar.version() < 78 {
            self.pan
                .get_or_insert_with(Default::default)
                .serialize(ar)?;
        }
        if ar.version() >= 106 {
            self.light_map_scale.get_or_insert(0.0).serialize(ar)?;
        }
        Ok(())
    }
}

/// A brush's polygons (`UPolys::Serialize`), after its tagged properties.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Polys {
    /// The size the array had in memory, stored after the number of
    /// polygons. The engine writes the number again.
    pub max: i32,
    pub elements: Vec<Poly>,
}

impl Serialize for Polys {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        db_array(ar, &mut self.max, &mut self.elements, "polygons")
    }
}

/// An old-format array (`DbNum` and `DbMax`, `i32`s, then the items), as
/// `UPolys` and `ULevelBase` store theirs.
pub(crate) fn db_array<T: Serialize + 'static>(
    ar: &mut dyn Archive,
    max: &mut i32,
    items: &mut Vec<T>,
    what: &'static str,
) -> Result<()> {
    let at = ar.pos();
    let mut num = i32::try_from(items.len()).expect("fewer than 2^31 items");
    num.serialize(ar)?;
    max.serialize(ar)?;
    let count = usize::try_from(num).map_err(|_| Error::InvalidData {
        offset: at,
        reason: format!("there are {} {}", num, what),
    })?;
    ar.items(items, count)
}
