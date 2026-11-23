//! The engine's maths types, as native data stores them (`Core/Inc/UnMath.h`
//! and `FColor` in `Engine/Inc/UnRenderResource.h`).

use crate::Result;
use crate::archive::{Archive, Serialize, native_struct};

native_struct! {
    /// `FVector`.
    pub struct Vector {
        pub x: f32,
        pub y: f32,
        pub z: f32,
    }
}

native_struct! {
    /// `FPlane`: a vector and `W`.
    pub struct Plane {
        pub x: f32,
        pub y: f32,
        pub z: f32,
        pub w: f32,
    }
}

/// `FSphere`: a centre and, from version 62, a radius (`W`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sphere {
    pub center: Vector,
    /// From version 62.
    pub radius: Option<f32>,
}

impl Serialize for Sphere {
    fn serialize(&mut self, ar: &mut dyn Archive) -> Result<()> {
        self.center.serialize(ar)?;
        if ar.version() > 61 {
            self.radius.get_or_insert(0.0).serialize(ar)?;
        }
        Ok(())
    }
}

native_struct! {
    /// `FBox`: its corners, and whether it's valid.
    pub struct BoundingBox {
        pub min: Vector,
        pub max: Vector,
        pub is_valid: u8,
    }
}

native_struct! {
    /// `FRotator`.
    pub struct Rotator {
        pub pitch: i32,
        pub yaw: i32,
        pub roll: i32,
    }
}

native_struct! {
    /// `FQuat`.
    pub struct Quat {
        pub x: f32,
        pub y: f32,
        pub z: f32,
        pub w: f32,
    }
}

native_struct! {
    /// `FCoords`: an origin and three axes.
    pub struct Coords {
        pub origin: Vector,
        pub x_axis: Vector,
        pub y_axis: Vector,
        pub z_axis: Vector,
    }
}

native_struct! {
    /// `FMatrix`: 4 rows of 4.
    pub struct Matrix {
        pub m: [[f32; 4]; 4],
    }
}

native_struct! {
    /// `FColor`, as its `operator<<` writes it: `R`, `G`, `B`, `A`. (A
    /// `Color` property, stored by its declaration, is `B`, `G`, `R`, `A`.)
    pub struct Color {
        pub r: u8,
        pub g: u8,
        pub b: u8,
        pub a: u8,
    }
}
