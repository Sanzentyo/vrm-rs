#version 450

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec2 in_tex_coord_0;
layout(location = 2) in vec4 in_color_0;
layout(location = 3) in vec3 in_normal;
layout(location = 4) in vec4 in_tangent;

layout(set = 0, binding = 9, std140) uniform AshSceneUniform {
    mat4 view_projection;
    mat4 view;
    mat4 world_from_view;
    vec4 light_dir;
    vec4 light_color;
    vec4 camera_pos;
    vec4 mtoon_lighting;
} scene;

layout(location = 0) out vec2 out_tex_coord_0;
layout(location = 1) out vec4 out_color_0;
layout(location = 2) out vec3 out_normal;
layout(location = 3) out vec4 out_tangent;
layout(location = 4) out vec3 out_world_position;

void main() {
    out_tex_coord_0 = in_tex_coord_0;
    out_color_0 = in_color_0;
    out_normal = normalize(in_normal);
    out_tangent = vec4(normalize(in_tangent.xyz), in_tangent.w);
    out_world_position = in_position;
    gl_Position = scene.view_projection * vec4(in_position, 1.0);
}
