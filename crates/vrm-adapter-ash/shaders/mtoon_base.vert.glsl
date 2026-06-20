#version 450

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec2 in_tex_coord_0;
layout(location = 2) in vec4 in_color_0;
layout(location = 3) in vec3 in_normal;
layout(location = 4) in vec4 in_tangent;
layout(location = 5) in float in_normal_scale;
layout(location = 6) in float in_double_sided;

layout(set = 0, binding = 9, std140) uniform AshSceneUniform {
    mat4 view_projection;
    mat4 view;
    mat4 world_from_view;
    vec4 light_dir;
    vec4 light_color;
    vec4 camera_pos;
    vec4 mtoon_lighting;
} scene;

layout(set = 0, binding = 0, std140) uniform MtoonGpuUniform {
    vec4 base_color_factor;
    vec4 shade_color_factor_cutoff;
    vec4 emissive_color_outline_width;
    vec4 shading;
    vec4 lighting;
    vec4 matcap_factor_debug;
    vec4 rim_color_lighting_mix;
    vec4 rim_params;
    vec4 outline_color_lighting_mix;
    vec4 uv_animation;
    uvec4 flags;
} mtoon;

layout(location = 0) out vec2 out_tex_coord_0;
layout(location = 1) out vec4 out_color_0;
layout(location = 2) out vec3 out_normal;
layout(location = 3) out vec4 out_tangent;
layout(location = 4) out vec3 out_world_position;
layout(location = 5) out float out_normal_scale;
layout(location = 6) out float out_double_sided;

void main() {
    out_tex_coord_0 = in_tex_coord_0;
    out_color_0 = in_color_0;
    out_normal = normalize(in_normal);
    out_tangent = vec4(normalize(in_tangent.xyz), in_tangent.w);
    out_world_position = in_position;
    out_normal_scale = in_normal_scale;
    out_double_sided = in_double_sided;
    gl_PointSize = 1.0;
    gl_Position = scene.view_projection * vec4(in_position, 1.0);
    if (mtoon.flags.z == 1u) {
        gl_Position.z += 0.000001 * gl_Position.w;
    }
}
