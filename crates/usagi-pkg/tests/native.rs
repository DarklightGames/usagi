mod common;

use common::{ExportBytes, ImportBytes, PackageBytes, SummaryBytes, name};
use usagi_pkg::archive::{FString, Index, LazyArray, Loader, NameRef, Saver, Serialize};
use usagi_pkg::level::{
    Level, StringPair, TerrainInfo, TerrainNormalPair, TerrainPrimitive, TerrainSector,
    TerrainSectorLightInfo, Url,
};
use usagi_pkg::math::{BoundingBox, Color, Coords, Matrix, Plane, Quat, Rotator, Sphere, Vector};
use usagi_pkg::mesh::{
    AnalogTrack, AnimMeshVertex, AnimMeshVertexStream, BoneCollisionFlags, BoneInfluence,
    ConvexVolume, ConvexVolumeEdge, ConvexVolumeFace, ImpostorProps, JointPos, KAggregateGeom,
    KBoxElem, KConvexElem, KCylinderElem, KDop, KDopNode, KDopTree, KDopTriangle, KMeshProps,
    KSphereElem, LodMesh, MeshAnimNotify, MeshAnimSeq, MeshAnimation, MeshBone, MeshFace,
    MeshMaterial, MeshSection, MeshTri, MeshUv, MeshWedge, MotionChunk, NamedBone, Primitive,
    RawColorStream, RawIndexBuffer, RawSkeletalMesh, RawTriangle, SkelBoneBox, SkelBoneSphere,
    SkeletalMesh, SkeletalMeshCollision, SkinPoint, SkinVertexStream, StaticLodModel, StaticMesh,
    StaticMeshCollisionNode, StaticMeshCollisionTriangle, StaticMeshInstance, StaticMeshLightInfo,
    StaticMeshSection, StaticMeshTriangle, StaticMeshUvStream, StaticMeshVertex,
    StaticMeshVertexStream, VertInfluence, VertMesh, WeightIndex,
};
use usagi_pkg::model::{
    BspNode, BspSection, BspSurf, BspVertex, BspVertexStream, Leaf, LightBitmap, LightMap,
    LightMapTexture, Model, Poly, Polys, StaticLightMapTexture, Vert, ZoneProperties,
};
use usagi_pkg::{
    CharRemap, Class, Const, Enum, FireTexture, Font, FontCharacter, Function, Mipmap, Native,
    NativeClass, ObjectRef, OldFontCharacter, OldFontPage, Package, Palette, Prefab,
    ProceduralSound, PropertyDecl, PropertyKind, Sound, SoundGroup, Spark, State, Struct,
    TextBuffer, Texture,
};

const ME: ObjectRef = ObjectRef::Export(0);
const CORE: ObjectRef = ObjectRef::Import(0);
const ALPHA: NameRef = NameRef(1);

/// A package whose names are `None`, `Alpha` and `Beta`, with an import of
/// `Core` and an export with `data`.
fn package_with(version: u32, licensee: u32, data: Vec<u8>) -> Package {
    let bytes = PackageBytes {
        summary: SummaryBytes {
            file_version: version | licensee << 16,
            ..SummaryBytes::default()
        },
        names: vec![name("None"), name("Alpha"), name("Beta")],
        imports: vec![ImportBytes {
            class_package: 1,
            class_name: 2,
            outer: 0,
            object_name: 1,
        }],
        exports: vec![ExportBytes {
            size: data.len() as i32,
            ..ExportBytes::default()
        }],
        data: vec![data],
    }
    .bytes();
    let mut package = Package::from_bytes(&bytes).unwrap();
    package.name = "MyPkg".to_string();
    package
}

/// Writes `native` as `class`'s, after a `None` that ends the object's
/// properties, and checks that it reads back the same, writes back the
/// same, and can't be read from less of it.
fn check(class: NativeClass, mut native: Native, version: u32, licensee: u32) {
    let (v, l) = (version as u16, licensee as u16);
    // The data's offset doesn't depend on its size.
    let offset = package_with(version, licensee, vec![0]).exports()[0]
        .serial_offset
        .unwrap() as usize;
    let bytes = native.write(v, l, offset + 1).unwrap();
    let data = [vec![0], bytes.clone()].concat();
    let package = package_with(version, licensee, data.clone());
    let read = package.native_data(0, &class, 1).unwrap();
    assert_eq!(read.native, native);
    assert_eq!((read.start, read.end), (1, data.len()));
    let round_trip = package.round_trip(0, &class, 1).unwrap();
    assert_eq!(round_trip.size, bytes.len());
    assert_eq!(round_trip.difference, None);
    for len in 1..data.len() {
        let package = package_with(version, licensee, data[..len].to_vec());
        assert!(package.native_data(0, &class, 1).is_err(), "{} bytes", len);
    }
}

/// The error reading `native`'s bytes as `class`'s, written at `written`
/// and read at `version`.
fn error(class: NativeClass, mut native: Native, written: u32, version: u32) -> String {
    let bytes = native.write(written as u16, 29, 0).unwrap();
    let data = [vec![0], bytes].concat();
    let package = package_with(version, 29, data);
    package.native_data(0, &class, 1).unwrap_err().to_string()
}

fn v(x: f32) -> Vector {
    Vector {
        x,
        y: x + 1.0,
        z: x + 2.0,
    }
}

fn plane(x: f32) -> Plane {
    Plane {
        x,
        y: x + 1.0,
        z: x + 2.0,
        w: x + 3.0,
    }
}

fn bounds() -> BoundingBox {
    BoundingBox {
        min: v(-1.0),
        max: v(1.0),
        is_valid: 1,
    }
}

