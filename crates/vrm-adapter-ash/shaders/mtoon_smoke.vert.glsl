#version 450

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec2 in_tex_coord_0;
layout(location = 2) in vec4 in_color_0;

layout(location = 0) out vec2 out_tex_coord_0;
layout(location = 1) out vec4 out_color_0;

void main() {
    out_tex_coord_0 = in_tex_coord_0;
    out_color_0 = in_color_0;
    gl_Position = vec4(in_position.xy, clamp(in_position.z, -1.0, 1.0), 1.0);
}
