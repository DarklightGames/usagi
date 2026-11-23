//! Meshes' native data: `UPrimitive`, `UStaticMesh` and
//! `UStaticMeshInstance` (`Engine/Inc/UnStaticMesh.h`,
//! `Engine/Src/UnStaticMesh.cpp`), `UKMeshProps` (`Engine/Inc/KTypes.h`),
//! `UConvexVolume` (`Engine/Inc/UnConvexVolume.h`), and `UMesh`, `ULodMesh`,
//! `USkeletalMesh`, `UVertMesh` and `UMeshAnimation` (`Engine/Inc/UnLodMesh.h`,
//! `Engine/Inc/UnSkeletalMesh.h`, `Engine/Src/UnLodMesh.cpp`,
//! `Engine/Src/UnSkeletalMesh.cpp`).

use crate::archive::{Archive, Index, LazyArray, NameRef, Serialize, native_struct};
use crate::math::{BoundingBox, Color, Coords, Matrix, Plane, Quat, Rotator, Sphere, Vector};
use crate::{Error, ObjectRef, Result};

native_struct! {
    /// `UPrimitive`: its bounds.
    pub struct Primitive {
        pub bounding_box: BoundingBox,
        pub bounding_sphere: Sphere,
    }
}

native_struct! {
    /// `FRawIndexBuffer`: vertex indices.
    pub struct RawIndexBuffer {
        pub indices: Vec<u16>,
        pub revision: i32,
    }
}

native_struct! {
    /// `FRawColorStream`: a colour per vertex.
    pub struct RawColorStream {
        pub colors: Vec<Color>,
        pub revision: i32,
    }
}

/// The first version whose static meshes have the layout that's read, with
/// their vertices in streams. The engine converts older ones as it loads
/// them.
pub const STATIC_MESH_VERSION: u16 = 112;

native_struct! {
    /// A section of a static mesh (`FStaticMeshSection`), from version
    /// [`STATIC_MESH_VERSION`]: triangles with the same material.
    pub struct StaticMeshSection {
        pub is_strip: i32,
        pub first_index: u16,
        pub min_vertex_index: u16,
        pub max_vertex_index: u16,
        pub num_triangles: u16,
        pub num_primitives: u16,
    }
}

native_struct! {
    /// `FStaticMeshVertex`, from version [`STATIC_MESH_VERSION`].
    pub struct StaticMeshVertex {
        pub position: Vector,
        pub normal: Vector,
    }
}

native_struct! {
    /// `FStaticMeshVertexStream`.
    pub struct StaticMeshVertexStream {
        pub vertices: Vec<StaticMeshVertex>,
        pub revision: i32,
    }
}

native_struct! {
    /// `FStaticMeshUV`.
    pub struct MeshUv {
        pub u: f32,
        pub v: f32,
    }
}

native_struct! {
    /// `FStaticMeshUVStream`: a texture coordinate per vertex.
    pub struct StaticMeshUvStream {
        pub uvs: Vec<MeshUv>,
        pub coordinate_index: i32,
        pub revision: i32,
    }
}

native_struct! {
    /// `FStaticMeshCollisionTriangle`, before version 126.
    pub struct StaticMeshCollisionTriangle {
        pub plane: Plane,
        pub side_planes: [Plane; 3],
        pub vertex_indices: [Index; 3],
        pub material_index: Index,
    }
}

native_struct! {
    /// `FStaticMeshCollisionNode`, before version 126.
    pub struct StaticMeshCollisionNode {
        pub triangle_index: Index,
        pub coplanar_index: Index,
        pub child_indices: [Index; 2],
        pub bounding_box: BoundingBox,
    }
}

native_struct! {
    /// `FkDOP` (`Engine/Inc/UnkDOP.h`): the minimum and maximum along each
    /// axis.
    pub struct KDop {
        pub min: [f32; 3],
        pub max: [f32; 3],
    }
}

native_struct! {
    /// `FkDOPNode`. Its last two values are a union: the left and right
    /// children's indices when `is_leaf` is set, and otherwise the number
    /// of triangles and the first's index (as the engine writes them,
    /// whatever their names say).
    pub struct KDopNode {
        pub bounding_volume: KDop,
        pub is_leaf: i32,
        pub a: u16,
        pub b: u16,
    }
}