fn sphere() -> Sphere {
    Sphere {
        center: v(0.5),
        radius: Some(4.0),
    }
}

fn primitive() -> Primitive {
    Primitive {
        bounding_box: bounds(),
        bounding_sphere: sphere(),
    }
}

fn color() -> Color {
    Color {
        r: 1,
        g: 2,
        b: 3,
        a: 4,
    }
}

fn matrix() -> Matrix {
    Matrix {
        m: [[1.0, 2.0, 3.0, 4.0]; 4],
    }
}

fn coords() -> Coords {
    Coords {
        origin: v(0.0),
        x_axis: v(1.0),
        y_axis: v(2.0),
        z_axis: v(3.0),
    }
}

fn quat() -> Quat {
    Quat {
        x: 0.0,
        y: 0.5,
        z: 0.25,
        w: 1.0,
    }
}

fn uv() -> MeshUv {
    MeshUv { u: 0.5, v: 0.75 }
}

fn index_buffer() -> RawIndexBuffer {
    RawIndexBuffer {
        indices: vec![0, 1, 2],
        revision: 3,
    }
}

fn color_stream() -> RawColorStream {
    RawColorStream {
        colors: vec![color()],
        revision: 2,
    }
}

fn lazy<T>(items: Vec<T>) -> LazyArray<T> {
    LazyArray {
        seek_pos: None,
        items,
    }
}

fn static_mesh_triangle(num_uvs: i32) -> StaticMeshTriangle {
    StaticMeshTriangle {
        vertices: [v(1.0), v(2.0), v(3.0)],
        num_uvs,
        uvs: vec![std::array::from_fn(|_| uv()); num_uvs.max(0) as usize],
        colors: std::array::from_fn(|_| color()),
        material_index: 1,
        smoothing_mask: 0xF0,
    }
}

fn static_mesh() -> StaticMesh {
    StaticMesh {
        primitive: primitive(),
        sections: vec![StaticMeshSection {
            is_strip: 1,
            first_index: 2,
            min_vertex_index: 3,
            max_vertex_index: 4,
            num_triangles: 5,
            num_primitives: 6,
        }],
        bounding_box: bounds(),
        vertex_stream: StaticMeshVertexStream {
            vertices: vec![StaticMeshVertex {
                position: v(1.0),
                normal: v(0.0),
            }],
            revision: 1,
        },
        color_stream: color_stream(),
        alpha_stream: color_stream(),
        uv_streams: vec![StaticMeshUvStream {
            uvs: vec![uv()],
            coordinate_index: 1,
            revision: 2,
        }],
        index_buffer: index_buffer(),
        wireframe_index_buffer: index_buffer(),
        collision_model: ME,
        raw_triangles: lazy(vec![static_mesh_triangle(2), static_mesh_triangle(-1)]),
        internal_version: 8,
        k_physics_props: CORE,
        ..StaticMesh::default()
    }
}

#[test]
fn static_meshes() {
    let mesh = StaticMesh {
        kdop_tree: Some(KDopTree {
            nodes: vec![KDopNode {
                bounding_volume: KDop {
                    min: [0.0, 1.0, 2.0],
                    max: [3.0, 4.0, 5.0],
                },
                is_leaf: 1,
                a: 2,
                b: 3,
            }],
            triangles: vec![KDopTriangle {
                vertices: [0, 1, 2],
                material_index: 1,
            }],
        }),
        max_smoothing_angles: Some(vec![45.0]),
        authentication_key: Some(0xDEAD),
        ..static_mesh()
    };
    check(
        NativeClass::StaticMesh,
        Native::StaticMesh(Box::new(mesh)),
        128,
        29,
    );
    // Before version 126, its collision is triangles and nodes; before 114,
    // with flags; and licensee version 22 has a smoothing threshold.
    let old = StaticMesh {
        collision_triangles: Some(vec![StaticMeshCollisionTriangle {
            plane: plane(1.0),
            side_planes: [plane(2.0), plane(3.0), plane(4.0)],
            vertex_indices: [Index(0), Index(100), Index(-3)],
            material_index: Index(1),
        }]),
        collision_nodes: Some(vec![StaticMeshCollisionNode {
            triangle_index: Index(0),
            coplanar_index: Index(-1),
            child_indices: [Index(1), Index(2)],
            bounding_box: bounds(),
        }]),
        use_simple_collision: Some([1, 0, 1]),
        smoothing_threshold: Some(0.5),
        ..static_mesh()
    };
    check(
        NativeClass::StaticMesh,
        Native::StaticMesh(Box::new(old)),
        112,
        0x16,
    );
    check(
        NativeClass::StaticMesh,
        Native::StaticMesh(Box::new(static_mesh())),
        120,
        0,
    );
    let error = error(
        NativeClass::StaticMesh,
        Native::StaticMesh(Box::default()),
        128,
        111,
    );
    assert!(
        error.contains("static meshes before version 112 aren't read"),
        "{}",
        error
    );
    // Nor are they written.
    assert!(
        Native::StaticMesh(Box::default())
            .write(111, 29, 0)
            .is_err()
    );
}

#[test]
fn a_static_mesh_triangle_has_up_to_8_uvs() {
    let mut mesh = static_mesh();
    mesh.raw_triangles.items = vec![static_mesh_triangle(9)];
    mesh.kdop_tree = Some(KDopTree::default());
    let error = error(
        NativeClass::StaticMesh,
        Native::StaticMesh(Box::new(mesh)),
        128,
        128,
    );
    assert!(
        error.contains("a static mesh triangle has 9 UVs"),
        "{}",
        error
    );
}

