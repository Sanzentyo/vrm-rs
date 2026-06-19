use glam::Vec3;
use thiserror::Error;

use crate::{GltfMorphTargetData, GltfPrimitiveData, GltfSkinData};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OptimizeOptions {
    pub remove_degenerate_triangles: bool,
    pub compact_vertices: bool,
    pub normalize_skin_weights: bool,
    pub compact_joint_palette: bool,
    pub remove_empty_morph_targets: bool,
    pub morph_epsilon: f32,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self {
            remove_degenerate_triangles: true,
            compact_vertices: true,
            normalize_skin_weights: true,
            compact_joint_palette: true,
            remove_empty_morph_targets: true,
            morph_epsilon: 0.000001,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OptimizeReport {
    pub removed_degenerate_triangles: usize,
    pub removed_vertices: usize,
    pub removed_morph_targets: usize,
    pub vertex_remap: VertexRemap,
    pub joint_compaction: Option<JointPaletteCompaction>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VertexRemap {
    pub new_to_old: Vec<usize>,
    pub old_to_new: Vec<Option<usize>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JointPaletteCompaction {
    pub new_to_old: Vec<usize>,
    pub old_to_new: Vec<Option<usize>>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OptimizeError {
    #[error("attribute {attribute} length mismatch: expected {expected}, got {actual}")]
    InconsistentAttribute {
        attribute: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("index {index} is out of bounds for {vertex_count} vertices")]
    IndexOutOfBounds { index: u32, vertex_count: usize },
    #[error("index count {count} is not a triangle list")]
    IndexCountNotTriangleList { count: usize },
    #[error("joint {joint} is out of bounds for {joint_count} joints")]
    JointOutOfBounds { joint: u16, joint_count: usize },
}

pub fn optimize_primitive(
    primitive: &mut GltfPrimitiveData,
    skin_joint_count: usize,
    options: OptimizeOptions,
) -> Result<OptimizeReport, OptimizeError> {
    validate_parallel_attributes(primitive)?;
    validate_indices(primitive)?;

    let removed_degenerate_triangles = if options.remove_degenerate_triangles {
        remove_degenerate_triangles(primitive)?
    } else {
        0
    };

    if options.normalize_skin_weights {
        normalize_skin_weights(primitive);
    }

    let removed_morph_targets = if options.remove_empty_morph_targets {
        remove_empty_morph_targets(primitive, options.morph_epsilon)
    } else {
        0
    };

    let vertex_remap = if options.compact_vertices {
        compact_vertices(primitive)
    } else {
        identity_vertex_remap(primitive.positions.len())
    };

    validate_joints(primitive, skin_joint_count)?;
    let joint_compaction = if options.compact_joint_palette {
        compact_primitive_joint_palette(primitive, skin_joint_count)?
    } else {
        None
    };

    Ok(OptimizeReport {
        removed_degenerate_triangles,
        removed_vertices: vertex_remap
            .old_to_new
            .iter()
            .filter(|entry| entry.is_none())
            .count(),
        removed_morph_targets,
        vertex_remap,
        joint_compaction,
    })
}

pub fn apply_joint_compaction_to_skin(
    skin: &mut GltfSkinData,
    compaction: &JointPaletteCompaction,
) {
    skin.joints = compaction
        .new_to_old
        .iter()
        .filter_map(|old| skin.joints.get(*old).copied())
        .collect();
    skin.inverse_bind_matrices = compaction
        .new_to_old
        .iter()
        .filter_map(|old| skin.inverse_bind_matrices.get(*old).copied())
        .collect();
}

fn validate_parallel_attributes(primitive: &GltfPrimitiveData) -> Result<(), OptimizeError> {
    let expected = primitive.positions.len();
    validate_len("NORMAL", primitive.normals.len(), expected)?;
    validate_len("TANGENT", primitive.tangents.len(), expected)?;
    validate_len("TEXCOORD_0", primitive.tex_coords_0.len(), expected)?;
    validate_len("COLOR_0", primitive.colors_0.len(), expected)?;
    validate_len("JOINTS_0", primitive.joints_0.len(), expected)?;
    validate_len("WEIGHTS_0", primitive.weights_0.len(), expected)?;
    for target in &primitive.morph_targets {
        validate_len("MORPH_POSITION", target.positions.len(), expected)?;
        validate_len("MORPH_NORMAL", target.normals.len(), expected)?;
        validate_len("MORPH_TANGENT", target.tangents.len(), expected)?;
    }
    Ok(())
}

fn validate_len(
    attribute: &'static str,
    actual: usize,
    expected: usize,
) -> Result<(), OptimizeError> {
    if actual == 0 || actual == expected {
        Ok(())
    } else {
        Err(OptimizeError::InconsistentAttribute {
            attribute,
            expected,
            actual,
        })
    }
}

fn validate_indices(primitive: &GltfPrimitiveData) -> Result<(), OptimizeError> {
    if !primitive.indices.len().is_multiple_of(3) {
        return Err(OptimizeError::IndexCountNotTriangleList {
            count: primitive.indices.len(),
        });
    }
    let vertex_count = primitive.positions.len();
    if let Some(index) = primitive
        .indices
        .iter()
        .copied()
        .find(|index| *index as usize >= vertex_count)
    {
        return Err(OptimizeError::IndexOutOfBounds {
            index,
            vertex_count,
        });
    }
    Ok(())
}

fn validate_joints(
    primitive: &GltfPrimitiveData,
    skin_joint_count: usize,
) -> Result<(), OptimizeError> {
    primitive
        .joints_0
        .iter()
        .zip(&primitive.weights_0)
        .flat_map(|(joints, weights)| joints.iter().copied().zip(*weights))
        .filter(|(_, weight)| *weight > f32::EPSILON)
        .map(|(joint, _)| joint)
        .find(|joint| *joint as usize >= skin_joint_count)
        .map_or(Ok(()), |joint| {
            Err(OptimizeError::JointOutOfBounds {
                joint,
                joint_count: skin_joint_count,
            })
        })
}

fn remove_degenerate_triangles(primitive: &mut GltfPrimitiveData) -> Result<usize, OptimizeError> {
    let original = primitive.indices.len() / 3;
    primitive.indices = primitive
        .indices
        .chunks_exact(3)
        .filter(|triangle| !is_degenerate_triangle(primitive, triangle))
        .flatten()
        .copied()
        .collect();
    Ok(original - primitive.indices.len() / 3)
}

fn is_degenerate_triangle(primitive: &GltfPrimitiveData, triangle: &[u32]) -> bool {
    if triangle[0] == triangle[1] || triangle[1] == triangle[2] || triangle[0] == triangle[2] {
        return true;
    }
    let [a, b, c] = triangle else {
        return true;
    };
    let Some(a) = primitive
        .positions
        .get(*a as usize)
        .copied()
        .map(Vec3::from_array)
    else {
        return true;
    };
    let Some(b) = primitive
        .positions
        .get(*b as usize)
        .copied()
        .map(Vec3::from_array)
    else {
        return true;
    };
    let Some(c) = primitive
        .positions
        .get(*c as usize)
        .copied()
        .map(Vec3::from_array)
    else {
        return true;
    };
    (b - a).cross(c - a).length_squared() <= f32::EPSILON
}

fn normalize_skin_weights(primitive: &mut GltfPrimitiveData) {
    for weights in &mut primitive.weights_0 {
        let sum = weights
            .iter()
            .copied()
            .filter(|weight| *weight > 0.0)
            .sum::<f32>();
        if sum > f32::EPSILON {
            for weight in weights {
                *weight = (*weight).max(0.0) / sum;
            }
        }
    }
}

fn remove_empty_morph_targets(primitive: &mut GltfPrimitiveData, epsilon: f32) -> usize {
    let before = primitive.morph_targets.len();
    primitive
        .morph_targets
        .retain(|target| !is_empty_morph_target(target, epsilon));
    before - primitive.morph_targets.len()
}

fn is_empty_morph_target(target: &GltfMorphTargetData, epsilon: f32) -> bool {
    target
        .positions
        .iter()
        .all(|value| vec3_abs_max(*value) <= epsilon)
        && target
            .normals
            .iter()
            .all(|value| vec3_abs_max(*value) <= epsilon)
        && target
            .tangents
            .iter()
            .all(|value| vec3_abs_max(*value) <= epsilon)
}

fn vec3_abs_max(value: [f32; 3]) -> f32 {
    value
        .into_iter()
        .map(f32::abs)
        .fold(0.0, |acc, value| acc.max(value))
}

fn compact_vertices(primitive: &mut GltfPrimitiveData) -> VertexRemap {
    let mut used = vec![false; primitive.positions.len()];
    if primitive.indices.is_empty() {
        used.fill(true);
    } else {
        for index in &primitive.indices {
            used[*index as usize] = true;
        }
    }
    let new_to_old = used
        .iter()
        .enumerate()
        .filter_map(|(old, used)| (*used).then_some(old))
        .collect::<Vec<_>>();
    let mut old_to_new = vec![None; primitive.positions.len()];
    for (new, old) in new_to_old.iter().copied().enumerate() {
        old_to_new[old] = Some(new);
    }

    primitive.positions = remap_vec(&primitive.positions, &new_to_old);
    primitive.normals = remap_optional_vec(&primitive.normals, &new_to_old);
    primitive.tangents = remap_optional_vec(&primitive.tangents, &new_to_old);
    primitive.tex_coords_0 = remap_optional_vec(&primitive.tex_coords_0, &new_to_old);
    primitive.colors_0 = remap_optional_vec(&primitive.colors_0, &new_to_old);
    primitive.joints_0 = remap_optional_vec(&primitive.joints_0, &new_to_old);
    primitive.weights_0 = remap_optional_vec(&primitive.weights_0, &new_to_old);
    for target in &mut primitive.morph_targets {
        target.positions = remap_optional_vec(&target.positions, &new_to_old);
        target.normals = remap_optional_vec(&target.normals, &new_to_old);
        target.tangents = remap_optional_vec(&target.tangents, &new_to_old);
    }
    for index in &mut primitive.indices {
        *index = old_to_new[*index as usize].expect("referenced vertex is mapped") as u32;
    }

    VertexRemap {
        new_to_old,
        old_to_new,
    }
}

fn identity_vertex_remap(vertex_count: usize) -> VertexRemap {
    VertexRemap {
        new_to_old: (0..vertex_count).collect(),
        old_to_new: (0..vertex_count).map(Some).collect(),
    }
}

fn compact_primitive_joint_palette(
    primitive: &mut GltfPrimitiveData,
    skin_joint_count: usize,
) -> Result<Option<JointPaletteCompaction>, OptimizeError> {
    if primitive.joints_0.is_empty() || primitive.weights_0.is_empty() || skin_joint_count == 0 {
        return Ok(None);
    }
    let mut used = vec![false; skin_joint_count];
    for (joints, weights) in primitive.joints_0.iter().zip(&primitive.weights_0) {
        for (joint, weight) in joints.iter().copied().zip(*weights) {
            if weight > f32::EPSILON {
                used[joint as usize] = true;
            }
        }
    }
    let new_to_old = used
        .iter()
        .enumerate()
        .filter_map(|(old, used)| (*used).then_some(old))
        .collect::<Vec<_>>();
    let mut old_to_new = vec![None; skin_joint_count];
    for (new, old) in new_to_old.iter().copied().enumerate() {
        old_to_new[old] = Some(new);
    }
    for (joints, weights) in primitive.joints_0.iter_mut().zip(&primitive.weights_0) {
        for (joint, weight) in joints.iter_mut().zip(*weights) {
            if weight <= f32::EPSILON {
                if *joint as usize >= skin_joint_count {
                    *joint = 0;
                }
                continue;
            }
            if let Some(new) = old_to_new[*joint as usize] {
                *joint = u16::try_from(new).map_err(|_| OptimizeError::JointOutOfBounds {
                    joint: *joint,
                    joint_count: skin_joint_count,
                })?;
            }
        }
    }

    Ok(Some(JointPaletteCompaction {
        new_to_old,
        old_to_new,
    }))
}

fn remap_vec<T: Copy>(values: &[T], new_to_old: &[usize]) -> Vec<T> {
    new_to_old.iter().map(|old| values[*old]).collect()
}

fn remap_optional_vec<T: Copy>(values: &[T], new_to_old: &[usize]) -> Vec<T> {
    if values.is_empty() {
        Vec::new()
    } else {
        remap_vec(values, new_to_old)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    fn sample_primitive() -> GltfPrimitiveData {
        GltfPrimitiveData {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [9.0, 9.0, 9.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 5],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 5],
            tex_coords_0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [0.5, 0.5]],
            colors_0: vec![[1.0, 1.0, 1.0, 1.0]; 5],
            joints_0: vec![
                [2, 4, 0, 0],
                [2, 0, 0, 0],
                [4, 2, 0, 0],
                [4, 0, 0, 0],
                [9, 0, 0, 0],
            ],
            weights_0: vec![
                [0.25, 0.25, 0.0, 0.0],
                [2.0, 0.0, 0.0, 0.0],
                [0.1, 0.3, 0.0, 0.0],
                [0.5, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
            ],
            indices: vec![0, 1, 2, 2, 2, 3, 1, 3, 2],
            morph_targets: vec![
                GltfMorphTargetData {
                    positions: vec![[0.0, 0.0, 0.0]; 5],
                    normals: Vec::new(),
                    tangents: Vec::new(),
                },
                GltfMorphTargetData {
                    positions: vec![
                        [0.0, 0.0, 0.0],
                        [0.0, 0.0, 0.1],
                        [0.0, 0.0, 0.0],
                        [0.0, 0.0, 0.0],
                        [0.0, 0.0, 0.0],
                    ],
                    normals: Vec::new(),
                    tangents: Vec::new(),
                },
            ],
            ..GltfPrimitiveData::default()
        }
    }

    #[test]
    fn optimizer_removes_degenerate_triangles_unused_vertices_and_empty_morphs() {
        let mut primitive = sample_primitive();

        let report = optimize_primitive(&mut primitive, 5, OptimizeOptions::default()).unwrap();

        assert_eq!(report.removed_degenerate_triangles, 1);
        assert_eq!(report.removed_vertices, 1);
        assert_eq!(report.removed_morph_targets, 1);
        assert_eq!(report.vertex_remap.new_to_old, vec![0, 1, 2, 3]);
        assert_eq!(
            report.vertex_remap.old_to_new,
            vec![Some(0), Some(1), Some(2), Some(3), None]
        );
        assert_eq!(primitive.positions.len(), 4);
        assert_eq!(primitive.indices, vec![0, 1, 2, 1, 3, 2]);
        assert_eq!(primitive.morph_targets.len(), 1);
        assert_eq!(primitive.morph_targets[0].positions.len(), 4);
    }

    #[test]
    fn optimizer_normalizes_weights_and_compacts_joint_palette() {
        let mut primitive = sample_primitive();

        let report = optimize_primitive(&mut primitive, 5, OptimizeOptions::default()).unwrap();
        let compaction = report.joint_compaction.unwrap();

        assert_eq!(compaction.new_to_old, vec![2, 4]);
        assert_eq!(
            compaction.old_to_new,
            vec![None, None, Some(0), None, Some(1)]
        );
        assert_eq!(primitive.joints_0[0], [0, 1, 0, 0]);
        assert_eq!(primitive.weights_0[0], [0.5, 0.5, 0.0, 0.0]);
        assert_eq!(primitive.weights_0[1], [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn joint_compaction_can_be_applied_to_skin() {
        let mut skin = GltfSkinData {
            joints: vec![10, 11, 12, 13, 14],
            inverse_bind_matrices: vec![
                Mat4::from_translation(Vec3::X),
                Mat4::from_translation(Vec3::Y),
                Mat4::from_translation(Vec3::Z),
                Mat4::from_translation(Vec3::NEG_X),
                Mat4::from_translation(Vec3::NEG_Y),
            ],
        };
        let compaction = JointPaletteCompaction {
            new_to_old: vec![2, 4],
            old_to_new: vec![None, None, Some(0), None, Some(1)],
        };

        apply_joint_compaction_to_skin(&mut skin, &compaction);

        assert_eq!(skin.joints, vec![12, 14]);
        assert_eq!(
            skin.inverse_bind_matrices,
            vec![
                Mat4::from_translation(Vec3::Z),
                Mat4::from_translation(Vec3::NEG_Y)
            ]
        );
    }

    #[test]
    fn optimizer_keeps_unindexed_primitives_intact() {
        let mut primitive = sample_primitive();
        primitive.indices.clear();

        let report = optimize_primitive(&mut primitive, 5, OptimizeOptions::default()).unwrap();

        assert_eq!(report.removed_vertices, 0);
        assert_eq!(report.vertex_remap.new_to_old, vec![0, 1, 2, 3, 4]);
        assert_eq!(primitive.positions.len(), 5);
        assert!(primitive.indices.is_empty());
    }

    #[test]
    fn optimizer_rejects_inconsistent_attributes_and_bad_indices() {
        let mut primitive = sample_primitive();
        primitive.normals.pop();

        assert!(matches!(
            optimize_primitive(&mut primitive, 5, OptimizeOptions::default()),
            Err(OptimizeError::InconsistentAttribute {
                attribute: "NORMAL",
                expected: 5,
                actual: 4
            })
        ));

        let mut primitive = sample_primitive();
        primitive.indices.push(0);
        assert!(matches!(
            optimize_primitive(&mut primitive, 5, OptimizeOptions::default()),
            Err(OptimizeError::IndexCountNotTriangleList { count: 10 })
        ));

        let mut primitive = sample_primitive();
        primitive.indices[0] = 99;
        assert!(matches!(
            optimize_primitive(&mut primitive, 5, OptimizeOptions::default()),
            Err(OptimizeError::IndexOutOfBounds {
                index: 99,
                vertex_count: 5
            })
        ));
    }

    #[test]
    fn optimizer_rejects_joints_outside_skin_palette() {
        let mut primitive = sample_primitive();
        primitive.joints_0[0] = [8, 0, 0, 0];

        assert!(matches!(
            optimize_primitive(&mut primitive, 5, OptimizeOptions::default()),
            Err(OptimizeError::JointOutOfBounds {
                joint: 8,
                joint_count: 5
            })
        ));
    }
}