native_struct! {
    /// `FkDOPCollisionTriangle`.
    pub struct KDopTriangle {
        pub vertices: [u16; 3],
        pub material_index: u16,
    }
}

native_struct! {
    /// `FkDOPTree`, from version 126: a static mesh's collision.
    pub struct KDopTree {
        pub nodes: Vec<KDopNode>,
        pub triangles: Vec<KDopTriangle>,
    }
}

/// `FStaticMeshTriangle`, from version [`STATIC_MESH_VERSION`]: a triangle
/// as it was imported.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StaticMeshTriangle {
    pub vertices: [Vector; 3],
    /// As stored; the number of `uvs`, if it isn't negative.
    pub num_uvs: i32,
    /// Each set of texture coordinates, for the three vertices.
    pub uvs: Vec<[MeshUv; 3]>,
    pub colors: [Color; 3],
    pub material_index: i32,
    pub smoothing_mask: u32,
}

impl Serialize for StaticMeshTriangle {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        self.vertices.serialize(ar)?;
        let at = ar.pos();
        self.num_uvs.serialize(ar)?;
        if ar.is_loading() {
            // The engine has room for 8.
            if self.num_uvs > 8 {
                return Err(Error::InvalidData {
                    offset: at,
                    reason: format!("a static mesh triangle has {} UVs", self.num_uvs),
                });
            }
            self.uvs = vec![<[MeshUv; 3]>::default(); self.num_uvs.max(0) as usize];
        }
        for uvs in &mut self.uvs {
            uvs.serialize(ar)?;
        }
        self.colors.serialize(ar)?;
        self.material_index.serialize(ar)?;
        self.smoothing_mask.serialize(ar)
    }
}

/// A static mesh's native data (`UStaticMesh::Serialize`), after its tagged
/// properties, from version [`STATIC_MESH_VERSION`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StaticMesh {
    pub primitive: Primitive,
    pub sections: Vec<StaticMeshSection>,
    /// `UPrimitive::BoundingBox` again.
    pub bounding_box: BoundingBox,
    pub vertex_stream: StaticMeshVertexStream,
    pub color_stream: RawColorStream,
    pub alpha_stream: RawColorStream,
    pub uv_streams: Vec<StaticMeshUvStream>,
    pub index_buffer: RawIndexBuffer,
    pub wireframe_index_buffer: RawIndexBuffer,
    pub collision_model: ObjectRef,
    /// Before version 126.
    pub collision_triangles: Option<Vec<StaticMeshCollisionTriangle>>,
    pub collision_nodes: Option<Vec<StaticMeshCollisionNode>>,
    /// From version 126.
    pub kdop_tree: Option<KDopTree>,
    /// Before version 114: `UseSimpleLineCollision`, `UseSimpleBoxCollision`
    /// and `UseVertexColor`.
    pub use_simple_collision: Option<[i32; 3]>,
    pub raw_triangles: LazyArray<StaticMeshTriangle>,
    pub internal_version: i32,
    /// In licensee version 22 (`0x16`) only.
    pub smoothing_threshold: Option<f32>,
    /// From licensee version 23.
    pub max_smoothing_angles: Option<Vec<f32>>,
    pub k_physics_props: ObjectRef,
    /// From version 120.
    pub authentication_key: Option<u32>,
}

impl Serialize for StaticMesh {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        at_least(ar, STATIC_MESH_VERSION, "static meshes")?;
        self.primitive.serialize(ar)?;
        self.sections.serialize(ar)?;
        self.bounding_box.serialize(ar)?;
        self.vertex_stream.serialize(ar)?;
        self.color_stream.serialize(ar)?;
        self.alpha_stream.serialize(ar)?;
        self.uv_streams.serialize(ar)?;
        self.index_buffer.serialize(ar)?;
        self.wireframe_index_buffer.serialize(ar)?;
        self.collision_model.serialize(ar)?;
        if ar.version() < 126 {
            self.collision_triangles
                .get_or_insert_with(Vec::new)
                .serialize(ar)?;
            self.collision_nodes
                .get_or_insert_with(Vec::new)
                .serialize(ar)?;
        } else {
            self.kdop_tree
                .get_or_insert_with(KDopTree::default)
                .serialize(ar)?;
        }
        if ar.version() < 114 {
            self.use_simple_collision
                .get_or_insert_with(Default::default)
                .serialize(ar)?;
        }
        self.raw_triangles.serialize(ar)?;
        self.internal_version.serialize(ar)?;
        match ar.licensee_version() {
            0x16 => self.smoothing_threshold.get_or_insert(0.0).serialize(ar)?,
            v if v > 0x16 => self
                .max_smoothing_angles
                .get_or_insert_with(Vec::new)
                .serialize(ar)?,
            _ => {}
        }
        self.k_physics_props.serialize(ar)?;
        if ar.version() >= 120 {
            self.authentication_key.get_or_insert(0).serialize(ar)?;
        }
        Ok(())
    }
}