#[test]
fn static_mesh_instances() {
    let instance = StaticMeshInstance {
        color_stream: color_stream(),
        lights: vec![StaticMeshLightInfo {
            light_actor: ME,
            visibility_bits: vec![0xFF, 0x0F],
            applied: 1,
        }],
    };
    check(
        NativeClass::StaticMeshInstance,
        Native::StaticMeshInstance(instance.clone()),
        128,
        29,
    );
    let error = error(
        NativeClass::StaticMeshInstance,
        Native::StaticMeshInstance(instance),
        128,
        111,
    );
    assert!(error.contains("before version 112"), "{}", error);
}

#[test]
fn karma_mesh_properties() {
    let props = KMeshProps {
        inertia_tensor: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        com_offset: v(1.0),
        agg_geom: KAggregateGeom {
            sphere_elems: vec![KSphereElem {
                tm: matrix(),
                radius: 1.0,
            }],
            box_elems: vec![KBoxElem {
                tm: matrix(),
                x: 1.0,
                y: 2.0,
                z: 3.0,
            }],
            cylinder_elems: vec![KCylinderElem {
                tm: matrix(),
                radius: 1.0,
                height: 2.0,
            }],
            convex_elems: vec![KConvexElem {
                tm: matrix(),
                vertex_data: vec![v(1.0), v(2.0)],
            }],
        },
    };
    check(
        NativeClass::KMeshProps,
        Native::KMeshProps(props.clone()),
        128,
        29,
    );
    let error = error(NativeClass::KMeshProps, Native::KMeshProps(props), 128, 112);
    assert!(error.contains("before version 113"), "{}", error);
}

#[test]
fn convex_volumes() {
    let volume = ConvexVolume {
        primitive: primitive(),
        faces: vec![ConvexVolumeFace {
            plane: plane(1.0),
            vertices: vec![v(1.0), v(2.0), v(3.0)],
        }],
        edges: vec![ConvexVolumeEdge {
            faces: [0, 1],
            vertices: [2, 3],
        }],
        bounding_box: bounds(),
    };
    check(
        NativeClass::ConvexVolume,
        Native::ConvexVolume(volume),
        128,
        29,
    );
}

fn lod_mesh(internal_version: i32) -> LodMesh {
    LodMesh {
        primitive: primitive(),
        internal_version,
        model_verts: 3,
        verts: vec![0x1234_5678],
        materials: vec![CORE],
        scale: v(1.0),
        origin: v(0.0),
        rot_origin: Rotator {
            pitch: 1,
            yaw: 2,
            roll: 3,
        },
        face_level: vec![1],
        faces: vec![MeshFace {
            wedges: [0, 1, 2],
            mesh_material_index: 0,
        }],
        collapse_wedge_thus: vec![0, 0],
        wedges: vec![MeshWedge {
            vertex: 1,
            tex_uv: uv(),
        }],
        mesh_materials: vec![MeshMaterial {
            poly_flags: 0x100,
            material_index: 0,
        }],
        mesh_scale_max: 1.0,
        lod_hysteresis: 0.5,
        lod_strength: 1.0,
        lod_min_verts: 10,
        lod_morph: 0.3,
        lod_z_displace: 0.0,
        ..LodMesh::default()
    }
}

#[test]
fn lod_meshes() {
    // Internal version 1 has triangles and points' collapses.
    let old = LodMesh {
        tris: Some(vec![MeshTri {
            vertices: [0, 1, 2],
            tex: std::array::from_fn(|_| uv()),
            poly_flags: 1,
            material_index: 0,
        }]),
        collapse_point_thus: Some(vec![1, 2]),
        ..lod_mesh(1)
    };
    check(
        NativeClass::LodMesh,
        Native::LodMesh(Box::new(old)),
        128,
        29,
    );
    // 3 an impostor, and 4 a tesselation factor.
    let impostor = || ImpostorProps {
        material: CORE,
        relative_location: v(1.0),
        relative_rotation: Rotator::default(),
        scale_3d: v(2.0),
        imp_color: color(),
        imp_space_mode: 1,
        imp_draw_mode: 2,
        imp_light_mode: 3,
    };
    let three = LodMesh {
        impostor_present: Some(1),
        impostor_props: Some(impostor()),
        ..lod_mesh(3)
    };
    check(
        NativeClass::LodMesh,
        Native::LodMesh(Box::new(three)),
        128,
        29,
    );
    let four = LodMesh {
        impostor_present: Some(1),
        impostor_props: Some(impostor()),
        skin_tesselation_factor: Some(2.0),
        ..lod_mesh(4)
    };
    check(
        NativeClass::LodMesh,
        Native::LodMesh(Box::new(four)),
        128,
        29,
    );
}

fn skeletal_mesh(internal_version: i32) -> SkeletalMesh {
    SkeletalMesh {
        lod_mesh: lod_mesh(internal_version),
        points: vec![v(1.0)],
        ref_skeleton: vec![MeshBone {
            name: ALPHA,
            flags: 0,
            bone_pos: JointPos {
                orientation: quat(),
                position: v(1.0),
                length: 1.0,
                x_size: 2.0,
                x_size_again: 3.0,
                z_size: 4.0,
            },
            num_children: 1,
            parent_index: 0,
        }],
        default_anim: CORE,
        skeletal_depth: 2,
        multi_blends: vec![WeightIndex {
            point_indices: vec![1, 2],
            weight_base: 3,
        }],
        weights: vec![BoneInfluence {
            bone_weight: 100,
            bone_index: 1,
        }],
        tag_aliases: vec![ALPHA],
        tag_names: vec![NameRef(2)],
        tag_coords: vec![coords()],
        ..SkeletalMesh::default()
    }
}

