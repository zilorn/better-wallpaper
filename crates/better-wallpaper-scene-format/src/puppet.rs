use thiserror::Error;

const MODEL_MAGIC: &[u8] = b"MDLV0017\0";
const SKELETON_MAGIC: &[u8] = b"MDLS0002\0";
const ANIMATION_MAGIC: &[u8] = b"MDLA0004\0";
const VERTEX_STRIDE: usize = 80;
const TRANSFORM_STRIDE: usize = 36;
const MAX_VERTICES: usize = u16::MAX as usize;
const MAX_INDICES: usize = 3 * 1024 * 1024;
const MAX_BONES: usize = 256;
const MAX_ANIMATIONS: usize = 256;
const MAX_FRAMES: usize = 10_000;
const MAX_STRING_BYTES: usize = 4096;

#[derive(Debug, Error, PartialEq)]
pub enum PuppetError {
    #[error("puppet model is truncated while reading {0}")]
    Truncated(&'static str),
    #[error("unsupported puppet model section: {0}")]
    Unsupported(&'static str),
    #[error("invalid puppet model value: {0}")]
    Invalid(&'static str),
}

#[derive(Debug, Clone)]
pub struct PuppetVertex {
    pub position: [f32; 3],
    pub bone_indices: [u32; 4],
    pub bone_weights: [f32; 4],
    pub uv: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct PuppetBone {
    pub parent: Option<usize>,
    pub bind_local: [f32; 16],
}

#[derive(Debug, Clone, Copy)]
pub struct PuppetTransform {
    pub translation: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct PuppetAnimation {
    pub id: u32,
    pub name: String,
    pub fps: f32,
    pub frame_count: usize,
    /// Bone-major tracks. Authored files contain a duplicate loop endpoint.
    pub tracks: Vec<Vec<PuppetTransform>>,
}

#[derive(Debug, Clone)]
pub struct PuppetModel {
    pub vertices: Vec<PuppetVertex>,
    pub indices: Vec<u16>,
    pub bones: Vec<PuppetBone>,
    pub animations: Vec<PuppetAnimation>,
}

impl PuppetModel {
    pub fn parse(bytes: &[u8]) -> Result<Self, PuppetError> {
        let mut cursor = Cursor::new(bytes);
        cursor.expect(MODEL_MAGIC, "MDLV0017 header")?;
        let vertex_format = cursor.u32("vertex format")?;
        if vertex_format != 0x0180_0009 {
            return Err(PuppetError::Unsupported("vertex format"));
        }
        if cursor.u32("mesh count")? != 1 {
            return Err(PuppetError::Unsupported("mesh count"));
        }
        let material_count = cursor.u32("material count")? as usize;
        if material_count == 0 || material_count > 16 {
            return Err(PuppetError::Invalid("material count"));
        }
        cursor.skip(
            material_count
                .checked_mul(64)
                .ok_or(PuppetError::Invalid("material table size"))?,
            "material table",
        )?;

        let vertex_bytes = cursor.u32("vertex buffer size")? as usize;
        if !vertex_bytes.is_multiple_of(VERTEX_STRIDE)
            || vertex_bytes / VERTEX_STRIDE > MAX_VERTICES
        {
            return Err(PuppetError::Invalid("vertex buffer size"));
        }
        let vertex_count = vertex_bytes / VERTEX_STRIDE;
        let vertex_buffer = cursor.bytes(vertex_bytes, "vertex buffer")?;
        let mut vertices = Vec::with_capacity(vertex_count);
        for record in vertex_buffer.chunks_exact(VERTEX_STRIDE) {
            let position = [f32_at(record, 0), f32_at(record, 4), f32_at(record, 8)];
            let bone_indices = [
                u32_at(record, 40),
                u32_at(record, 44),
                u32_at(record, 48),
                u32_at(record, 52),
            ];
            let bone_weights = [
                f32_at(record, 56),
                f32_at(record, 60),
                f32_at(record, 64),
                f32_at(record, 68),
            ];
            let uv = [f32_at(record, 72), f32_at(record, 76)];
            if !position
                .iter()
                .chain(bone_weights.iter())
                .chain(uv.iter())
                .all(|v| v.is_finite())
                || bone_weights.iter().any(|weight| *weight < 0.0)
            {
                return Err(PuppetError::Invalid("vertex data"));
            }
            vertices.push(PuppetVertex {
                position,
                bone_indices,
                bone_weights,
                uv,
            });
        }

        let index_bytes = cursor.u32("index buffer size")? as usize;
        if !index_bytes.is_multiple_of(2)
            || !(index_bytes / 2).is_multiple_of(3)
            || index_bytes / 2 > MAX_INDICES
        {
            return Err(PuppetError::Invalid("index buffer size"));
        }
        let mut indices = Vec::with_capacity(index_bytes / 2);
        for index in cursor.bytes(index_bytes, "index buffer")?.chunks_exact(2) {
            let index = u16::from_le_bytes([index[0], index[1]]);
            if usize::from(index) >= vertex_count {
                return Err(PuppetError::Invalid("index outside vertex buffer"));
            }
            indices.push(index);
        }

        cursor.expect(SKELETON_MAGIC, "MDLS0002 header")?;
        let skeleton_end = cursor.u32("skeleton end offset")? as usize;
        let bone_count = cursor.u32("bone count")? as usize;
        if bone_count == 0 || bone_count > MAX_BONES {
            return Err(PuppetError::Invalid("bone count"));
        }
        let mut bones = Vec::with_capacity(bone_count);
        for _ in 0..bone_count {
            cursor.skip(1, "bone marker")?;
            let _flags = cursor.u32("bone flags")?;
            let parent = cursor.i32("bone parent")?;
            if parent < -1 || (parent >= 0 && parent as usize >= bone_count) {
                return Err(PuppetError::Invalid("bone parent"));
            }
            if cursor.u32("bone matrix size")? != 64 {
                return Err(PuppetError::Unsupported("bone matrix size"));
            }
            let mut bind_local = [0.0; 16];
            for value in &mut bind_local {
                *value = cursor.f32("bone matrix")?;
                if !value.is_finite() {
                    return Err(PuppetError::Invalid("bone matrix"));
                }
            }
            // Constraint metadata is currently not executed, but the bounded
            // string must still be consumed so the following section is safe.
            let _metadata = cursor.c_string("bone metadata")?;
            bones.push(PuppetBone {
                parent: (parent >= 0).then_some(parent as usize),
                bind_local,
            });
        }
        if skeleton_end < cursor.offset || skeleton_end > bytes.len() {
            return Err(PuppetError::Invalid("skeleton end offset"));
        }
        cursor.offset = skeleton_end;

        cursor.expect(ANIMATION_MAGIC, "MDLA0004 header")?;
        let animation_end = cursor.u32("animation end offset")? as usize;
        let animation_count = cursor.u32("animation count")? as usize;
        if animation_count > MAX_ANIMATIONS {
            return Err(PuppetError::Invalid("animation count"));
        }
        let mut animations = Vec::with_capacity(animation_count);
        for _ in 0..animation_count {
            let id = cursor.u32("animation id")?;
            let _flags = cursor.u32("animation flags")?;
            let name = cursor.c_string("animation name")?.to_owned();
            let _mode = cursor.c_string("animation mode")?;
            let fps = cursor.f32("animation fps")?;
            let frame_count = cursor.u32("animation frame count")? as usize;
            let _reserved = cursor.u32("animation reserved field")?;
            let track_count = cursor.u32("animation track count")? as usize;
            if !fps.is_finite()
                || fps <= 0.0
                || frame_count == 0
                || frame_count > MAX_FRAMES
                || track_count != bone_count
            {
                return Err(PuppetError::Invalid("animation dimensions"));
            }
            let mut tracks = Vec::with_capacity(track_count);
            for _ in 0..track_count {
                let _track_flags = cursor.u32("animation track flags")?;
                let track_bytes = cursor.u32("animation track size")? as usize;
                if !track_bytes.is_multiple_of(TRANSFORM_STRIDE)
                    || track_bytes / TRANSFORM_STRIDE < frame_count
                    || track_bytes / TRANSFORM_STRIDE > frame_count + 1
                {
                    return Err(PuppetError::Invalid("animation track size"));
                }
                let mut track = Vec::with_capacity(frame_count);
                for frame in 0..track_bytes / TRANSFORM_STRIDE {
                    let transform = PuppetTransform {
                        translation: [
                            cursor.f32("animation translation")?,
                            cursor.f32("animation translation")?,
                            cursor.f32("animation translation")?,
                        ],
                        rotation: [
                            cursor.f32("animation rotation")?,
                            cursor.f32("animation rotation")?,
                            cursor.f32("animation rotation")?,
                        ],
                        scale: [
                            cursor.f32("animation scale")?,
                            cursor.f32("animation scale")?,
                            cursor.f32("animation scale")?,
                        ],
                    };
                    if !transform
                        .translation
                        .iter()
                        .chain(transform.rotation.iter())
                        .chain(transform.scale.iter())
                        .all(|value| value.is_finite())
                    {
                        return Err(PuppetError::Invalid("animation transform"));
                    }
                    if frame < frame_count {
                        track.push(transform);
                    }
                }
                tracks.push(track);
            }
            animations.push(PuppetAnimation {
                id,
                name,
                fps,
                frame_count,
                tracks,
            });
        }
        if animation_end >= bytes.len() || cursor.offset > animation_end + 1 {
            return Err(PuppetError::Invalid("animation end offset"));
        }

        for vertex in &vertices {
            for (&bone, &weight) in vertex.bone_indices.iter().zip(vertex.bone_weights.iter()) {
                if weight > 0.0 && bone as usize >= bone_count {
                    return Err(PuppetError::Invalid("vertex bone index"));
                }
            }
        }
        validate_hierarchy(&bones)?;
        Ok(Self {
            vertices,
            indices,
            bones,
            animations,
        })
    }

    pub fn skinned_positions(
        &self,
        animation: &PuppetAnimation,
        frame: usize,
    ) -> Result<Vec<[f32; 3]>, PuppetError> {
        if frame >= animation.frame_count || animation.tracks.len() != self.bones.len() {
            return Err(PuppetError::Invalid("animation frame"));
        }
        let bind_global = global_matrices(
            &self.bones,
            self.bones.iter().map(|bone| bone.bind_local).collect(),
        )?;
        let animated_local = animation
            .tracks
            .iter()
            .map(|track| transform_matrix(track[frame]))
            .collect();
        let animated_global = global_matrices(&self.bones, animated_local)?;
        let mut skin = Vec::with_capacity(self.bones.len());
        for (animated, bind) in animated_global.into_iter().zip(bind_global) {
            skin.push(matrix_mul(animated, matrix_inverse(bind)?));
        }

        let mut positions = Vec::with_capacity(self.vertices.len());
        for vertex in &self.vertices {
            let input = [
                vertex.position[0],
                vertex.position[1],
                vertex.position[2],
                1.0,
            ];
            let mut output = [0.0; 3];
            let mut total_weight = 0.0;
            for (&bone, &weight) in vertex.bone_indices.iter().zip(vertex.bone_weights.iter()) {
                if weight <= 0.0 {
                    continue;
                }
                let transformed = matrix_vector_mul(skin[bone as usize], input);
                for axis in 0..3 {
                    output[axis] += transformed[axis] * weight;
                }
                total_weight += weight;
            }
            if total_weight <= f32::EPSILON {
                output.copy_from_slice(&vertex.position);
            }
            positions.push(output);
        }
        Ok(positions)
    }
}

fn validate_hierarchy(bones: &[PuppetBone]) -> Result<(), PuppetError> {
    for start in 0..bones.len() {
        let mut current = Some(start);
        let mut depth = 0;
        while let Some(index) = current {
            depth += 1;
            if depth > bones.len() {
                return Err(PuppetError::Invalid("bone hierarchy cycle"));
            }
            current = bones[index].parent;
        }
    }
    Ok(())
}

fn global_matrices(
    bones: &[PuppetBone],
    local: Vec<[f32; 16]>,
) -> Result<Vec<[f32; 16]>, PuppetError> {
    let mut output = vec![None; bones.len()];
    for index in 0..bones.len() {
        resolve_global(index, bones, &local, &mut output, 0)?;
    }
    Ok(output.into_iter().flatten().collect())
}

fn resolve_global(
    index: usize,
    bones: &[PuppetBone],
    local: &[[f32; 16]],
    output: &mut [Option<[f32; 16]>],
    depth: usize,
) -> Result<[f32; 16], PuppetError> {
    if let Some(matrix) = output[index] {
        return Ok(matrix);
    }
    if depth >= bones.len() {
        return Err(PuppetError::Invalid("bone hierarchy cycle"));
    }
    let global = if let Some(parent) = bones[index].parent {
        matrix_mul(
            resolve_global(parent, bones, local, output, depth + 1)?,
            local[index],
        )
    } else {
        local[index]
    };
    output[index] = Some(global);
    Ok(global)
}

fn transform_matrix(transform: PuppetTransform) -> [f32; 16] {
    let [rx, ry, rz] = transform.rotation;
    let (sx, cx) = rx.sin_cos();
    let (sy, cy) = ry.sin_cos();
    let (sz, cz) = rz.sin_cos();
    let rotation_x = [
        1.0, 0.0, 0.0, 0.0, 0.0, cx, sx, 0.0, 0.0, -sx, cx, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let rotation_y = [
        cy, 0.0, -sy, 0.0, 0.0, 1.0, 0.0, 0.0, sy, 0.0, cy, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let rotation_z = [
        cz, sz, 0.0, 0.0, -sz, cz, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let scale = [
        transform.scale[0],
        0.0,
        0.0,
        0.0,
        0.0,
        transform.scale[1],
        0.0,
        0.0,
        0.0,
        0.0,
        transform.scale[2],
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ];
    let mut matrix = matrix_mul(
        rotation_z,
        matrix_mul(rotation_y, matrix_mul(rotation_x, scale)),
    );
    matrix[12] = transform.translation[0];
    matrix[13] = transform.translation[1];
    matrix[14] = transform.translation[2];
    matrix
}

fn matrix_mul(left: [f32; 16], right: [f32; 16]) -> [f32; 16] {
    let mut output = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            output[column * 4 + row] = (0..4)
                .map(|index| left[index * 4 + row] * right[column * 4 + index])
                .sum();
        }
    }
    output
}

fn matrix_vector_mul(matrix: [f32; 16], vector: [f32; 4]) -> [f32; 4] {
    let mut output = [0.0; 4];
    for row in 0..4 {
        output[row] = (0..4)
            .map(|column| matrix[column * 4 + row] * vector[column])
            .sum();
    }
    output
}

fn matrix_inverse(matrix: [f32; 16]) -> Result<[f32; 16], PuppetError> {
    let mut augmented = [[0.0; 8]; 4];
    for row in 0..4 {
        for column in 0..4 {
            augmented[row][column] = matrix[column * 4 + row];
        }
        augmented[row][row + 4] = 1.0;
    }
    for pivot in 0..4 {
        let best = (pivot..4)
            .max_by(|left, right| {
                augmented[*left][pivot]
                    .abs()
                    .total_cmp(&augmented[*right][pivot].abs())
            })
            .ok_or(PuppetError::Invalid("singular bone matrix"))?;
        if augmented[best][pivot].abs() <= 1.0e-8 {
            return Err(PuppetError::Invalid("singular bone matrix"));
        }
        augmented.swap(pivot, best);
        let divisor = augmented[pivot][pivot];
        for value in &mut augmented[pivot] {
            *value /= divisor;
        }
        for row in 0..4 {
            if row == pivot {
                continue;
            }
            let factor = augmented[row][pivot];
            let pivot_values = augmented[pivot];
            for (value, pivot_value) in augmented[row].iter_mut().zip(pivot_values) {
                *value -= factor * pivot_value;
            }
        }
    }
    let mut inverse = [0.0; 16];
    for row in 0..4 {
        for column in 0..4 {
            inverse[column * 4 + row] = augmented[row][column + 4];
        }
    }
    Ok(inverse)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn bytes(&mut self, length: usize, field: &'static str) -> Result<&'a [u8], PuppetError> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(PuppetError::Truncated(field))?;
        let output = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(output)
    }

    fn skip(&mut self, length: usize, field: &'static str) -> Result<(), PuppetError> {
        self.bytes(length, field).map(|_| ())
    }

    fn expect(&mut self, expected: &[u8], field: &'static str) -> Result<(), PuppetError> {
        if self.bytes(expected.len(), field)? != expected {
            return Err(PuppetError::Unsupported(field));
        }
        Ok(())
    }

    fn u32(&mut self, field: &'static str) -> Result<u32, PuppetError> {
        let bytes = self.bytes(4, field)?;
        Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")))
    }

    fn i32(&mut self, field: &'static str) -> Result<i32, PuppetError> {
        let bytes = self.bytes(4, field)?;
        Ok(i32::from_le_bytes(bytes.try_into().expect("four bytes")))
    }

    fn f32(&mut self, field: &'static str) -> Result<f32, PuppetError> {
        let bytes = self.bytes(4, field)?;
        Ok(f32::from_le_bytes(bytes.try_into().expect("four bytes")))
    }

    fn c_string(&mut self, field: &'static str) -> Result<&'a str, PuppetError> {
        let remaining = self
            .bytes
            .get(self.offset..)
            .ok_or(PuppetError::Truncated(field))?;
        let length = remaining
            .iter()
            .take(MAX_STRING_BYTES + 1)
            .position(|byte| *byte == 0)
            .filter(|length| *length <= MAX_STRING_BYTES)
            .ok_or(PuppetError::Invalid(field))?;
        let bytes = self.bytes(length + 1, field)?;
        std::str::from_utf8(&bytes[..length]).map_err(|_| PuppetError::Invalid(field))
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated stride"),
    )
}

fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated stride"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_and_inverse_round_trip() {
        let matrix = transform_matrix(PuppetTransform {
            translation: [20.0, -10.0, 3.0],
            rotation: [0.0, 0.0, 0.25],
            scale: [2.0, 0.5, 1.0],
        });
        let point = [7.0, 9.0, 1.0, 1.0];
        let transformed = matrix_vector_mul(matrix, point);
        let restored = matrix_vector_mul(matrix_inverse(matrix).unwrap(), transformed);
        for (actual, expected) in restored.into_iter().zip(point) {
            assert!((actual - expected).abs() < 0.0001);
        }
    }
}