native_struct! {
    /// `FStaticMeshLightInfo`: a light that lights a static mesh actor, and
    /// which of its vertices it reaches.
    pub struct StaticMeshLightInfo {
        pub light_actor: ObjectRef,
        pub visibility_bits: Vec<u8>,
        pub applied: i32,
    }
}

/// A static mesh actor's lighting (`UStaticMeshInstance::Serialize`),
/// after its tagged properties, from version [`STATIC_MESH_VERSION`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StaticMeshInstance {
    pub color_stream: RawColorStream,
    pub lights: Vec<StaticMeshLightInfo>,
}

impl Serialize for StaticMeshInstance {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        at_least(ar, STATIC_MESH_VERSION, "static mesh instances")?;
        self.color_stream.serialize(ar)?;
        self.lights.serialize(ar)
    }
}

native_struct! {
    /// `FKSphereElem`.
    pub struct KSphereElem {
        pub tm: Matrix,
        pub radius: f32,
    }
}

native_struct! {
    /// `FKBoxElem`: its lengths along each axis.
    pub struct KBoxElem {
        pub tm: Matrix,
        pub x: f32,
        pub y: f32,
        pub z: f32,
    }
}

native_struct! {
    /// `FKCylinderElem`.
    pub struct KCylinderElem {
        pub tm: Matrix,
        pub radius: f32,
        pub height: f32,
    }
}

native_struct! {
    /// `FKConvexElem`. Its planes aren't stored.
    pub struct KConvexElem {
        pub tm: Matrix,
        pub vertex_data: Vec<Vector>,
    }
}

/// The first version whose `FKAggregateGeom` has convex elements. Before
/// it, the spheres are stored twice; the engine keeps the second.
pub const K_CONVEX_VERSION: u16 = 113;

native_struct! {
    /// `FKAggregateGeom`, from version [`K_CONVEX_VERSION`].
    pub struct KAggregateGeom {
        pub sphere_elems: Vec<KSphereElem>,
        pub box_elems: Vec<KBoxElem>,
        pub cylinder_elems: Vec<KCylinderElem>,
        pub convex_elems: Vec<KConvexElem>,
    }
}

/// Karma's collision and mass for a mesh (`UKMeshProps::Serialize`), after
/// its tagged properties, from version [`K_CONVEX_VERSION`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KMeshProps {
    pub inertia_tensor: [f32; 6],
    pub com_offset: Vector,
    pub agg_geom: KAggregateGeom,
}

impl Serialize for KMeshProps {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        at_least(ar, K_CONVEX_VERSION, "Karma mesh properties")?;
        self.inertia_tensor.serialize(ar)?;
        self.com_offset.serialize(ar)?;
        self.agg_geom.serialize(ar)
    }
}

native_struct! {
    /// `FConvexVolumeFace`.
    pub struct ConvexVolumeFace {
        pub plane: Plane,
        pub vertices: Vec<Vector>,
    }
}

native_struct! {
    /// `FConvexVolumeEdge`: the two faces that share it, and where it
    /// starts in each.
    pub struct ConvexVolumeEdge {
        pub faces: [i32; 2],
        pub vertices: [i32; 2],
    }
}

native_struct! {
    /// A convex volume's native data (`UConvexVolume::Serialize`), after its
    /// tagged properties.
    pub struct ConvexVolume {
        pub primitive: Primitive,
        pub faces: Vec<ConvexVolumeFace>,
        pub edges: Vec<ConvexVolumeEdge>,
        pub bounding_box: BoundingBox,
    }
}