fn section() -> MeshSection {
    MeshSection {
        material_index: 1,
        first_index: 2,
        min_index: 3,
        max_index: 4,
        total_verts: 5,
        max_influences: 6,
        shader_index: 7,
        first_face: 8,
        total_faces: 9,
    }
}

fn lod_model() -> StaticLodModel {
    StaticLodModel {
        skinning_stream: vec![1, 2, 3],
        smooth_verts: vec![SkinPoint {
            point: v(1.0),
            normal: 0x3FF,
        }],
        smooth_stream_wedges: 1,
        smooth_sections: vec![section()],
        rigid_sections: vec![section()],
        smooth_index_buffer: index_buffer(),
        rigid_index_buffer: index_buffer(),
        rigid_vertex_stream: SkinVertexStream {
            revision: 1,
            partial: 0,
            stream_callback: 0,
            vertices: vec![AnimMeshVertex {
                position: v(1.0),
                normal: v(0.0),
                u: 0.5,
                v: 0.5,
            }],
        },
        influences: lazy(vec![VertInfluence {
            weight: 1.0,
            vert_index: 2,
            bone_index: 3,
        }]),
        wedges: lazy(vec![MeshWedge {
            vertex: 1,
            tex_uv: uv(),
        }]),
        faces: lazy(vec![MeshFace {
            wedges: [0, 1, 2],
            mesh_material_index: 0,
        }]),
        points: lazy(vec![v(1.0)]),
        display_factor: 1.0,
        lod_hysteresis: 0.1,
        dup_vert_count: 2,
        max_influences: 4,
        unique_subset: 0,
        use_smoothing: 1,
    }
}

fn raw_skeletal_mesh() -> RawSkeletalMesh {
    RawSkeletalMesh {
        default_ref_mesh: ME,
        verts: lazy(vec![v(1.0)]),
        wedges: lazy(vec![MeshWedge {
            vertex: 0,
            tex_uv: uv(),
        }]),
        faces: lazy(vec![RawTriangle {
            wedge_index: [0, 1, 2],
            mat_index: 1,
            aux_mat_index: 2,
            smoothing_groups: 3,
        }]),
        influences: lazy(vec![VertInfluence::default()]),
        collapse_wedges: lazy(vec![1, 2]),
        face_level: lazy(vec![3]),
    }
}

fn collision(flags: BoneCollisionFlags) -> SkeletalMeshCollision {
    SkeletalMeshCollision {
        k_physics_props: CORE,
        bone_collision_spheres: vec![SkelBoneSphere {
            bone_name: ALPHA,
            offset: v(1.0),
            radius: 2.0,
            flags: flags.clone(),
        }],
        bone_collision_boxes: vec![SkelBoneBox {
            bone_name: ALPHA,
            offset: v(1.0),
            radii: v(2.0),
            flags,
        }],
        bone_collision_box_models: vec![ME],
    }
}

#[test]
fn skeletal_meshes() {
    let mesh = |internal: i32| SkeletalMesh {
        lod_models: Some(vec![lod_model()]),
        raw: Some(raw_skeletal_mesh()),
        ..skeletal_mesh(internal)
    };
    let all = BoneCollisionFlags {
        block_karma: Some(1),
        block_non_zero_extent: Some(1),
        block_zero_extent: Some(0),
    };
    let latest = SkeletalMesh {
        authentication_key: Some(7),
        collision: Some(collision(all)),
        collision_static_mesh: Some(CORE),
        ..mesh(4)
    };
    check(
        NativeClass::SkeletalMesh,
        Native::SkeletalMesh(Box::new(latest)),
        128,
        29,
    );
    // Bone collision without flags before version 124, and with only
    // `bBlockKarma` before 125.
    for (version, flags) in [
        (123, BoneCollisionFlags::default()),
        (
            124,
            BoneCollisionFlags {
                block_karma: Some(1),
                ..BoneCollisionFlags::default()
            },
        ),
    ] {
        let mesh = SkeletalMesh {
            authentication_key: Some(7),
            collision: Some(collision(flags)),
            ..mesh(4)
        };
        check(
            NativeClass::SkeletalMesh,
            Native::SkeletalMesh(Box::new(mesh)),
            version,
            29,
        );
    }
    // Internal version 1 has sections rather than LOD models, and before
    // version 120, there's no authentication key.
    let old = SkeletalMesh {
        lod_mesh: LodMesh {
            tris: Some(Vec::new()),
            collapse_point_thus: Some(Vec::new()),
            ..lod_mesh(1)
        },
        smooth_sections: Some(vec![section()]),
        static_sections: Some(vec![section()]),
        collapse_remap: Some(vec![1]),
        ..skeletal_mesh(1)
    };
    check(
        NativeClass::SkeletalMesh,
        Native::SkeletalMesh(Box::new(old)),
        119,
        29,
    );
}

#[test]
fn mesh_animations() {
    let animation = |notify_object, bookmark| MeshAnimation {
        internal_version: 1,
        ref_bones: vec![NamedBone {
            name: ALPHA,
            flags: 0,
            parent_index: 0,
        }],
        moves: vec![MotionChunk {
            root_speed_3d: v(1.0),
            track_time: 2.0,
            start_bone: 0,
            flags: 0,
            bone_indices: vec![0, -1],
            anim_tracks: vec![AnalogTrack {
                flags: 0,
                key_quat: vec![quat()],
                key_pos: vec![v(1.0)],
                key_time: vec![0.5],
            }],
            root_track: AnalogTrack::default(),
        }],
        anim_seqs: vec![MeshAnimSeq {
            bookmark,
            name: ALPHA,
            groups: vec![NameRef(2)],
            start_frame: 0,
            num_frames: 10,
            notifys: vec![MeshAnimNotify {
                time: 0.5,
                function: ALPHA,
                notify_object,
            }],
            rate: 30.0,
        }],
    };
    check(
        NativeClass::MeshAnimation,
        Native::MeshAnimation(animation(Some(ME), Some(1.0))),
        128,
        29,
    );
    // Before version 115, there's no bookmark; before 112, notifications
    // have no object.
    check(
        NativeClass::MeshAnimation,
        Native::MeshAnimation(animation(None, None)),
        111,
        29,
    );
}

