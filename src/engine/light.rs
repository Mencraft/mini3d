//! Light sources: directional (sun), point and spot lights.

use glam::Vec3;

/// The most lights the shader evaluates per frame; extra enabled lights are ignored.
pub const MAX_LIGHTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    /// Parallel rays travelling along `direction`, like the sun. Can cast shadows.
    Directional { direction: Vec3 },
    /// Shines in all directions from `position`, fading out completely at `range`.
    Point { position: Vec3, range: f32 },
    /// A cone along `direction`; full strength inside `inner_angle`, fading to zero at
    /// `outer_angle` (both half-angles in radians).
    Spot { position: Vec3, direction: Vec3, range: f32, inner_angle: f32, outer_angle: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    /// Linear RGB color.
    pub color: Vec3,
    /// Brightness multiplier. Point and spot lights fall off with distance squared,
    /// so they need much larger values than directional lights (e.g. 20–60 vs 2–4).
    pub intensity: f32,
    /// Only the first enabled directional light with this set casts shadows.
    pub cast_shadows: bool,
    pub enabled: bool,
}

impl Light {
    fn new(kind: LightKind, color: [f32; 3], intensity: f32) -> Self {
        Self { kind, color: color.into(), intensity, cast_shadows: false, enabled: true }
    }

    pub fn directional(direction: Vec3, color: [f32; 3], intensity: f32) -> Self {
        Self::new(LightKind::Directional { direction }, color, intensity)
    }

    pub fn point(position: Vec3, color: [f32; 3], intensity: f32, range: f32) -> Self {
        Self::new(LightKind::Point { position, range }, color, intensity)
    }

    /// Angles are half-angles in degrees.
    pub fn spot(
        position: Vec3,
        direction: Vec3,
        color: [f32; 3],
        intensity: f32,
        range: f32,
        inner_degrees: f32,
        outer_degrees: f32,
    ) -> Self {
        let kind = LightKind::Spot {
            position,
            direction,
            range,
            inner_angle: inner_degrees.to_radians(),
            outer_angle: outer_degrees.max(inner_degrees + 0.1).to_radians(),
        };
        Self::new(kind, color, intensity)
    }

    pub fn with_shadows(mut self) -> Self {
        self.cast_shadows = true;
        self
    }

    /// Moves a point or spot light; does nothing for directional lights.
    pub fn set_position(&mut self, p: Vec3) {
        match &mut self.kind {
            LightKind::Point { position, .. } | LightKind::Spot { position, .. } => *position = p,
            LightKind::Directional { .. } => {}
        }
    }

    pub fn position(&self) -> Option<Vec3> {
        match self.kind {
            LightKind::Point { position, .. } | LightKind::Spot { position, .. } => Some(position),
            LightKind::Directional { .. } => None,
        }
    }
}