native_struct! {
    /// `FMeshTri`, in a LOD mesh of internal version 1 or older.
    pub struct MeshTri {
        pub vertices: [u16; 3],
        pub tex: [MeshUv; 3],
        pub poly_flags: u32,
        pub material_index: i32,
    }
}

native_struct! {
    /// `FMeshFace`: three wedges and a material.
    pub struct MeshFace {
        pub wedges: [u16; 3],
        pub mesh_material_index: u16,
    }
}

native_struct! {
    /// `FMeshWedge`: a vertex with texture coordinates.
    pub struct MeshWedge {
        pub vertex: u16,
        pub tex_uv: MeshUv,
    }
}

native_struct! {
    /// `FMeshMaterial`.
    pub struct MeshMaterial {
        pub poly_flags: u32,
        pub material_index: i32,
    }
}

native_struct! {
    /// `FImpostorProps`, as its `operator<<` orders them.
    pub struct ImpostorProps {
        pub material: ObjectRef,
        pub relative_location: Vector,
        pub relative_rotation: Rotator,
        pub scale_3d: Vector,
        pub imp_color: Color,
        pub imp_space_mode: i32,
        pub imp_draw_mode: i32,
        pub imp_light_mode: i32,
    }
}

/// `ULodMesh::Serialize`: what every skeletal or vertex mesh starts with,
/// after its tagged properties. What's stored depends on
/// `internal_version`, the mesh's own version.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LodMesh {
    /// `UMesh` adds nothing to `UPrimitive`.
    pub primitive: Primitive,
    pub internal_version: i32,
    pub model_verts: i32,
    /// Each a packed `FMeshVert`: 11 bits of X, 11 of Y and 10 of Z.
    pub verts: Vec<u32>,
    /// Internal version 1 or older.
    pub tris: Option<Vec<MeshTri>>,
    pub materials: Vec<ObjectRef>,
    pub scale: Vector,
    pub origin: Vector,
    pub rot_origin: Rotator,
    /// Internal version 1 or older.
    pub collapse_point_thus: Option<Vec<u16>>,
    pub face_level: Vec<u16>,
    pub faces: Vec<MeshFace>,
    pub collapse_wedge_thus: Vec<u16>,
    pub wedges: Vec<MeshWedge>,
    pub mesh_materials: Vec<MeshMaterial>,
    pub mesh_scale_max: f32,
    pub lod_hysteresis: f32,
    pub lod_strength: f32,
    pub lod_min_verts: i32,
    pub lod_morph: f32,
    pub lod_z_displace: f32,
    /// Internal version 3 or newer.
    pub impostor_present: Option<i32>,
    pub impostor_props: Option<ImpostorProps>,
    /// Internal version 4 or newer.
    pub skin_tesselation_factor: Option<f32>,
}

impl Serialize for LodMesh {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        self.primitive.serialize(ar)?;
        self.internal_version.serialize(ar)?;
        self.model_verts.serialize(ar)?;
        self.verts.serialize(ar)?;
        let old = self.internal_version < 2;
        if old {
            self.tris.get_or_insert_with(Vec::new).serialize(ar)?;
        }
        self.materials.serialize(ar)?;
        self.scale.serialize(ar)?;
        self.origin.serialize(ar)?;
        self.rot_origin.serialize(ar)?;
        if old {
            self.collapse_point_thus
                .get_or_insert_with(Vec::new)
                .serialize(ar)?;
        }
        self.face_level.serialize(ar)?;
        self.faces.serialize(ar)?;
        self.collapse_wedge_thus.serialize(ar)?;
        self.wedges.serialize(ar)?;
        self.mesh_materials.serialize(ar)?;
        self.mesh_scale_max.serialize(ar)?;
        self.lod_hysteresis.serialize(ar)?;
        self.lod_strength.serialize(ar)?;
        self.lod_min_verts.serialize(ar)?;
        self.lod_morph.serialize(ar)?;
        self.lod_z_displace.serialize(ar)?;
        if self.internal_version >= 3 {
            self.impostor_present.get_or_insert(0).serialize(ar)?;
            self.impostor_props
                .get_or_insert_with(ImpostorProps::default)
                .serialize(ar)?;
        }
        if self.internal_version >= 4 {
            self.skin_tesselation_factor
                .get_or_insert(0.0)
                .serialize(ar)?;
        }
        Ok(())
    }
}