fn bsp_node(inclusive_sphere_bound: Option<Sphere>) -> BspNode {
    BspNode {
        plane: plane(1.0),
        zone_mask: u64::MAX,
        node_flags: 3,
        vert_pool: Index(1),
        surf: Index(2),
        children: [Index(0), Index(1), Index(-1)],
        collision_bound: Index(3),
        render_bound: Index(4),
        exclusive_sphere_bound: sphere(),
        inclusive_sphere_bound,
        zones: [1, 2],
        num_vertices: 4,
        leaves: [0, -1],
        section: 1,
        first_vertex: 2,
        light_map: -1,
    }
}

fn model(
    inclusive_sphere_bound: Option<Sphere>,
    static_texture: Option<StaticLightMapTexture>,
) -> Model {
    Model {
        primitive: primitive(),
        vectors: vec![v(1.0)],
        points: vec![v(2.0)],
        nodes: vec![bsp_node(inclusive_sphere_bound)],
        surfs: vec![BspSurf {
            material: CORE,
            poly_flags: 1,
            base: Index(0),
            normal: Index(1),
            texture_u: Index(2),
            texture_v: Index(3),
            brush_poly: Index(-1),
            actor: ME,
            plane: plane(0.0),
            light_map_scale: 32.0,
        }],
        verts: vec![Vert {
            vertex: Index(1),
            side: Index(-1),
        }],
        num_shared_sides: 2,
        num_zones: 1,
        zones: vec![ZoneProperties {
            zone_actor: ME,
            connectivity: 1,
            visibility: 2,
            last_render_time: 0.5,
        }],
        polys: ME,
        bounds: vec![bounds()],
        leaf_hulls: vec![1, 2],
        leaves: vec![Leaf {
            zone: Index(1),
            permeating: Index(2),
            volumetric: Index(3),
            visible_zones: 4,
        }],
        lights: vec![ME],
        root_outside: 1,
        linked: 0,
        sections: vec![BspSection {
            vertices: BspVertexStream {
                vertices: vec![BspVertex {
                    position: v(1.0),
                    u: 0.0,
                    v: 1.0,
                    u2: 2.0,
                    v2: 3.0,
                    normal: v(0.0),
                }],
                revision: 1,
            },
            material: CORE,
            num_nodes: 1,
            poly_flags: 0,
            light_map_texture: 0,
        }],
        light_maps: vec![LightMap {
            texture: Index(0),
            surf: Index(1),
            zone: Index(2),
            offset_x: Index(3),
            offset_y: Index(4),
            size_x: Index(5),
            size_y: Index(6),
            world_to_light_map: matrix(),
            light_map_base: v(0.0),
            light_map_x: v(1.0),
            light_map_y: v(2.0),
            bitmaps: vec![LightBitmap {
                light_actor: ME,
                bits: vec![1, 2, 3],
                size_x: 1,
                size_y: 2,
                stride: 3,
                min_x: 4,
                min_y: 5,
                max_x: 6,
                max_y: 7,
            }],
            level: ME,
            revision: 1,
        }],
        light_map_textures: vec![LightMapTexture {
            level: ME,
            light_maps: vec![0],
            cache_id: 1 << 40,
            revision: 2,
            static_texture,
        }],
    }
}

#[test]
fn models() {
    let texture = StaticLightMapTexture {
        data: [lazy(vec![1, 2, 3]), lazy(vec![4])],
        format: 5,
        width: 512,
        height: 512,
        revision: 1,
    };
    check(
        NativeClass::Model,
        Native::Model(Box::new(model(None, Some(texture)))),
        128,
        29,
    );
    // Before version 123, nodes have an inclusive sphere; before 116,
    // lightmap textures have no texels.
    check(
        NativeClass::Model,
        Native::Model(Box::new(model(Some(sphere()), None))),
        110,
        29,
    );
    let error = error(
        NativeClass::Model,
        Native::Model(Box::new(model(Some(sphere()), None))),
        110,
        109,
    );
    assert!(error.contains("models before version 110"), "{}", error);
}

#[test]
fn a_model_has_up_to_64_zones_and_none_if_it_says_fewer_than_none() {
    let mut many = model(None, None);
    many.num_zones = 65;
    many.zones = vec![ZoneProperties::default(); 65];
    let error = error(NativeClass::Model, Native::Model(Box::new(many)), 115, 115);
    assert!(error.contains("a model has 65 zones"), "{}", error);
    let mut none = model(None, None);
    none.num_zones = -1;
    none.zones = Vec::new();
    check(NativeClass::Model, Native::Model(Box::new(none)), 115, 29);
}

fn poly(pan: Option<[u16; 2]>, light_map_scale: Option<f32>) -> Poly {
    Poly {
        base: v(0.0),
        normal: v(1.0),
        texture_u: v(2.0),
        texture_v: v(3.0),
        vertices: vec![v(4.0), v(5.0), v(6.0)],
        poly_flags: 1,
        actor: ME,
        material: CORE,
        item_name: ALPHA,
        link: Index(-1),
        brush_poly: Index(2),
        pan,
        light_map_scale,
    }
}

