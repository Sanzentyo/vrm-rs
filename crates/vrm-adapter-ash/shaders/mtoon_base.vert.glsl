#version 450

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec2 in_tex_coord_0;
layout(location = 2) in vec4 in_color_0;
layout(location = 3) in vec3 in_normal;
layout(location = 4) in vec4 in_tangent;

layout(location = 0) out vec2 out_tex_coord_0;
layout(location = 1) out vec4 out_color_0;
layout(location = 2) out vec3 out_normal;
layout(location = 3) out vec4 out_tangent;

void main() {
    out_tex_coord_0 = in_tex_coord_0;
    out_color_0 = in_color_0;
    out_normal = normalize(in_normal);
    out_tangent = vec4(normalize(in_tangent.xyz), in_tangent.w);
    gl_Position = vec4(in_position.xy, clamp(in_position.z, -1.0, 1.0), 1.0);
}