native_struct! {
    /// `VJointPos`: a bone's position. The engine writes `XSize` twice and
    /// no `YSize`, so `x_size_again` is the second `XSize` (the one the
    /// engine keeps).
    pub struct JointPos {
        pub orientation: Quat,
        pub position: Vector,
        pub length: f32,
        pub x_size: f32,
        pub x_size_again: f32,
        pub z_size: f32,
    }
}

native_struct! {
    /// `FMeshBone`: a bone of a skeleton.
    pub struct MeshBone {
        pub name: NameRef,
        pub flags: u32,
        pub bone_pos: JointPos,
        pub num_children: i32,
        pub parent_index: i32,
    }
}

native_struct! {
    /// `VWeightIndex`.
    pub struct WeightIndex {
        pub point_indices: Vec<u16>,
        pub weight_base: i32,
    }
}

native_struct! {
    /// `VBoneInfluence`.
    pub struct BoneInfluence {
        pub bone_weight: u16,
        pub bone_index: u16,
    }
}

native_struct! {
    /// `FLODMeshSection` and `FSkelMeshSection`, as they're stored (the
    /// latter's `TotalWedges` isn't).
    pub struct MeshSection {
        pub material_index: u16,
        pub first_index: u16,
        pub min_index: u16,
        pub max_index: u16,
        pub total_verts: u16,
        pub max_influences: u16,
        pub shader_index: u16,
        pub first_face: u16,
        pub total_faces: u16,
    }
}

native_struct! {
    /// `FSkinPoint`: a vertex and its packed normal (`FMeshNorm`: 10 bits
    /// each of X, Y and Z).
    pub struct SkinPoint {
        pub point: Vector,
        pub normal: u32,
    }
}

native_struct! {
    /// `FAnimMeshVertex`.
    pub struct AnimMeshVertex {
        pub position: Vector,
        pub normal: Vector,
        pub u: f32,
        pub v: f32,
    }
}

native_struct! {
    /// `FSkinVertexStream`.
    pub struct SkinVertexStream {
        pub revision: i32,
        pub partial: i32,
        pub stream_callback: i32,
        pub vertices: Vec<AnimMeshVertex>,
    }
}

native_struct! {
    /// `FVertInfluence`.
    pub struct VertInfluence {
        pub weight: f32,
        pub vert_index: u16,
        pub bone_index: u16,
    }
}

native_struct! {
    /// `FStaticLODModel`: a level of detail of a skeletal mesh.
    pub struct StaticLodModel {
        pub skinning_stream: Vec<u32>,
        pub smooth_verts: Vec<SkinPoint>,
        pub smooth_stream_wedges: i32,
        pub smooth_sections: Vec<MeshSection>,
        pub rigid_sections: Vec<MeshSection>,
        pub smooth_index_buffer: RawIndexBuffer,
        pub rigid_index_buffer: RawIndexBuffer,
        pub rigid_vertex_stream: SkinVertexStream,
        pub influences: LazyArray<VertInfluence>,
        pub wedges: LazyArray<MeshWedge>,
        pub faces: LazyArray<MeshFace>,
        pub points: LazyArray<Vector>,
        pub display_factor: f32,
        pub lod_hysteresis: f32,
        pub dup_vert_count: i32,
        pub max_influences: i32,
        pub unique_subset: i32,
        pub use_smoothing: i32,
    }
}

native_struct! {
    /// `VTriangle`: a face as it was imported.
    pub struct RawTriangle {
        pub wedge_index: [u16; 3],
        pub mat_index: u8,
        pub aux_mat_index: u8,
        pub smoothing_groups: u32,
    }
}

/// `FSkelBoneSphere` and `FSkelBoneBox`'s flags, as each version stores
/// them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BoneCollisionFlags {
    /// From version 124.
    pub block_karma: Option<i32>,
    /// From version 125.
    pub block_non_zero_extent: Option<i32>,
    pub block_zero_extent: Option<i32>,
}