#[test]
fn polys() {
    let polys = |poly| Polys {
        max: 4,
        elements: vec![poly],
    };
    check(
        NativeClass::Polys,
        Native::Polys(polys(poly(None, Some(32.0)))),
        128,
        29,
    );
    // Before version 78, polygons are panned; before 106, they have no
    // lightmap scale.
    check(
        NativeClass::Polys,
        Native::Polys(polys(poly(Some([1, 0xFFFF]), None))),
        77,
        29,
    );
    let mut big = poly(None, Some(1.0));
    big.vertices = vec![v(0.0); 17];
    let error = error(NativeClass::Polys, Native::Polys(polys(big)), 128, 128);
    assert!(error.contains("a polygon has 17 vertices"), "{}", error);
}

#[test]
fn an_old_style_array_cant_have_fewer_than_no_items() {
    let data = [vec![0], (-1i32).to_le_bytes().to_vec(), vec![0; 4]].concat();
    let package = package_with(128, 29, data);
    let error = package
        .native_data(0, &NativeClass::Polys, 1)
        .unwrap_err()
        .to_string();
    assert!(error.contains("there are -1 polygons"), "{}", error);
}

fn level() -> Level {
    let text = |s: &str| FString::Latin1(format!("{}\0", s).into_bytes());
    Level {
        actors_max: 3,
        actors: vec![ME, ObjectRef::Null],
        url: Url {
            protocol: text("unreal"),
            host: text(""),
            map: text("DM-Test"),
            portal: text(""),
            op: vec![text("Game=X")],
            port: 7777,
            valid: 1,
        },
        model: ME,
        approx_time: 1.5,
        first_deleted: ObjectRef::Null,
        text_blocks: [CORE; 16],
        travel_info: vec![StringPair {
            key: text("a"),
            value: text("b"),
        }],
    }
}

#[test]
fn levels() {
    check(
        NativeClass::Level,
        Native::Level(Box::new(level())),
        128,
        29,
    );
    let error = error(
        NativeClass::Level,
        Native::Level(Box::new(level())),
        128,
        97,
    );
    assert!(error.contains("levels before version 98"), "{}", error);
}

#[test]
fn terrain() {
    let info = TerrainInfo {
        sectors: vec![ME],
        vertices: vec![v(1.0), v(2.0)],
        sectors_x: 1,
        sectors_y: 2,
        face_normals: vec![TerrainNormalPair {
            normal1: v(0.0),
            normal2: v(1.0),
        }],
        to_world: coords(),
        to_heightmap: coords(),
        heightmap_x: 256,
        heightmap_y: 256,
        vertex_colors: vec![color()],
    };
    check(
        NativeClass::TerrainInfo,
        Native::TerrainInfo(info.clone()),
        128,
        29,
    );
    let error_text = error(
        NativeClass::TerrainInfo,
        Native::TerrainInfo(info),
        128,
        116,
    );
    assert!(
        error_text.contains("terrain before version 117"),
        "{}",
        error_text
    );

    let sector = TerrainSector {
        info: ME,
        quads_x: 16,
        quads_y: 16,
        offset_x: 0,
        offset_y: 16,
        bounding_box: bounds(),
        light_infos: vec![TerrainSectorLightInfo {
            light_actor: ME,
            visibility_bitmap: vec![1, 2],
        }],
    };
    check(
        NativeClass::TerrainSector,
        Native::TerrainSector(sector.clone()),
        128,
        29,
    );
    let error_text = error(
        NativeClass::TerrainSector,
        Native::TerrainSector(sector),
        128,
        116,
    );
    assert!(
        error_text.contains("terrain sectors before version 117"),
        "{}",
        error_text
    );

    check(
        NativeClass::TerrainPrimitive,
        Native::TerrainPrimitive(TerrainPrimitive {
            primitive: primitive(),
            info: ME,
        }),
        128,
        29,
    );
}

#[test]
fn primitives_palettes_and_procedural_sounds() {
    check(
        NativeClass::Primitive,
        Native::Primitive(primitive()),
        128,
        29,
    );
    check(
        NativeClass::Palette,
        Native::Palette(Palette {
            colors: vec![color(), color(), color()],
        }),
        128,
        29,
    );
    check(
        NativeClass::ProceduralSound,
        Native::ProceduralSound(ProceduralSound {
            likelihood: 1.0,
            base_sound: CORE,
            pitch_modification: 2.0,
            volume_modification: 3.0,
            pitch_variance: 4.0,
            volume_variance: 5.0,
        }),
        128,
        29,
    );
}

#[test]
fn a_sphere_before_version_62_has_no_radius() {
    let package = package_with(61, 0, Vec::new());
    let mut sphere = Sphere {
        center: v(1.0),
        radius: None,
    };
    let mut saver = Saver::new(61, 0);
    sphere.serialize(&mut saver).unwrap();
    let bytes = saver.into_bytes();
    assert_eq!(bytes.len(), 12);
    let mut loaded = Sphere::default();
    loaded
        .serialize(&mut Loader::new(&package, &bytes))
        .unwrap();
    assert_eq!(loaded, sphere);
}

#[test]
fn fonts() {
    let font = Font {
        characters: Some(vec![FontCharacter {
            start_u: 1,
            start_v: 2,
            u_size: 3,
            v_size: 4,
            texture_index: 0,
        }]),
        textures: Some(vec![CORE]),
        kerning: -1,
        char_remap: Some(vec![CharRemap {
            from: 0x41,
            to: 0x2014,
        }]),
        is_remapped: Some(1),
        ..Font::default()
    };
    check(NativeClass::Font, Native::Font(font), 128, 29);
    // Before licensee version 29, characters are in pages; before version
    // 69, nothing is remapped.
    let old = Font {
        pages: Some(vec![OldFontPage {
            texture: CORE,
            characters: vec![OldFontCharacter {
                start_u: 1,
                start_v: 2,
                u_size: 3,
                v_size: 4,
            }],
        }]),
        characters_per_page: Some(256),
        kerning: 0,
        ..Font::default()
    };
    check(NativeClass::Font, Native::Font(old), 68, 28);
}

