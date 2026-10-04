attribute vec2 a_position;
attribute vec2 a_tex_coord;
attribute vec2 a_effect_coord;

varying vec2 v_tex_coord;
varying vec2 v_effect_coord;
uniform vec2 u_layout_scale;
uniform vec2 u_parallax_offset;

void main() {
    v_tex_coord = a_tex_coord;
    v_effect_coord = a_effect_coord;
    gl_Position = vec4((a_position + u_parallax_offset) * u_layout_scale, 0.0, 1.0);
}