impl Serialize for BoneCollisionFlags {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        if ar.version() >= 124 {
            self.block_karma.get_or_insert(0).serialize(ar)?;
        }
        if ar.version() >= 125 {
            self.block_non_zero_extent.get_or_insert(0).serialize(ar)?;
            self.block_zero_extent.get_or_insert(0).serialize(ar)?;
        }
        Ok(())
    }
}

native_struct! {
    /// `FSkelBoneSphere`: a sphere attached to a bone, for collision.
    pub struct SkelBoneSphere {
        pub bone_name: NameRef,
        pub offset: Vector,
        pub radius: f32,
        pub flags: BoneCollisionFlags,
    }
}

native_struct! {
    /// `FSkelBoneBox`: a box attached to a bone, for collision.
    pub struct SkelBoneBox {
        pub bone_name: NameRef,
        pub offset: Vector,
        pub radii: Vector,
        pub flags: BoneCollisionFlags,
    }
}

/// A skeletal mesh's native data (`USkeletalMesh::Serialize`), after its
/// tagged properties.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkeletalMesh {
    pub lod_mesh: LodMesh,
    pub points: Vec<Vector>,
    pub ref_skeleton: Vec<MeshBone>,
    pub default_anim: ObjectRef,
    pub skeletal_depth: i32,
    pub multi_blends: Vec<WeightIndex>,
    pub weights: Vec<BoneInfluence>,
    pub tag_aliases: Vec<NameRef>,
    pub tag_names: Vec<NameRef>,
    pub tag_coords: Vec<Coords>,
    /// Internal version 1 or older.
    pub smooth_sections: Option<Vec<MeshSection>>,
    pub static_sections: Option<Vec<MeshSection>>,
    pub collapse_remap: Option<Vec<u16>>,
    /// Internal version 2 or newer.
    pub lod_models: Option<Vec<StaticLodModel>>,
    pub raw: Option<RawSkeletalMesh>,
    /// From version 120.
    pub authentication_key: Option<u32>,
    /// From version 122.
    pub collision: Option<SkeletalMeshCollision>,
    /// From version 127.
    pub collision_static_mesh: Option<ObjectRef>,
}

native_struct! {
    /// A skeletal mesh's raw data, as it was imported, from internal
    /// version 2.
    pub struct RawSkeletalMesh {
        pub default_ref_mesh: ObjectRef,
        pub verts: LazyArray<Vector>,
        pub wedges: LazyArray<MeshWedge>,
        pub faces: LazyArray<RawTriangle>,
        pub influences: LazyArray<VertInfluence>,
        pub collapse_wedges: LazyArray<u16>,
        pub face_level: LazyArray<u16>,
    }
}

native_struct! {
    /// A skeletal mesh's collision, from version 122.
    pub struct SkeletalMeshCollision {
        pub k_physics_props: ObjectRef,
        pub bone_collision_spheres: Vec<SkelBoneSphere>,
        pub bone_collision_boxes: Vec<SkelBoneBox>,
        pub bone_collision_box_models: Vec<ObjectRef>,
    }
}

impl Serialize for SkeletalMesh {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        self.lod_mesh.serialize(ar)?;
        self.points.serialize(ar)?;
        self.ref_skeleton.serialize(ar)?;
        self.default_anim.serialize(ar)?;
        self.skeletal_depth.serialize(ar)?;
        self.multi_blends.serialize(ar)?;
        self.weights.serialize(ar)?;
        self.tag_aliases.serialize(ar)?;
        self.tag_names.serialize(ar)?;
        self.tag_coords.serialize(ar)?;
        if self.lod_mesh.internal_version < 2 {
            self.smooth_sections
                .get_or_insert_with(Vec::new)
                .serialize(ar)?;
            self.static_sections
                .get_or_insert_with(Vec::new)
                .serialize(ar)?;
            self.collapse_remap
                .get_or_insert_with(Vec::new)
                .serialize(ar)?;
        } else {
            self.lod_models.get_or_insert_with(Vec::new).serialize(ar)?;
            self.raw
                .get_or_insert_with(RawSkeletalMesh::default)
                .serialize(ar)?;
        }
        if ar.version() >= 120 {
            self.authentication_key.get_or_insert(0).serialize(ar)?;
        }
        if ar.version() >= 122 {
            self.collision
                .get_or_insert_with(SkeletalMeshCollision::default)
                .serialize(ar)?;
        }
        if ar.version() >= 127 {
            self.collision_static_mesh
                .get_or_insert(ObjectRef::Null)
                .serialize(ar)?;
        }
        Ok(())
    }
}