#[test]
fn every_native_class() {
    let text = || FString::Latin1(b"x\0".to_vec());
    let cases = [
        (NativeClass::Object, Native::None, "Object"),
        (
            NativeClass::State,
            Native::State(Box::new(State {
                structure: Struct {
                    struct_flags: Some(0),
                    ..Struct::default()
                },
                ..State::default()
            })),
            "State",
        ),
        (
            NativeClass::Function,
            Native::Function(Box::new(Function {
                structure: Struct {
                    struct_flags: Some(0),
                    ..Struct::default()
                },
                ..Function::default()
            })),
            "Function",
        ),
        (
            NativeClass::Struct,
            Native::Struct(Box::new(Struct {
                struct_flags: Some(0),
                ..Struct::default()
            })),
            "Struct",
        ),
        (
            NativeClass::Property(PropertyKind::Int),
            Native::Property(Box::new(PropertyDecl {
                array_dim: 1,
                ..PropertyDecl::default()
            })),
            "Property",
        ),
        (
            NativeClass::Enum,
            Native::Enum(Enum {
                names: vec![ALPHA],
                ..Enum::default()
            }),
            "Enum",
        ),
        (
            NativeClass::Const,
            Native::Const(Const {
                value: text(),
                ..Const::default()
            }),
            "Const",
        ),
        (
            NativeClass::TextBuffer,
            Native::TextBuffer(TextBuffer {
                pos: 1,
                top: 2,
                text: text(),
            }),
            "TextBuffer",
        ),
        (
            NativeClass::Texture,
            Native::Texture(Texture {
                mips: vec![Mipmap {
                    data: lazy(vec![1, 2, 3, 4]),
                    u_size: 2,
                    v_size: 2,
                    u_bits: 1,
                    v_bits: 1,
                }],
            }),
            "Texture",
        ),
        (
            NativeClass::Sound,
            Native::Sound(Sound {
                file_type: ALPHA,
                likelihood: Some(1.0),
                data: lazy(vec![1, 2]),
            }),
            "Sound",
        ),
        (
            NativeClass::SoundGroup,
            Native::SoundGroup(SoundGroup {
                package: None,
                sounds: Some(vec![CORE]),
            }),
            "SoundGroup",
        ),
        (
            NativeClass::FireTexture,
            Native::FireTexture(FireTexture {
                texture: Texture {
                    mips: vec![Mipmap {
                        data: lazy(Vec::new()),
                        u_size: 2,
                        v_size: 2,
                        u_bits: 1,
                        v_bits: 1,
                    }],
                },
                sparks: vec![Spark {
                    spark_type: 1,
                    heat: 2,
                    x: 3,
                    y: 4,
                    byte_a: 5,
                    byte_b: 6,
                    byte_c: 7,
                    byte_d: 8,
                }],
            }),
            "FireTexture",
        ),
        (
            NativeClass::VertMesh,
            Native::VertMesh(Box::new(VertMesh {
                lod_mesh: lod_mesh(4),
                vertex_stream: AnimMeshVertexStream {
                    vertices: vec![AnimMeshVertex::default()],
                    revision: 1,
                },
                anim_verts: vec![1, 2],
                frame_keys: vec![0.5],
                anim_seqs: vec![MeshAnimSeq {
                    bookmark: Some(0.0),
                    ..MeshAnimSeq::default()
                }],
                anim_normals: vec![3],
                frame_verts: 2,
                anim_frames: 1,
                bounding_boxes: vec![bounds()],
                bounding_spheres: vec![sphere()],
            })),
            "VertMesh",
        ),
        (
            NativeClass::Prefab,
            Native::Prefab(Prefab {
                file_type: ALPHA,
                t3d_text: text(),
            }),
            "Prefab",
        ),
        (
            NativeClass::Palette,
            Native::Palette(Palette::default()),
            "Palette",
        ),
        (
            NativeClass::Font,
            Native::Font(Font {
                characters: Some(Vec::new()),
                textures: Some(Vec::new()),
                char_remap: Some(Vec::new()),
                is_remapped: Some(0),
                ..Font::default()
            }),
            "Font",
        ),
        (
            NativeClass::ProceduralSound,
            Native::ProceduralSound(ProceduralSound::default()),
            "ProceduralSound",
        ),
        (
            NativeClass::Primitive,
            Native::Primitive(primitive()),
            "Primitive",
        ),
        (
            NativeClass::StaticMesh,
            Native::StaticMesh(Box::new(StaticMesh {
                kdop_tree: Some(KDopTree::default()),
                max_smoothing_angles: Some(Vec::new()),
                authentication_key: Some(0),
                ..static_mesh()
            })),
            "StaticMesh",
        ),
        (
            NativeClass::StaticMeshInstance,
            Native::StaticMeshInstance(StaticMeshInstance::default()),
            "StaticMeshInstance",
        ),
        (
            NativeClass::Model,
            Native::Model(Box::new(model(
                None,
                Some(StaticLightMapTexture::default()),
            ))),
            "Model",
        ),
        (NativeClass::Polys, Native::Polys(Polys::default()), "Polys"),
        (
            NativeClass::KMeshProps,
            Native::KMeshProps(KMeshProps::default()),
            "KMeshProps",
        ),
        (
            NativeClass::ConvexVolume,
            Native::ConvexVolume(ConvexVolume {
                primitive: primitive(),
                ..ConvexVolume::default()
            }),
            "ConvexVolume",
        ),
        (
            NativeClass::LodMesh,
            Native::LodMesh(Box::new(LodMesh {
                primitive: primitive(),
                internal_version: 2,
                ..LodMesh::default()
            })),
            "LodMesh",
        ),
        (
            NativeClass::SkeletalMesh,
            Native::SkeletalMesh(Box::new(SkeletalMesh {
                lod_mesh: LodMesh {
                    primitive: primitive(),
                    internal_version: 2,
                    ..LodMesh::default()
                },
                lod_models: Some(Vec::new()),
                raw: Some(RawSkeletalMesh::default()),
                authentication_key: Some(0),
                collision: Some(SkeletalMeshCollision::default()),
                collision_static_mesh: Some(ObjectRef::Null),
                ..SkeletalMesh::default()
            })),
            "SkeletalMesh",
        ),
        (
            NativeClass::MeshAnimation,
            Native::MeshAnimation(MeshAnimation::default()),
            "MeshAnimation",
        ),
        (
            NativeClass::Level,
            Native::Level(Box::new(level())),
            "Level",
        ),
        (
            NativeClass::TerrainInfo,
            Native::TerrainInfo(TerrainInfo::default()),
            "TerrainInfo",
        ),
        (
            NativeClass::TerrainSector,
            Native::TerrainSector(TerrainSector::default()),
            "TerrainSector",
        ),
        (
            NativeClass::TerrainPrimitive,
            Native::TerrainPrimitive(TerrainPrimitive {
                primitive: primitive(),
                info: ObjectRef::Null,
            }),
            "TerrainPrimitive",
        ),
    ];
    for (class, native, name) in cases {
        assert_eq!(class.name(), name);
        let offsets = matches!(
            name,
            "Texture" | "FireTexture" | "Sound" | "StaticMesh" | "Model" | "SkeletalMesh"
        );
        assert_eq!(class.has_absolute_offsets(), offsets, "{}", name);
        check(class, native, 128, 29);
    }
    assert_eq!(NativeClass::Class.name(), "Class");
}