native_struct! {
    /// `FAnimMeshVertexStream`.
    pub struct AnimMeshVertexStream {
        pub vertices: Vec<AnimMeshVertex>,
        pub revision: i32,
    }
}

native_struct! {
    /// A vertex-animated mesh's native data (`UVertMesh::Serialize`), after
    /// its tagged properties.
    pub struct VertMesh {
        pub lod_mesh: LodMesh,
        pub vertex_stream: AnimMeshVertexStream,
        /// Each frame's vertices, packed as `FMeshVert`s.
        pub anim_verts: Vec<u32>,
        pub frame_keys: Vec<f32>,
        pub anim_seqs: Vec<MeshAnimSeq>,
        /// Each frame's normals, packed as `FMeshNorm`s.
        pub anim_normals: Vec<u32>,
        pub frame_verts: i32,
        pub anim_frames: i32,
        pub bounding_boxes: Vec<BoundingBox>,
        pub bounding_spheres: Vec<Sphere>,
    }
}

native_struct! {
    /// `FNamedBone`: a bone of an animation's skeleton.
    pub struct NamedBone {
        pub name: NameRef,
        pub flags: u32,
        pub parent_index: i32,
    }
}

native_struct! {
    /// `AnalogTrack`: a bone's keys.
    pub struct AnalogTrack {
        pub flags: u32,
        pub key_quat: Vec<Quat>,
        pub key_pos: Vec<Vector>,
        pub key_time: Vec<f32>,
    }
}

native_struct! {
    /// `MotionChunk`: an animation's tracks.
    pub struct MotionChunk {
        pub root_speed_3d: Vector,
        pub track_time: f32,
        pub start_bone: i32,
        pub flags: u32,
        pub bone_indices: Vec<i32>,
        pub anim_tracks: Vec<AnalogTrack>,
        pub root_track: AnalogTrack,
    }
}

/// `FMeshAnimNotify`: something to do at a time in an animation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshAnimNotify {
    pub time: f32,
    pub function: NameRef,
    /// From version 112.
    pub notify_object: Option<ObjectRef>,
}

impl Serialize for MeshAnimNotify {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        self.time.serialize(ar)?;
        self.function.serialize(ar)?;
        if ar.version() >= 112 {
            self.notify_object
                .get_or_insert(ObjectRef::Null)
                .serialize(ar)?;
        }
        Ok(())
    }
}

/// `FMeshAnimSeq`: an animation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshAnimSeq {
    /// From version 115.
    pub bookmark: Option<f32>,
    pub name: NameRef,
    pub groups: Vec<NameRef>,
    pub start_frame: i32,
    pub num_frames: i32,
    pub notifys: Vec<MeshAnimNotify>,
    pub rate: f32,
}

impl Serialize for MeshAnimSeq {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        if ar.version() >= 115 {
            self.bookmark.get_or_insert(0.0).serialize(ar)?;
        }
        self.name.serialize(ar)?;
        self.groups.serialize(ar)?;
        self.start_frame.serialize(ar)?;
        self.num_frames.serialize(ar)?;
        self.notifys.serialize(ar)?;
        self.rate.serialize(ar)
    }
}

native_struct! {
    /// A skeletal animation's native data (`UMeshAnimation::Serialize`),
    /// after its tagged properties.
    pub struct MeshAnimation {
        pub internal_version: i32,
        pub ref_bones: Vec<NamedBone>,
        pub moves: Vec<MotionChunk>,
        pub anim_seqs: Vec<MeshAnimSeq>,
    }
}

/// Fails, as for an unknown layout, if the package is older than `version`.
pub(crate) fn at_least(ar: &dyn Archive, version: u16, what: &str) -> Result<()> {
    if ar.version() < version {
        return Err(Error::InvalidData {
            offset: ar.pos(),
            reason: format!("{} before version {} aren't read", what, version),
        });
    }
    Ok(())
}