#[test]
fn a_class_ends_where_its_defaults_start() {
    let mut class = Native::Class(Box::new(Class {
        state: State {
            structure: Struct {
                struct_flags: Some(0),
                ..Struct::default()
            },
            ..State::default()
        },
        within: Some(ObjectRef::Null),
        config_name: Some(NameRef(0)),
        hide_categories: Some(Vec::new()),
        ..Class::default()
    }));
    let bytes = class.write(128, 29, 0).unwrap();
    let data = [bytes.clone(), vec![0]].concat();
    let package = package_with(128, 29, data);
    let read = package.native_data(0, &NativeClass::Class, 0).unwrap();
    assert_eq!(read.native, class);
    assert_eq!((read.start, read.end), (0, bytes.len()));
    let round_trip = package.round_trip(0, &NativeClass::Class, 0).unwrap();
    assert_eq!(round_trip.difference, None);
}

#[test]
fn native_data_must_end_where_the_data_does() {
    let data = vec![0, 1, 2];
    let package = package_with(128, 29, data);
    let error = package
        .native_data(0, &NativeClass::Object, 1)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "invalid data at offset 1: 2 bytes are left after the native data (Object)"
    );
}

#[test]
fn textures_before_version_84_arent_read() {
    let error = error(
        NativeClass::Texture,
        Native::Texture(Texture::default()),
        128,
        83,
    );
    assert!(error.contains("textures before version 84"), "{}", error);
    let error = error_of(
        NativeClass::FireTexture,
        Native::FireTexture(FireTexture::default()),
    );
    assert!(error.contains("textures before version 84"), "{}", error);
}

fn error_of(class: NativeClass, native: Native) -> String {
    error(class, native, 128, 83)
}

#[test]
fn a_round_trip_finds_where_the_bytes_differ() {
    // A compact index written in more bytes than it needs reads the same,
    // but is written back shorter.
    let data = [vec![0], vec![0x41, 0x00], vec![0; 4]].concat();
    let package = package_with(128, 29, data);
    let class = NativeClass::Palette;
    let round_trip = package.round_trip(0, &class, 1).unwrap();
    assert_eq!(round_trip.difference, Some(1));
    assert_eq!(round_trip.size, 6);
}

#[test]
fn a_round_trip_of_what_cant_be_read_fails() {
    let package = package_with(128, 29, vec![0]);
    let error = package.round_trip(0, &NativeClass::Palette, 1).unwrap_err();
    assert!(error.to_string().contains("the data ends"), "{}", error);
}

#[test]
fn a_lazy_array_of_what_cant_be_written_cant_be_written() {
    let mut array = lazy(vec![StaticMesh::default()]);
    assert!(array.serialize(&mut Saver::new(100, 0)).is_err());
}

#[test]
fn native_classes_by_path() {
    use usagi_pkg::NativeKind;
    assert_eq!(
        NativeClass::of_path("engine.staticmesh"),
        Some(NativeKind::Known(NativeClass::StaticMesh))
    );
    assert_eq!(
        NativeClass::of_path("Core.IntProperty"),
        Some(NativeKind::Known(NativeClass::Property(PropertyKind::Int)))
    );
    assert_eq!(
        NativeClass::of_path("Engine.Animation"),
        Some(NativeKind::Unread)
    );
    assert_eq!(NativeClass::of_path("Engine.Pawn"), None);
    assert_eq!(NativeClass::of_path("Engine.IntProperty"), None);
    assert_eq!(NativeClass::of_path("Texture"), None);
}
